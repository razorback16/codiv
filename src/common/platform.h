#pragma once
#include <vector>
#include <functional>
#include <unistd.h>

#ifdef __APPLE__
#include <sys/event.h>
#else
#include <sys/epoll.h>
#endif

namespace slate {

struct Event {
    int fd;
    bool readable;
    bool writable;
    bool error;
};

class EventLoop {
public:
    EventLoop() {
#ifdef __APPLE__
        kq_ = kqueue();
#else
        epfd_ = epoll_create1(0);
#endif
    }

    ~EventLoop() {
#ifdef __APPLE__
        if (kq_ >= 0) ::close(kq_);
#else
        if (epfd_ >= 0) ::close(epfd_);
#endif
    }

    EventLoop(const EventLoop&) = delete;
    EventLoop& operator=(const EventLoop&) = delete;

    bool add_fd(int fd, bool read = true, bool write = false) {
#ifdef __APPLE__
        std::vector<struct kevent> changes;
        if (read) {
            struct kevent ev;
            EV_SET(&ev, fd, EVFILT_READ, EV_ADD | EV_ENABLE, 0, 0, nullptr);
            changes.push_back(ev);
        }
        if (write) {
            struct kevent ev;
            EV_SET(&ev, fd, EVFILT_WRITE, EV_ADD | EV_ENABLE, 0, 0, nullptr);
            changes.push_back(ev);
        }
        return kevent(kq_, changes.data(), static_cast<int>(changes.size()),
                       nullptr, 0, nullptr) == 0;
#else
        struct epoll_event ev{};
        if (read) ev.events |= EPOLLIN;
        if (write) ev.events |= EPOLLOUT;
        ev.data.fd = fd;
        return epoll_ctl(epfd_, EPOLL_CTL_ADD, fd, &ev) == 0;
#endif
    }

    bool remove_fd(int fd) {
#ifdef __APPLE__
        struct kevent changes[2];
        EV_SET(&changes[0], fd, EVFILT_READ, EV_DELETE, 0, 0, nullptr);
        EV_SET(&changes[1], fd, EVFILT_WRITE, EV_DELETE, 0, 0, nullptr);
        // Ignore errors — filter may not exist
        kevent(kq_, changes, 2, nullptr, 0, nullptr);
        return true;
#else
        return epoll_ctl(epfd_, EPOLL_CTL_DEL, fd, nullptr) == 0;
#endif
    }

    std::vector<Event> wait(int timeout_ms = -1) {
        std::vector<Event> results;
#ifdef __APPLE__
        struct kevent events[64];
        struct timespec ts;
        struct timespec* tsp = nullptr;
        if (timeout_ms >= 0) {
            ts.tv_sec = timeout_ms / 1000;
            ts.tv_nsec = (timeout_ms % 1000) * 1000000L;
            tsp = &ts;
        }
        int n = kevent(kq_, nullptr, 0, events, 64, tsp);
        for (int i = 0; i < n; ++i) {
            Event e{};
            e.fd = static_cast<int>(events[i].ident);
            e.readable = (events[i].filter == EVFILT_READ);
            e.writable = (events[i].filter == EVFILT_WRITE);
            e.error = (events[i].flags & EV_ERROR) != 0;
            results.push_back(e);
        }
#else
        struct epoll_event events[64];
        int n = epoll_wait(epfd_, events, 64, timeout_ms);
        for (int i = 0; i < n; ++i) {
            Event e{};
            e.fd = events[i].data.fd;
            e.readable = (events[i].events & EPOLLIN) != 0;
            e.writable = (events[i].events & EPOLLOUT) != 0;
            e.error = (events[i].events & (EPOLLERR | EPOLLHUP)) != 0;
            results.push_back(e);
        }
#endif
        return results;
    }

private:
#ifdef __APPLE__
    int kq_ = -1;
#else
    int epfd_ = -1;
#endif
};

} // namespace slate
