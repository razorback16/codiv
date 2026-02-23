#include "bash_coprocess.h"

#include <cerrno>
#include <cstdlib>
#include <cstring>
#include <random>
#include <sstream>
#include <stdexcept>

#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <sys/wait.h>
#include <termios.h>
#include <unistd.h>
#include <util.h>  // forkpty on macOS

namespace slate {

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

std::string BashCoprocess::generate_sentinel() {
    std::random_device rd;
    std::mt19937 gen(rd());
    std::uniform_int_distribution<uint32_t> dist(0, 0xFFFFFFFF);

    char buf[48];
    std::snprintf(buf, sizeof(buf), "__SLATE_SENTINEL_%08x%08x_",
                  dist(gen), dist(gen));
    return std::string(buf);
}

/// Find the expanded sentinel in accumulated output.
/// The expanded form is: sentinel + digits + "__"
/// The echoed (unexpanded) form is: sentinel + "${__SLATE_EXIT}__"
/// We distinguish them by checking if sentinel is followed by a digit.
static std::string::size_type find_expanded_sentinel(
    const std::string& text, const std::string& sentinel) {
    std::string::size_type search_from = 0;
    while (true) {
        auto pos = text.find(sentinel, search_from);
        if (pos == std::string::npos) return std::string::npos;

        // Check what follows the sentinel
        auto after = pos + sentinel.size();
        if (after < text.size()) {
            char ch = text[after];
            // Expanded form starts with a digit (exit code)
            if (ch >= '0' && ch <= '9') {
                return pos;
            }
        }
        // This was the unexpanded echo — skip past it
        search_from = pos + sentinel.size();
    }
}

bool BashCoprocess::write_all(const std::string& data) {
    const char* ptr = data.data();
    size_t remaining = data.size();
    while (remaining > 0) {
        ssize_t n = ::write(master_fd_, ptr, remaining);
        if (n < 0) {
            if (errno == EINTR) continue;
            return false;
        }
        ptr += n;
        remaining -= static_cast<size_t>(n);
    }
    return true;
}

std::string BashCoprocess::read_until_sentinel(const std::string& sentinel,
                                                int timeout_ms) {
    std::string accumulated;
    char buf[4096];

    // Deadline-based polling
    auto deadline = std::chrono::steady_clock::now() +
                    std::chrono::milliseconds(timeout_ms);

    while (true) {
        auto now = std::chrono::steady_clock::now();
        int remaining_ms = static_cast<int>(
            std::chrono::duration_cast<std::chrono::milliseconds>(
                deadline - now).count());
        if (remaining_ms <= 0) break;

        struct pollfd pfd{};
        pfd.fd = master_fd_;
        pfd.events = POLLIN;

        int ret = ::poll(&pfd, 1, std::min(remaining_ms, 500));
        if (ret < 0) {
            if (errno == EINTR) continue;
            break;
        }
        if (ret == 0) {
            // Timeout on this poll — check if sentinel already in buffer
            if (find_expanded_sentinel(accumulated, sentinel) != std::string::npos)
                break;
            continue;
        }

        if (pfd.revents & (POLLERR | POLLHUP)) {
            // Read any remaining data
            ssize_t n = ::read(master_fd_, buf, sizeof(buf));
            if (n > 0) accumulated.append(buf, static_cast<size_t>(n));
            break;
        }

        if (pfd.revents & POLLIN) {
            ssize_t n = ::read(master_fd_, buf, sizeof(buf));
            if (n <= 0) break;
            accumulated.append(buf, static_cast<size_t>(n));

            if (find_expanded_sentinel(accumulated, sentinel) != std::string::npos)
                break;
        }
    }

    return accumulated;
}

std::string BashCoprocess::clean_output(const std::string& raw,
                                         const std::string& command,
                                         const std::string& sentinel) {
    // Split into lines
    std::vector<std::string> lines;
    std::istringstream iss(raw);
    std::string line;
    while (std::getline(iss, line)) {
        lines.push_back(line);
    }

    std::vector<std::string> cleaned;
    for (auto& l : lines) {
        // Strip carriage returns
        while (!l.empty() && l.back() == '\r') l.pop_back();

        // Skip lines that contain the sentinel marker
        if (l.find("__SLATE_SENTINEL_") != std::string::npos) continue;

        // Skip lines that contain __SLATE_EXIT (the echoed command tail)
        if (l.find("__SLATE_EXIT") != std::string::npos) continue;

        // Skip common bash prompt patterns
        bool is_prompt_only = false;
        {
            // Remove ANSI escape sequences for comparison
            std::string clean_line;
            bool in_escape = false;
            for (char c : l) {
                if (c == '\033') { in_escape = true; continue; }
                if (in_escape) {
                    if ((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z'))
                        in_escape = false;
                    continue;
                }
                clean_line += c;
            }
            // Trim whitespace
            while (!clean_line.empty() && (clean_line.front() == ' ' || clean_line.front() == '\t'))
                clean_line.erase(clean_line.begin());
            while (!clean_line.empty() && (clean_line.back() == ' ' || clean_line.back() == '\t'))
                clean_line.pop_back();

            if (clean_line.empty()) {
                is_prompt_only = true;
            } else if (clean_line == "$" || clean_line == "#") {
                is_prompt_only = true;
            } else if (clean_line.size() <= 4 && clean_line.back() == '$') {
                is_prompt_only = true;
            }
        }

        // Skip echoed command line. Bash -i echoes the input.
        if (!command.empty() && l.find(command) != std::string::npos) continue;

        if (is_prompt_only) continue;

        cleaned.push_back(l);
    }

    // Rejoin
    std::string result;
    for (size_t i = 0; i < cleaned.size(); ++i) {
        if (i > 0) result += '\n';
        result += cleaned[i];
    }
    return result;
}

// ---------------------------------------------------------------------------
// Construction / Destruction
// ---------------------------------------------------------------------------

BashCoprocess::BashCoprocess() {
    struct winsize ws{};
    ws.ws_row = 24;
    ws.ws_col = 80;

    child_pid_ = forkpty(&master_fd_, nullptr, nullptr, &ws);
    if (child_pid_ < 0) {
        throw std::runtime_error("forkpty() failed: " +
                                 std::string(std::strerror(errno)));
    }

    if (child_pid_ == 0) {
        // Child process — exec bash
        // Set a minimal PS1 to reduce prompt noise
        ::setenv("PS1", "$ ", 1);
        ::unsetenv("PROMPT_COMMAND");
        ::setenv("HISTFILE", "/dev/null", 1);

        const char* args[] = {"bash", "--noediting", "--norc", "--noprofile", "-i", nullptr};
        execvp("bash", const_cast<char**>(args));
        _exit(127);  // exec failed
    }

    // Parent
    initialized_ = true;

    // Drain initial prompt/startup output
    struct pollfd pfd{};
    pfd.fd = master_fd_;
    pfd.events = POLLIN;

    for (int i = 0; i < 20; ++i) {
        int ret = ::poll(&pfd, 1, 200);
        if (ret > 0 && (pfd.revents & POLLIN)) {
            char buf[4096];
            ::read(master_fd_, buf, sizeof(buf));
        } else {
            break;
        }
    }
}

BashCoprocess::~BashCoprocess() {
    if (child_pid_ > 0) {
        ::kill(child_pid_, SIGTERM);
        int status;
        int ret = ::waitpid(child_pid_, &status, WNOHANG);
        if (ret == 0) {
            usleep(50000);
            ::kill(child_pid_, SIGKILL);
            ::waitpid(child_pid_, &status, 0);
        }
    }
    if (master_fd_ >= 0) {
        ::close(master_fd_);
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

CommandResult BashCoprocess::execute(const std::string& command,
                                      int timeout_ms) {
    if (!initialized_ || master_fd_ < 0) {
        return CommandResult{"", -1};
    }

    std::string sentinel = generate_sentinel();

    // Build the command: run command, capture exit code, echo sentinel+exitcode
    std::string full_cmd = command +
        "; __SLATE_EXIT=$?; echo \"" + sentinel + "${__SLATE_EXIT}__\"\n";

    if (!write_all(full_cmd)) {
        return CommandResult{"", -1};
    }

    // Read until we see the expanded sentinel (sentinel + digit)
    std::string raw = read_until_sentinel(sentinel, timeout_ms);

    // Extract exit code from the expanded sentinel line
    int exit_code = -1;
    auto pos = find_expanded_sentinel(raw, sentinel);
    if (pos != std::string::npos) {
        auto code_start = pos + sentinel.size();
        auto code_end = raw.find("__", code_start);
        if (code_end != std::string::npos) {
            std::string code_str = raw.substr(code_start, code_end - code_start);
            try {
                exit_code = std::stoi(code_str);
            } catch (...) {
                exit_code = -1;
            }
        }
    }

    // Clean the output
    std::string output = clean_output(raw, command, sentinel);

    return CommandResult{output, exit_code};
}

void BashCoprocess::send_signal(int sig) {
    if (child_pid_ > 0) {
        ::kill(child_pid_, sig);
    }
}

std::string BashCoprocess::capture_cwd() {
    auto result = execute("pwd");
    auto& s = result.output;
    while (!s.empty() && (s.back() == '\n' || s.back() == '\r' || s.back() == ' '))
        s.pop_back();
    while (!s.empty() && (s.front() == '\n' || s.front() == '\r' || s.front() == ' '))
        s.erase(s.begin());
    return s;
}

std::vector<std::pair<std::string, std::string>> BashCoprocess::capture_env() {
    auto result = execute("env");
    std::vector<std::pair<std::string, std::string>> env;

    std::istringstream iss(result.output);
    std::string line;
    while (std::getline(iss, line)) {
        while (!line.empty() && line.back() == '\r') line.pop_back();
        auto eq = line.find('=');
        if (eq != std::string::npos && eq > 0) {
            env.emplace_back(line.substr(0, eq), line.substr(eq + 1));
        }
    }
    return env;
}

bool BashCoprocess::is_alive() const {
    if (child_pid_ <= 0) return false;
    int status;
    pid_t ret = ::waitpid(child_pid_, &status, WNOHANG);
    return (ret == 0);
}

} // namespace slate
