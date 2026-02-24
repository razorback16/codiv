#include "daemon.h"
#include "types.h"

#include <iostream>
#include <cstring>
#include <csignal>
#include <unistd.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <filesystem>

static slate::SlatedDaemon* g_daemon = nullptr;

static void signal_handler(int /*sig*/) {
    if (g_daemon) g_daemon->request_shutdown();
}

static void daemonize() {
    pid_t pid = fork();
    if (pid < 0) {
        std::cerr << "slated: fork() failed" << std::endl;
        _exit(1);
    }
    if (pid > 0) {
        // Parent exits.
        _exit(0);
    }
    // Child continues as daemon.
    setsid();

    // Redirect stdin/stdout to /dev/null.
    int devnull = open("/dev/null", O_RDWR);
    if (devnull >= 0) {
        dup2(devnull, STDIN_FILENO);
        dup2(devnull, STDOUT_FILENO);
        if (devnull > STDERR_FILENO) close(devnull);
    }

    // Redirect stderr to log file for daemon debugging.
    auto log_path = slate::log_file_path();
    std::filesystem::create_directories(std::filesystem::path(log_path).parent_path());
    int log_fd = open(log_path.c_str(), O_WRONLY | O_CREAT | O_APPEND, 0640);
    if (log_fd >= 0) {
        dup2(log_fd, STDERR_FILENO);
        if (log_fd > STDERR_FILENO) close(log_fd);
    }
}

int main(int argc, char* argv[]) {
    bool foreground = false;

    for (int i = 1; i < argc; ++i) {
        if (std::strcmp(argv[i], "--version") == 0 || std::strcmp(argv[i], "-V") == 0) {
            std::cout << "slated " << slate::kVersion << std::endl;
            return 0;
        }
        if (std::strcmp(argv[i], "--foreground") == 0 || std::strcmp(argv[i], "-f") == 0) {
            foreground = true;
        }
    }

    if (!foreground) {
        daemonize();
    }

    // Install signal handlers.
    struct sigaction sa{};
    sa.sa_handler = signal_handler;
    sigemptyset(&sa.sa_mask);
    sa.sa_flags = 0;
    sigaction(SIGTERM, &sa, nullptr);
    sigaction(SIGINT,  &sa, nullptr);

    // Ignore SIGPIPE (broken pipe on client disconnect).
    signal(SIGPIPE, SIG_IGN);

    slate::SlatedDaemon daemon;
    g_daemon = &daemon;

    try {
        daemon.start();
        daemon.run();
        daemon.shutdown();
    } catch (const std::exception& e) {
        std::cerr << "slated: " << e.what() << std::endl;
        daemon.shutdown();
        g_daemon = nullptr;
        return 1;
    }

    g_daemon = nullptr;
    return 0;
}
