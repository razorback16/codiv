#pragma once

#include <string>
#include <vector>
#include <utility>
#include <chrono>

namespace slate {

class ClientSession {
public:
    explicit ClientSession(int fd)
        : fd_(fd), last_heartbeat_(std::chrono::steady_clock::now()) {}

    ClientSession() = default;
    ~ClientSession() = default;

    int fd() const { return fd_; }

    void update_env(const std::vector<std::pair<std::string, std::string>>& env_vars,
                    const std::string& path,
                    const std::string& aliases,
                    const std::string& functions) {
        env_vars_ = env_vars;
        path_ = path;
        aliases_ = aliases;
        functions_ = functions;
    }

    void update_cwd(const std::string& cwd) { cwd_ = cwd; }

    void touch_heartbeat() { last_heartbeat_ = std::chrono::steady_clock::now(); }

    bool is_stale(std::chrono::seconds timeout) const {
        return (std::chrono::steady_clock::now() - last_heartbeat_) > timeout;
    }

    const std::string& cwd() const { return cwd_; }
    const std::string& path() const { return path_; }
    const std::vector<std::pair<std::string, std::string>>& env_vars() const { return env_vars_; }

private:
    int fd_ = -1;
    std::string cwd_;
    std::vector<std::pair<std::string, std::string>> env_vars_;
    std::string path_;
    std::string aliases_;
    std::string functions_;
    std::chrono::steady_clock::time_point last_heartbeat_;
};

}  // namespace slate
