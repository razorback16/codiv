#pragma once

#include <string>
#include <map>
#include <list>
#include <memory>
#include <atomic>
#include <vector>
#include <chrono>

#include "session.h"
#include "worker_process.h"
#include "ipc/socket_server.h"
#include "ipc_generated.h"

namespace slate {

class SlatedDaemon {
public:
    SlatedDaemon();
    ~SlatedDaemon();

    void start();
    void run();
    void shutdown();

    void request_shutdown() {
        shutdown_requested_.store(true, std::memory_order_relaxed);
    }

    bool shutdown_requested() const {
        return shutdown_requested_.load(std::memory_order_relaxed);
    }

private:
    void handle_connect(int fd);
    void handle_message(int fd, std::vector<uint8_t> data);
    void handle_disconnect(int fd);

    void dispatch_message(int fd, const slate::ipc::Message* msg);

    // Worker management
    void start_command(int fd, const slate::ipc::ExecuteCommandMsg* exec);
    void poll_workers();
    void reap_finished_workers();
    void cleanup_stale_sessions();
    void kill_workers_for_client(int fd);

    // Message senders
    void send_heartbeat_response(int fd);
    void send_command_output(int fd, const std::string& request_id,
                             const uint8_t* data, size_t len, bool is_stderr);
    void send_command_complete(int fd, const std::string& request_id, int exit_code);
    void send_error(int fd, const std::string& request_id, const std::string& message);

    static void ensure_directory(const std::string& path);
    static bool is_process_alive(pid_t pid);

    std::unique_ptr<SocketServer>    server_;
    std::map<int, ClientSession>     sessions_;
    std::list<WorkerProcess>         workers_;
    std::atomic<bool>                shutdown_requested_{false};
    std::string                      pid_file_path_;
    std::chrono::steady_clock::time_point last_cleanup_;
};

}  // namespace slate
