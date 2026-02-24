#include "socket_server.h"
#include "../types.h"

#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>
#include <fcntl.h>
#include <cerrno>
#include <cstring>
#include <stdexcept>
#include <iostream>
#include <filesystem>

namespace slate {

SocketServer::SocketServer(const std::string& socket_path)
    : socket_path_(socket_path) {}

SocketServer::~SocketServer() {
    stop();
}

void SocketServer::start() {
    // Remove stale socket file if it exists.
    std::filesystem::remove(socket_path_);

    listen_fd_ = ::socket(AF_UNIX, SOCK_STREAM, 0);
    if (listen_fd_ < 0) {
        throw std::runtime_error("socket() failed: " + std::string(std::strerror(errno)));
    }

    // Set non-blocking on the listen socket.
    int flags = ::fcntl(listen_fd_, F_GETFL, 0);
    ::fcntl(listen_fd_, F_SETFL, flags | O_NONBLOCK);

    struct sockaddr_un addr{};
    addr.sun_family = AF_UNIX;
    std::strncpy(addr.sun_path, socket_path_.c_str(), sizeof(addr.sun_path) - 1);

    if (::bind(listen_fd_, reinterpret_cast<struct sockaddr*>(&addr), sizeof(addr)) < 0) {
        ::close(listen_fd_);
        listen_fd_ = -1;
        throw std::runtime_error("bind() failed on " + socket_path_ + ": " + std::strerror(errno));
    }

    if (::listen(listen_fd_, SOMAXCONN) < 0) {
        ::close(listen_fd_);
        listen_fd_ = -1;
        throw std::runtime_error("listen() failed: " + std::string(std::strerror(errno)));
    }

    event_loop_.add_fd(listen_fd_, /*read=*/true, /*write=*/false);
    running_ = true;

    std::cerr << "slated: listening on " << socket_path_ << std::endl;
}

bool SocketServer::poll(int timeout_ms) {
    if (!running_) return false;

    auto events = event_loop_.wait(timeout_ms);
    for (const auto& ev : events) {
        if (ev.fd == listen_fd_) {
            if (ev.readable) handle_new_connection();
        } else {
            if (ev.error) {
                disconnect_client(ev.fd);
            } else if (ev.readable) {
                handle_client_data(ev.fd);
            }
        }
    }
    return running_;
}

void SocketServer::run() {
    while (running_) {
        if (!poll(100)) break;
    }
}

bool SocketServer::send(int fd, const std::vector<uint8_t>& data) {
    return write_message(fd, data);
}

bool SocketServer::send(int fd, const uint8_t* data, size_t size) {
    return write_message(fd, data, size);
}

void SocketServer::stop() {
    running_ = false;

    // Disconnect all clients.
    auto fds = client_fds_;  // copy — disconnect_client modifies client_fds_
    for (int fd : fds) {
        disconnect_client(fd);
    }

    if (listen_fd_ >= 0) {
        event_loop_.remove_fd(listen_fd_);
        ::close(listen_fd_);
        listen_fd_ = -1;
    }

    // Clean up the socket file.
    std::filesystem::remove(socket_path_);
}

void SocketServer::handle_new_connection() {
    struct sockaddr_un client_addr{};
    socklen_t len = sizeof(client_addr);
    int client_fd = ::accept(listen_fd_, reinterpret_cast<struct sockaddr*>(&client_addr), &len);
    if (client_fd < 0) return;

    // Set non-blocking on the client socket.
    int flags = ::fcntl(client_fd, F_GETFL, 0);
    ::fcntl(client_fd, F_SETFL, flags | O_NONBLOCK);

    event_loop_.add_fd(client_fd, /*read=*/true, /*write=*/false);
    client_fds_.insert(client_fd);

    if (on_connect) on_connect(client_fd);
}

void SocketServer::handle_client_data(int fd) {
    auto msg = read_message(fd);
    if (!msg) {
        disconnect_client(fd);
        return;
    }
    if (on_message) on_message(fd, std::move(*msg));
}

void SocketServer::disconnect_client(int fd) {
    event_loop_.remove_fd(fd);
    ::close(fd);
    client_fds_.erase(fd);
    if (on_disconnect) on_disconnect(fd);
}

}  // namespace slate
