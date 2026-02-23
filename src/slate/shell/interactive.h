#pragma once

#include <string>
#include <unordered_set>
#include <termios.h>

namespace slate {

class InteractiveSession {
public:
    InteractiveSession();
    ~InteractiveSession();

    /// Check if a command's first word is a known interactive program.
    static bool needs_passthrough(const std::string& command);

    /// Enter interactive passthrough mode: save terminal state, set raw mode,
    /// and proxy stdin <-> master_fd bidirectionally.
    /// Blocks until the interactive program exits (master_fd read returns 0/error).
    void enter(int master_fd);

    /// Restore original terminal state. Called automatically when enter() returns,
    /// but can also be called explicitly.
    void exit();

private:
    static const std::unordered_set<std::string> interactive_commands_;

    struct termios saved_termios_{};
    bool terminal_saved_ = false;
    bool in_session_ = false;
};

} // namespace slate
