#include "interactive.h"

#include <poll.h>
#include <unistd.h>
#include <sstream>

namespace slate {

const std::unordered_set<std::string> InteractiveSession::interactive_commands_ = {
    "vim", "vi", "nvim", "nano", "emacs",
    "htop", "top",
    "less", "more", "man",
    "ssh", "tmux", "screen",
    "python", "python3", "node", "irb", "ghci"
};

InteractiveSession::InteractiveSession() = default;

InteractiveSession::~InteractiveSession() {
    if (in_session_) {
        exit();
    }
}

bool InteractiveSession::needs_passthrough(const std::string& command) {
    // Extract first word
    std::istringstream iss(command);
    std::string first_word;
    iss >> first_word;

    if (first_word.empty()) return false;

    // Strip any leading path (e.g., /usr/bin/vim -> vim)
    auto slash = first_word.rfind('/');
    if (slash != std::string::npos) {
        first_word = first_word.substr(slash + 1);
    }

    return interactive_commands_.count(first_word) > 0;
}

void InteractiveSession::enter(int master_fd) {
    // Save current terminal state
    if (::tcgetattr(STDIN_FILENO, &saved_termios_) == 0) {
        terminal_saved_ = true;
    }

    // Set raw mode on stdin
    struct termios raw = saved_termios_;
    ::cfmakeraw(&raw);
    ::tcsetattr(STDIN_FILENO, TCSANOW, &raw);

    in_session_ = true;

    // Bidirectional proxy loop: stdin <-> master_fd
    struct pollfd fds[2];
    fds[0].fd = STDIN_FILENO;
    fds[0].events = POLLIN;
    fds[1].fd = master_fd;
    fds[1].events = POLLIN;

    char buf[4096];

    while (in_session_) {
        int ret = ::poll(fds, 2, 100);
        if (ret < 0) {
            if (errno == EINTR) continue;
            break;
        }

        // Data from stdin -> forward to master_fd (user typing)
        if (fds[0].revents & POLLIN) {
            ssize_t n = ::read(STDIN_FILENO, buf, sizeof(buf));
            if (n <= 0) break;
            ssize_t written = 0;
            while (written < n) {
                ssize_t w = ::write(master_fd, buf + written, n - written);
                if (w < 0) {
                    if (errno == EINTR) continue;
                    goto done;
                }
                written += w;
            }
        }

        // Data from master_fd -> forward to stdout (program output)
        if (fds[1].revents & POLLIN) {
            ssize_t n = ::read(master_fd, buf, sizeof(buf));
            if (n <= 0) break;  // Child exited
            ssize_t written = 0;
            while (written < n) {
                ssize_t w = ::write(STDOUT_FILENO, buf + written, n - written);
                if (w < 0) {
                    if (errno == EINTR) continue;
                    goto done;
                }
                written += w;
            }
        }

        // Check for hangup on master fd (child exited)
        if (fds[1].revents & (POLLHUP | POLLERR)) {
            // Drain remaining output
            while (true) {
                ssize_t n = ::read(master_fd, buf, sizeof(buf));
                if (n <= 0) break;
                ::write(STDOUT_FILENO, buf, n);
            }
            break;
        }
    }

done:
    exit();
}

void InteractiveSession::exit() {
    if (terminal_saved_) {
        ::tcsetattr(STDIN_FILENO, TCSANOW, &saved_termios_);
        terminal_saved_ = false;
    }
    in_session_ = false;
}

} // namespace slate
