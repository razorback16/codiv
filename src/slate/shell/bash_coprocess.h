#pragma once

#include <string>
#include <utility>
#include <vector>
#include <cstdint>

namespace slate {

struct CommandResult {
    std::string output;
    int exit_code = -1;
};

class BashCoprocess {
public:
    /// Spawn a bash coprocess via forkpty.
    BashCoprocess();
    ~BashCoprocess();

    // Non-copyable, non-movable (owns child process + fd)
    BashCoprocess(const BashCoprocess&) = delete;
    BashCoprocess& operator=(const BashCoprocess&) = delete;

    /// Execute a command and return its output and exit code.
    /// Uses a unique sentinel to delimit output boundaries.
    /// @param command  Shell command string
    /// @param timeout_ms  Poll timeout in milliseconds (default 30s)
    CommandResult execute(const std::string& command, int timeout_ms = 30000);

    /// Send a signal to the child bash process.
    void send_signal(int sig);

    /// Capture the current working directory of the shell.
    std::string capture_cwd();

    /// Capture the full environment as KEY=VALUE pairs.
    std::vector<std::pair<std::string, std::string>> capture_env();

    /// Check if the child bash process is still running.
    bool is_alive() const;

    /// Get the master pty file descriptor (for interactive passthrough).
    int master_fd() const { return master_fd_; }

    /// Get the child PID.
    pid_t child_pid() const { return child_pid_; }

private:
    /// Generate a random hex sentinel string.
    static std::string generate_sentinel();

    /// Write all bytes to master fd.
    bool write_all(const std::string& data);

    /// Read from master fd using poll(), accumulating until sentinel found.
    /// Returns the raw accumulated output.
    std::string read_until_sentinel(const std::string& sentinel, int timeout_ms);

    /// Strip bash echoed input and prompt lines from raw output.
    static std::string clean_output(const std::string& raw,
                                    const std::string& command,
                                    const std::string& sentinel);

    int master_fd_ = -1;
    pid_t child_pid_ = -1;
    bool initialized_ = false;
};

} // namespace slate
