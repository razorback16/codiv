#pragma once
#include "ui/terminal_ui.h"
#include "shell/bash_coprocess.h"
#include "shell/command_index.h"
#include "shell/interactive.h"
#include "ipc/daemon_client.h"
#include <memory>
#include <atomic>
#include <csignal>

namespace slate {

class SlateApp : public UIDelegate {
public:
    SlateApp();
    ~SlateApp();

    /// Initialize all subsystems
    void init();

    /// Run the UI main loop
    int run();

    /// Request graceful shutdown
    void request_shutdown();

    /// Forward signal to bash child (for SIGINT handling)
    void forward_signal(int sig);

    // UIDelegate interface
    void on_command_submitted(const std::string& input) override;
    std::vector<std::string> on_autocomplete(const std::string& prefix) override;

private:
    void update_status();
    std::string get_git_branch();
    std::string extract_first_word(const std::string& input);

    std::unique_ptr<BashCoprocess> bash_;
    CommandIndex command_index_;
    InteractiveSession interactive_;
    std::unique_ptr<DaemonClient> daemon_;
    std::unique_ptr<TerminalUI> ui_;

    std::atomic<bool> shutdown_requested_{false};
};

} // namespace slate
