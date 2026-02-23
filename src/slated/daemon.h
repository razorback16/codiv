#pragma once
#include <string>
#include <map>
#include <memory>
#include <atomic>
#include "session.h"
#include "ipc/socket_server.h"
#include "ipc_generated.h"

namespace slate {

class SlatedDaemon {
public:
    SlatedDaemon();
    ~SlatedDaemon();

    SlatedDaemon(const SlatedDaemon&) = delete;
    SlatedDaemon& operator=(const SlatedDaemon&) = delete;

    // Start the daemon: check PID, create dirs, write PID file, start socket server
    void start();

    // Run the main event loop
    void run();

    // Graceful shutdown: stop server, remove PID file
    void shutdown();

    // Request shutdown from signal handler (async-signal-safe)
    void request_shutdown() { shutdown_requested_.store(true, std::memory_order_relaxed); }

    bool shutdown_requested() const { return shutdown_requested_.load(std::memory_order_relaxed); }

private:
    void handle_connect(int fd);
    void handle_message(int fd, std::vector<uint8_t> data);
    void handle_disconnect(int fd);

    void dispatch_message(int fd, const slate::ipc::Message* msg);
    void send_heartbeat_response(int fd);
    void send_ack(int fd, const char* request_id);

    static void ensure_directory(const std::string& path);
    static bool is_process_alive(pid_t pid);

    std::unique_ptr<SocketServer> server_;
    std::map<int, ClientSession> sessions_;
    std::atomic<bool> shutdown_requested_{false};
    std::string pid_file_path_;
};

} // namespace slate
