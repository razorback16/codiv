#include "app.h"
#include <cstdio>
#include <iostream>
#include <sstream>
#include <algorithm>
#include <unistd.h>

namespace slate {

SlateApp::SlateApp() = default;

SlateApp::~SlateApp() {
    if (daemon_) {
        daemon_->disconnect();
    }
}

void SlateApp::init() {
    // 1. Spawn bash coprocess
    bash_ = std::make_unique<BashCoprocess>();

    // 2. Build command index from bash
    command_index_.build(bash_.get());

    // 3. Connect daemon (non-fatal)
    daemon_ = std::make_unique<DaemonClient>();
    try {
        daemon_->ensure_daemon_running();
        daemon_->connect();
    } catch (...) {
        // Non-fatal: daemon may not be available
    }

    // 4. Create UI with this as delegate
    ui_ = std::make_unique<TerminalUI>(this);

    // Set initial status
    update_status();
}

int SlateApp::run() {
    ui_->run();  // blocks until UI exits
    return 0;
}

void SlateApp::request_shutdown() {
    shutdown_requested_.store(true);
    if (ui_) {
        ui_->stop();
    }
    if (bash_ && bash_->is_alive()) {
        bash_->send_signal(SIGTERM);
    }
    if (daemon_) {
        daemon_->disconnect();
    }
}

void SlateApp::forward_signal(int sig) {
    if (bash_ && bash_->is_alive()) {
        bash_->send_signal(sig);
    }
}

void SlateApp::on_command_submitted(const std::string& input) {
    std::string first_word = extract_first_word(input);

    // Empty input — do nothing
    if (first_word.empty()) {
        return;
    }

    // exit / quit
    if (first_word == "exit" || first_word == "quit") {
        request_shutdown();
        return;
    }

    // Interactive passthrough (vim, htop, etc.)
    if (InteractiveSession::needs_passthrough(input)) {
        // First execute the command in bash so the interactive program launches
        // Then pass through to interactive session
        ui_->suspend();
        // Write the command to bash and enter interactive mode
        // The interactive session handles raw I/O with the pty
        std::string cmd = input + "\n";
        write(bash_->master_fd(), cmd.c_str(), cmd.size());
        interactive_.enter(bash_->master_fd());
        ui_->resume();
        update_status();
        return;
    }

    // Known command — execute via bash coprocess
    if (command_index_.is_known_command(first_word)) {
        auto result = bash_->execute(input);
        if (!result.output.empty()) {
            ui_->append_output(result.output);
        }
        if (result.exit_code != 0) {
            ui_->append_output("[exit code: " + std::to_string(result.exit_code) + "]\n");
        }
        update_status();
        return;
    }

    // AI mode placeholder
    if (first_word[0] == '?') {
        ui_->append_output("AI mode not yet available (Phase 2)\n");
        return;
    }

    // Unknown command
    ui_->append_output("command not found: " + first_word + "\n");
}

std::vector<std::string> SlateApp::on_autocomplete(const std::string& prefix) {
    return command_index_.complete(prefix);
}

void SlateApp::update_status() {
    std::string cwd;
    if (bash_ && bash_->is_alive()) {
        cwd = bash_->capture_cwd();
    }

    std::string branch = get_git_branch();

    bool connected = daemon_ && daemon_->is_connected();

    if (ui_) {
        ui_->set_status(cwd, branch, connected);
    }
}

std::string SlateApp::get_git_branch() {
    std::string cwd;
    if (bash_ && bash_->is_alive()) {
        cwd = bash_->capture_cwd();
    }
    if (cwd.empty()) {
        return {};
    }

    std::string cmd = "git -C " + cwd + " branch --show-current 2>/dev/null";
    FILE* pipe = popen(cmd.c_str(), "r");
    if (!pipe) {
        return {};
    }

    std::string result;
    char buffer[128];
    while (fgets(buffer, sizeof(buffer), pipe)) {
        result += buffer;
    }
    pclose(pipe);

    // Trim trailing newline
    while (!result.empty() && (result.back() == '\n' || result.back() == '\r')) {
        result.pop_back();
    }
    return result;
}

std::string SlateApp::extract_first_word(const std::string& input) {
    auto start = input.find_first_not_of(" \t");
    if (start == std::string::npos) {
        return {};
    }
    auto end = input.find_first_of(" \t", start);
    if (end == std::string::npos) {
        return input.substr(start);
    }
    return input.substr(start, end - start);
}

} // namespace slate
