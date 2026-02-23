#include "daemon_client.h"
#include "types.h"
#include "ipc_protocol.h"
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>
#include <cerrno>
#include <cstring>
#include <chrono>
#include <thread>
#include <fcntl.h>
#include <iostream>

namespace slate {

DaemonClient::DaemonClient() = default;

DaemonClient::~DaemonClient() {
    disconnect();
}

bool DaemonClient::connect() {
    if (fd_ >= 0) {
        return true; // Already connected
    }

    fd_ = ::socket(AF_UNIX, SOCK_STREAM, 0);
    if (fd_ < 0) {
        return false;
    }

    struct sockaddr_un addr{};
    addr.sun_family = AF_UNIX;
    auto path = default_socket_path();
    if (path.size() >= sizeof(addr.sun_path)) {
        ::close(fd_);
        fd_ = -1;
        return false;
    }
    std::strncpy(addr.sun_path, path.c_str(), sizeof(addr.sun_path) - 1);

    if (::connect(fd_, reinterpret_cast<struct sockaddr*>(&addr), sizeof(addr)) < 0) {
        ::close(fd_);
        fd_ = -1;
        return false;
    }

    return true;
}

bool DaemonClient::ensure_daemon_running() {
    // Try connecting first
    if (connect()) {
        return true;
    }

    // Start the daemon via fork+exec
    pid_t pid = ::fork();
    if (pid < 0) {
        return false;
    }

    if (pid == 0) {
        // Child: exec slated
        // Detach from controlling terminal
        ::setsid();
        // Redirect stdin/stdout/stderr to /dev/null for daemon behavior
        int devnull = ::open("/dev/null", O_RDWR);
        if (devnull >= 0) {
            ::dup2(devnull, STDIN_FILENO);
            ::dup2(devnull, STDOUT_FILENO);
            // Keep stderr for logging
            ::close(devnull);
        }
        ::execlp("slated", "slated", nullptr);
        // If exec fails, exit child
        ::_exit(127);
    }

    // Parent: poll for socket availability
    for (int i = 0; i < 10; ++i) {
        std::this_thread::sleep_for(std::chrono::milliseconds(100));
        if (connect()) {
            return true;
        }
    }

    return false;
}

bool DaemonClient::send_execute(const std::string& command, const std::string& cwd,
                                 const std::string& request_id) {
    if (fd_ < 0) return false;

    flatbuffers::FlatBufferBuilder builder(256);
    auto cmd = slate::ipc::CreateExecuteCommandMsgDirect(
        builder, command.c_str(), cwd.c_str(), request_id.c_str());
    auto msg = slate::ipc::CreateMessage(builder,
        slate::ipc::MessageType_ExecuteCommand,
        slate::ipc::MessageBody_ExecuteCommandMsg,
        cmd.Union());
    builder.Finish(msg);

    return send_raw(builder.GetBufferPointer(), builder.GetSize());
}

bool DaemonClient::send_env_snapshot(
    const std::vector<std::pair<std::string, std::string>>& env_vars,
    const std::string& path,
    const std::string& cwd) {
    if (fd_ < 0) return false;

    flatbuffers::FlatBufferBuilder builder(1024);

    std::vector<flatbuffers::Offset<slate::ipc::KeyValue>> kv_offsets;
    for (const auto& [k, v] : env_vars) {
        kv_offsets.push_back(
            slate::ipc::CreateKeyValueDirect(builder, k.c_str(), v.c_str()));
    }

    auto env = slate::ipc::CreateEnvSnapshotMsgDirect(
        builder, &kv_offsets, path.c_str(), nullptr, nullptr, cwd.c_str());
    auto msg = slate::ipc::CreateMessage(builder,
        slate::ipc::MessageType_EnvSnapshot,
        slate::ipc::MessageBody_EnvSnapshotMsg,
        env.Union());
    builder.Finish(msg);

    return send_raw(builder.GetBufferPointer(), builder.GetSize());
}

bool DaemonClient::send_heartbeat() {
    if (fd_ < 0) return false;

    flatbuffers::FlatBufferBuilder builder(128);
    auto ts = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::system_clock::now().time_since_epoch()).count();
    auto hb = slate::ipc::CreateHeartbeatMsg(builder, ts);
    auto msg = slate::ipc::CreateMessage(builder,
        slate::ipc::MessageType_Heartbeat,
        slate::ipc::MessageBody_HeartbeatMsg,
        hb.Union());
    builder.Finish(msg);

    return send_raw(builder.GetBufferPointer(), builder.GetSize());
}

bool DaemonClient::recv(MessageHandler handler) {
    if (fd_ < 0) return false;

    auto data = read_message(fd_);
    if (!data.has_value()) {
        return false;
    }

    auto* msg = slate::ipc::GetMessage(data->data());
    if (msg && handler) {
        handler(msg);
    }
    return true;
}

void DaemonClient::disconnect() {
    if (fd_ >= 0) {
        ::close(fd_);
        fd_ = -1;
    }
}

bool DaemonClient::send_raw(const uint8_t* data, size_t size) {
    return write_message(fd_, data, size);
}

} // namespace slate
