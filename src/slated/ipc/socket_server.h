#pragma once
#include <string>
#include <vector>
#include <set>
#include <functional>
#include <cstdint>
#include "platform.h"
#include "ipc_protocol.h"

namespace slate {

class SocketServer {
public:
    explicit SocketServer(const std::string& socket_path);
    ~SocketServer();

    SocketServer(const SocketServer&) = delete;
    SocketServer& operator=(const SocketServer&) = delete;

    // Start listening on the Unix domain socket
    void start();

    // Run one iteration of the event loop. Returns false if not running.
    bool poll(int timeout_ms = 100);

    // Run the event loop continuously until stop() is called
    void run();

    // Send a framed message to a specific client fd
    bool send(int fd, const std::vector<uint8_t>& data);
    bool send(int fd, const uint8_t* data, size_t size);

    // Stop the server, close all connections, unlink socket
    void stop();

    bool is_running() const { return running_; }

    // Callbacks
    std::function<void(int fd)> on_connect;
    std::function<void(int fd, std::vector<uint8_t> data)> on_message;
    std::function<void(int fd)> on_disconnect;

private:
    void handle_new_connection();
    void handle_client_data(int fd);
    void disconnect_client(int fd);

    std::string socket_path_;
    int listen_fd_ = -1;
    EventLoop event_loop_;
    std::set<int> client_fds_;
    bool running_ = false;
};

} // namespace slate
