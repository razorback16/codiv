#include "daemon.h"
#include "types.h"
#include "ipc_protocol.h"
#include <iostream>
#include <fstream>
#include <filesystem>
#include <csignal>
#include <unistd.h>
#include <chrono>

namespace slate {

SlatedDaemon::SlatedDaemon() = default;

SlatedDaemon::~SlatedDaemon() = default;

void SlatedDaemon::ensure_directory(const std::string& path) {
    std::filesystem::create_directories(path);
}

bool SlatedDaemon::is_process_alive(pid_t pid) {
    return ::kill(pid, 0) == 0;
}

void SlatedDaemon::start() {
    pid_file_path_ = slate::pid_file_path();

    // Check for existing daemon
    {
        std::ifstream pf(pid_file_path_);
        if (pf.is_open()) {
            pid_t existing_pid = 0;
            pf >> existing_pid;
            pf.close();
            if (existing_pid > 0 && is_process_alive(existing_pid)) {
                throw std::runtime_error(
                    "daemon already running (pid " + std::to_string(existing_pid) + ")");
            }
            // Stale PID file, remove it
            std::filesystem::remove(pid_file_path_);
        }
    }

    // Create config directory
    auto dir = std::filesystem::path(pid_file_path_).parent_path();
    ensure_directory(dir.string());

    // Write PID file
    {
        std::ofstream pf(pid_file_path_);
        if (!pf.is_open()) {
            throw std::runtime_error("cannot write PID file: " + pid_file_path_);
        }
        pf << ::getpid();
    }

    // Create and configure socket server
    server_ = std::make_unique<SocketServer>(default_socket_path());

    server_->on_connect = [this](int fd) { handle_connect(fd); };
    server_->on_message = [this](int fd, std::vector<uint8_t> data) {
        handle_message(fd, std::move(data));
    };
    server_->on_disconnect = [this](int fd) { handle_disconnect(fd); };

    server_->start();
    std::cerr << "slated: started (pid " << ::getpid() << ")" << std::endl;
}

void SlatedDaemon::run() {
    while (!shutdown_requested_.load(std::memory_order_relaxed)) {
        if (!server_->poll(100)) {
            break;
        }
    }
}

void SlatedDaemon::shutdown() {
    if (server_) {
        server_->stop();
        server_.reset();
    }

    // Remove PID file
    if (!pid_file_path_.empty()) {
        std::filesystem::remove(pid_file_path_);
    }

    sessions_.clear();
    std::cerr << "slated: daemon shutdown complete" << std::endl;
}

void SlatedDaemon::handle_connect(int fd) {
    sessions_.emplace(fd, ClientSession(fd));
}

void SlatedDaemon::handle_message(int fd, std::vector<uint8_t> data) {
    auto* msg = slate::ipc::GetMessage(data.data());
    if (!msg) {
        return;
    }
    dispatch_message(fd, msg);
}

void SlatedDaemon::handle_disconnect(int fd) {
    sessions_.erase(fd);
}

void SlatedDaemon::dispatch_message(int fd, const slate::ipc::Message* msg) {
    using namespace slate::ipc;

    switch (msg->type()) {
    case MessageType_ExecuteCommand: {
        auto* exec = msg->body_as_ExecuteCommandMsg();
        if (exec) {
            // Phase 2 will implement actual command execution.
            // For now, update session cwd and acknowledge.
            auto it = sessions_.find(fd);
            if (it != sessions_.end() && exec->cwd()) {
                it->second.update_cwd(exec->cwd()->str());
            }
            const char* req_id = exec->request_id() ? exec->request_id()->c_str() : "";
            send_ack(fd, req_id);
        }
        break;
    }
    case MessageType_EnvSnapshot: {
        auto* env = msg->body_as_EnvSnapshotMsg();
        if (env) {
            auto it = sessions_.find(fd);
            if (it != sessions_.end()) {
                std::vector<std::pair<std::string, std::string>> vars;
                if (env->env_vars()) {
                    for (auto* kv : *env->env_vars()) {
                        if (kv->key() && kv->value()) {
                            vars.emplace_back(kv->key()->str(), kv->value()->str());
                        }
                    }
                }
                it->second.update_env(
                    vars,
                    env->path() ? env->path()->str() : "",
                    env->aliases() ? env->aliases()->str() : "",
                    env->functions() ? env->functions()->str() : ""
                );
                if (env->cwd()) {
                    it->second.update_cwd(env->cwd()->str());
                }
            }
        }
        break;
    }
    case MessageType_Heartbeat: {
        auto it = sessions_.find(fd);
        if (it != sessions_.end()) {
            it->second.touch_heartbeat();
        }
        send_heartbeat_response(fd);
        break;
    }
    case MessageType_Shutdown: {
        request_shutdown();
        break;
    }
    default:
        break;
    }
}

void SlatedDaemon::send_heartbeat_response(int fd) {
    flatbuffers::FlatBufferBuilder builder(128);
    auto ts = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::system_clock::now().time_since_epoch()).count();
    auto hb = slate::ipc::CreateHeartbeatMsg(builder, ts);
    auto msg = slate::ipc::CreateMessage(builder,
        slate::ipc::MessageType_Heartbeat,
        slate::ipc::MessageBody_HeartbeatMsg,
        hb.Union());
    builder.Finish(msg);

    server_->send(fd, builder.GetBufferPointer(), builder.GetSize());
}

void SlatedDaemon::send_ack(int fd, const char* request_id) {
    flatbuffers::FlatBufferBuilder builder(128);
    auto comp = slate::ipc::CreateCommandCompleteMsgDirect(builder, request_id, 0);
    auto msg = slate::ipc::CreateMessage(builder,
        slate::ipc::MessageType_CommandComplete,
        slate::ipc::MessageBody_CommandCompleteMsg,
        comp.Union());
    builder.Finish(msg);

    server_->send(fd, builder.GetBufferPointer(), builder.GetSize());
}

} // namespace slate
