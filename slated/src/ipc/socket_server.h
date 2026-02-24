#pragma once

#include <string>
#include <vector>
#include <set>
#include <functional>
#include <cstdint>

#include "event_loop.h"
#include "../ipc_protocol.h"

namespace slate {

class SocketServer {
public:
    explicit SocketServer(const std::string& socket_path);
    ~SocketServer();

    // Non-copyable.
    SocketServer(const SocketServer&) = delete;
    SocketServer& operator=(const SocketServer&) = delete;

    /// Bind, listen, and register the listen fd with the event loop.
    void start();

    /// Run one iteration of the event loop.  Returns false if the server
    /// has been stopped or an unrecoverable error occurred.
    bool poll(int timeout_ms = 100);

    /// Blocking run loop — calls poll() until the server is stopped.
    void run();

    /// Send a framed message to a connected client.
    bool send(int fd, const std::vector<uint8_t>& data);
    bool send(int fd, const uint8_t* data, size_t size);

    /// Stop listening and close all client connections.
    void stop();

    bool is_running() const { return running_; }

    // ---- Callbacks ----
    std::function<void(int fd)>                          on_connect;
    std::function<void(int fd, std::vector<uint8_t>)>    on_message;
    std::function<void(int fd)>                          on_disconnect;

private:
    void handle_new_connection();
    void handle_client_data(int fd);
    void disconnect_client(int fd);

    std::string  socket_path_;
    int          listen_fd_ = -1;
    EventLoop    event_loop_;
    std::set<int> client_fds_;
    bool         running_ = false;
};

}  // namespace slate
