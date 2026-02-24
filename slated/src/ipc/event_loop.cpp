#include "event_loop.h"

#include <stdexcept>
#include <unistd.h>

#ifdef __linux__
// ---- Linux: epoll ----
#include <sys/epoll.h>

namespace slate {

EventLoop::EventLoop() {
    epoll_fd_ = ::epoll_create1(0);
    if (epoll_fd_ < 0) {
        throw std::runtime_error("epoll_create1 failed");
    }
}

EventLoop::~EventLoop() {
    if (epoll_fd_ >= 0) ::close(epoll_fd_);
}

bool EventLoop::add_fd(int fd, bool read, bool write) {
    struct epoll_event ev{};
    if (read)  ev.events |= EPOLLIN;
    if (write) ev.events |= EPOLLOUT;
    ev.data.fd = fd;
    return ::epoll_ctl(epoll_fd_, EPOLL_CTL_ADD, fd, &ev) == 0;
}

bool EventLoop::remove_fd(int fd) {
    return ::epoll_ctl(epoll_fd_, EPOLL_CTL_DEL, fd, nullptr) == 0;
}

std::vector<Event> EventLoop::wait(int timeout_ms) {
    static constexpr int kMaxEvents = 64;
    struct epoll_event events[kMaxEvents];

    int n = ::epoll_wait(epoll_fd_, events, kMaxEvents, timeout_ms);
    std::vector<Event> result;
    if (n < 0) return result;  // interrupted or error

    result.reserve(static_cast<size_t>(n));
    for (int i = 0; i < n; ++i) {
        Event e;
        e.fd       = events[i].data.fd;
        e.readable = (events[i].events & EPOLLIN)  != 0;
        e.writable = (events[i].events & EPOLLOUT) != 0;
        e.error    = (events[i].events & (EPOLLERR | EPOLLHUP)) != 0;
        result.push_back(e);
    }
    return result;
}

}  // namespace slate

#elif defined(__APPLE__)
// ---- macOS: kqueue ----
#include <sys/types.h>
#include <sys/event.h>
#include <sys/time.h>

namespace slate {

EventLoop::EventLoop() {
    epoll_fd_ = ::kqueue();  // reuse the member name for the kqueue fd
    if (epoll_fd_ < 0) {
        throw std::runtime_error("kqueue failed");
    }
}

EventLoop::~EventLoop() {
    if (epoll_fd_ >= 0) ::close(epoll_fd_);
}

bool EventLoop::add_fd(int fd, bool read, bool write) {
    struct kevent changes[2];
    int n = 0;
    if (read) {
        EV_SET(&changes[n], fd, EVFILT_READ, EV_ADD | EV_ENABLE, 0, 0, nullptr);
        ++n;
    }
    if (write) {
        EV_SET(&changes[n], fd, EVFILT_WRITE, EV_ADD | EV_ENABLE, 0, 0, nullptr);
        ++n;
    }
    return ::kevent(epoll_fd_, changes, n, nullptr, 0, nullptr) == 0;
}

bool EventLoop::remove_fd(int fd) {
    struct kevent changes[2];
    EV_SET(&changes[0], fd, EVFILT_READ,  EV_DELETE, 0, 0, nullptr);
    EV_SET(&changes[1], fd, EVFILT_WRITE, EV_DELETE, 0, 0, nullptr);
    // Ignore errors — the filter might not have been registered.
    ::kevent(epoll_fd_, changes, 2, nullptr, 0, nullptr);
    return true;
}

std::vector<Event> EventLoop::wait(int timeout_ms) {
    static constexpr int kMaxEvents = 64;
    struct kevent events[kMaxEvents];

    struct timespec ts;
    struct timespec* ts_ptr = nullptr;
    if (timeout_ms >= 0) {
        ts.tv_sec  = timeout_ms / 1000;
        ts.tv_nsec = (timeout_ms % 1000) * 1000000L;
        ts_ptr = &ts;
    }

    int n = ::kevent(epoll_fd_, nullptr, 0, events, kMaxEvents, ts_ptr);
    std::vector<Event> result;
    if (n < 0) return result;

    result.reserve(static_cast<size_t>(n));
    for (int i = 0; i < n; ++i) {
        Event e;
        e.fd       = static_cast<int>(events[i].ident);
        e.readable = (events[i].filter == EVFILT_READ);
        e.writable = (events[i].filter == EVFILT_WRITE);
        e.error    = (events[i].flags & EV_ERROR) != 0;
        result.push_back(e);
    }
    return result;
}

}  // namespace slate

#else
#error "Unsupported platform — need epoll (Linux) or kqueue (macOS)"
#endif
