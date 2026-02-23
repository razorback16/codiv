#pragma once
#include <string>
#include <vector>
#include <functional>
#include <cstdint>
#include "ipc_generated.h"

namespace slate {

class DaemonClient {
public:
    DaemonClient();
    ~DaemonClient();

    DaemonClient(const DaemonClient&) = delete;
    DaemonClient& operator=(const DaemonClient&) = delete;

    // Connect to the daemon's Unix socket. Returns true on success.
    bool connect();

    // Ensure daemon is running, starting it if necessary. Returns true on success.
    bool ensure_daemon_running();

    // Send an ExecuteCommand message
    bool send_execute(const std::string& command, const std::string& cwd,
                      const std::string& request_id);

    // Send an EnvSnapshot message
    bool send_env_snapshot(const std::vector<std::pair<std::string, std::string>>& env_vars,
                           const std::string& path,
                           const std::string& cwd);

    // Send a Heartbeat message
    bool send_heartbeat();

    // Read one message and invoke the handler callback
    using MessageHandler = std::function<void(const slate::ipc::Message*)>;
    bool recv(MessageHandler handler);

    // Connection state
    bool is_connected() const { return fd_ >= 0; }
    int fd() const { return fd_; }
    void disconnect();

private:
    bool send_raw(const uint8_t* data, size_t size);

    int fd_ = -1;
};

} // namespace slate
