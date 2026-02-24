#pragma once

#include <string>
#include <vector>
#include <utility>
#include <sys/types.h>

namespace slate {

struct WorkerProcess {
    pid_t pid = -1;
    int stdout_fd = -1;    // read end of stdout pipe
    int stderr_fd = -1;    // read end of stderr pipe
    int client_fd = -1;    // owning client session fd
    std::string request_id;
    bool stdout_eof = false;
    bool stderr_eof = false;

    bool is_done() const { return stdout_eof && stderr_eof; }
};

/// Spawn a bash worker that executes `command` in the given working directory
/// with the specified environment. Returns a WorkerProcess with pipe fds for
/// reading stdout/stderr.
WorkerProcess spawn_worker(
    const std::string& command,
    const std::string& cwd,
    const std::vector<std::pair<std::string, std::string>>& env_vars,
    const std::string& path,
    int client_fd,
    const std::string& request_id);

}  // namespace slate
