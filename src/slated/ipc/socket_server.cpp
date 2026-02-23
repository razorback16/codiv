#include "socket_server.h"
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>
#include <fcntl.h>
#include <cerrno>
#include <cstring>
#include <stdexcept>
#include <iostream>

namespace slate {

SocketServer::SocketServer(const std::string& socket_path)
    : socket_path_(socket_path) {}

SocketServer::~SocketServer() {
    if (running_) {
        stop();
    }
}

void SocketServer::start() {
    listen_fd_ = ::socket(AF_UNIX, SOCK_STREAM, 0);
    if (listen_fd_ < 0) {
        throw std::runtime_error("socket() failed: " + std::string(strerror(errno)));
    }

    // Set non-blocking on listen fd
    int flags = fcntl(listen_fd_, F_GETFL, 0);
    if (flags >= 0) {
        fcntl(listen_fd_, F_SETFL, flags | O_NONBLOCK);
    }

    // Remove existing socket file
    ::unlink(socket_path_.c_str());

    struct sockaddr_un addr{};
    addr.sun_family = AF_UNIX;
    if (socket_path_.size() >= sizeof(addr.sun_path)) {
        ::close(listen_fd_);
        listen_fd_ = -1;
        throw std::runtime_error("socket path too long");
    }
    std::strncpy(addr.sun_path, socket_path_.c_str(), sizeof(addr.sun_path) - 1);

    if (::bind(listen_fd_, reinterpret_cast<struct sockaddr*>(&addr), sizeof(addr)) < 0) {
        ::close(listen_fd_);
        listen_fd_ = -1;
        throw std::runtime_error("bind() failed: " + std::string(strerror(errno)));
    }

    if (::listen(listen_fd_, 5) < 0) {
        ::close(listen_fd_);
        listen_fd_ = -1;
        ::unlink(socket_path_.c_str());
        throw std::runtime_error("listen() failed: " + std::string(strerror(errno)));
    }

    event_loop_.add_fd(listen_fd_, true, false);
    running_ = true;
}

bool SocketServer::poll(int timeout_ms) {
    if (!running_) return false;

    auto events = event_loop_.wait(timeout_ms);
    for (const auto& ev : events) {
        if (ev.fd == listen_fd_) {
            if (ev.readable) {
                handle_new_connection();
            }
        } else if (client_fds_.count(ev.fd)) {
            if (ev.error) {
                disconnect_client(ev.fd);
            } else if (ev.readable) {
                handle_client_data(ev.fd);
            }
        }
    }
    return true;
}

void SocketServer::run() {
    while (running_) {
        poll(100);
    }
}

void SocketServer::handle_new_connection() {
    struct sockaddr_un client_addr{};
    socklen_t client_len = sizeof(client_addr);
    int client_fd = ::accept(listen_fd_,
                             reinterpret_cast<struct sockaddr*>(&client_addr),
                             &client_len);
    if (client_fd < 0) {
        return; // Non-blocking accept, EAGAIN is normal
    }

    client_fds_.insert(client_fd);
    event_loop_.add_fd(client_fd, true, false);

    if (on_connect) {
        on_connect(client_fd);
    }
}

void SocketServer::handle_client_data(int fd) {
    auto msg = read_message(fd);
    if (!msg.has_value()) {
        // EOF or read error — client disconnected
        disconnect_client(fd);
        return;
    }

    if (on_message) {
        on_message(fd, std::move(*msg));
    }
}

void SocketServer::disconnect_client(int fd) {
    event_loop_.remove_fd(fd);
    client_fds_.erase(fd);

    if (on_disconnect) {
        on_disconnect(fd);
    }

    ::close(fd);
}

bool SocketServer::send(int fd, const std::vector<uint8_t>& data) {
    return write_message(fd, data);
}

bool SocketServer::send(int fd, const uint8_t* data, size_t size) {
    return write_message(fd, data, size);
}

void SocketServer::stop() {
    running_ = false;

    // Close all client connections
    for (int fd : client_fds_) {
        event_loop_.remove_fd(fd);
        ::close(fd);
    }
    client_fds_.clear();

    // Close listen socket
    if (listen_fd_ >= 0) {
        event_loop_.remove_fd(listen_fd_);
        ::close(listen_fd_);
        listen_fd_ = -1;
    }

    // Remove socket file
    ::unlink(socket_path_.c_str());
}

} // namespace slate
