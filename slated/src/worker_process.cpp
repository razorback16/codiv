#include "worker_process.h"

#include <unistd.h>
#include <fcntl.h>
#include <cstdlib>
#include <cstring>
#include <cerrno>
#include <iostream>

extern char** environ;

namespace slate {

WorkerProcess spawn_worker(
    const std::string& command,
    const std::string& cwd,
    const std::vector<std::pair<std::string, std::string>>& env_vars,
    const std::string& path,
    int client_fd,
    const std::string& request_id)
{
    int stdout_pipe[2];
    int stderr_pipe[2];

    if (pipe(stdout_pipe) < 0 || pipe(stderr_pipe) < 0) {
        std::cerr << "slated: pipe() failed: " << std::strerror(errno) << std::endl;
        return {};
    }

    pid_t pid = fork();
    if (pid < 0) {
        std::cerr << "slated: fork() failed: " << std::strerror(errno) << std::endl;
        close(stdout_pipe[0]); close(stdout_pipe[1]);
        close(stderr_pipe[0]); close(stderr_pipe[1]);
        return {};
    }

    if (pid == 0) {
        // ---- Child process ----

        // Close read ends of pipes.
        close(stdout_pipe[0]);
        close(stderr_pipe[0]);

        // Redirect stdout/stderr to pipe write ends.
        dup2(stdout_pipe[1], STDOUT_FILENO);
        dup2(stderr_pipe[1], STDERR_FILENO);
        close(stdout_pipe[1]);
        close(stderr_pipe[1]);

        // Redirect stdin from /dev/null.
        int devnull = open("/dev/null", O_RDONLY);
        if (devnull >= 0) {
            dup2(devnull, STDIN_FILENO);
            close(devnull);
        }

        // Set up environment from session snapshot.
        clearenv();
        for (const auto& [key, value] : env_vars) {
            setenv(key.c_str(), value.c_str(), 1);
        }
        if (!path.empty()) {
            setenv("PATH", path.c_str(), 1);
        }

        // Change working directory.
        if (!cwd.empty()) {
            if (chdir(cwd.c_str()) != 0) {
                // If chdir fails, continue in current dir — the command
                // may still succeed.
            }
        }

        // Execute the command via bash.
        execl("/bin/bash", "bash", "-c", command.c_str(), nullptr);

        // If execl returns, it failed.
        _exit(127);
    }

    // ---- Parent process ----

    // Close write ends of pipes.
    close(stdout_pipe[1]);
    close(stderr_pipe[1]);

    // Set read ends non-blocking.
    fcntl(stdout_pipe[0], F_SETFL, fcntl(stdout_pipe[0], F_GETFL) | O_NONBLOCK);
    fcntl(stderr_pipe[0], F_SETFL, fcntl(stderr_pipe[0], F_GETFL) | O_NONBLOCK);

    WorkerProcess worker;
    worker.pid = pid;
    worker.stdout_fd = stdout_pipe[0];
    worker.stderr_fd = stderr_pipe[0];
    worker.client_fd = client_fd;
    worker.request_id = request_id;
    return worker;
}

}  // namespace slate
