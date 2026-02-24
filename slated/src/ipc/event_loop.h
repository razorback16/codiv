#pragma once

#include <vector>

namespace slate {

/// A single I/O event returned by EventLoop::wait().
struct Event {
    int  fd;
    bool readable;
    bool writable;
    bool error;
};

/// Thin abstraction over epoll (Linux) / kqueue (macOS).
class EventLoop {
public:
    EventLoop();
    ~EventLoop();

    // Non-copyable, non-movable (owns the epoll/kqueue fd).
    EventLoop(const EventLoop&) = delete;
    EventLoop& operator=(const EventLoop&) = delete;

    /// Register a file descriptor for monitoring.
    bool add_fd(int fd, bool read = true, bool write = false);

    /// Unregister a file descriptor.
    bool remove_fd(int fd);

    /// Block for up to \p timeout_ms milliseconds and return ready events.
    /// A negative timeout means wait indefinitely.
    std::vector<Event> wait(int timeout_ms = -1);

private:
    int epoll_fd_ = -1;
};

}  // namespace slate
