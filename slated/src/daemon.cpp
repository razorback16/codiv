#include "daemon.h"
#include "types.h"
#include "ipc_protocol.h"

#include <iostream>
#include <fstream>
#include <filesystem>
#include <csignal>
#include <unistd.h>
#include <sys/wait.h>
#include <chrono>

namespace slate {

SlatedDaemon::SlatedDaemon()
    : last_cleanup_(std::chrono::steady_clock::now()) {}

SlatedDaemon::~SlatedDaemon() = default;

void SlatedDaemon::ensure_directory(const std::string& path) {
    std::filesystem::create_directories(path);
}

bool SlatedDaemon::is_process_alive(pid_t pid) {
    return ::kill(pid, 0) == 0;
}

void SlatedDaemon::start() {
    pid_file_path_ = slate::pid_file_path();

    // Check for an existing daemon process.
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
            std::filesystem::remove(pid_file_path_);
        }
    }

    auto dir = std::filesystem::path(pid_file_path_).parent_path();
    ensure_directory(dir.string());

    {
        std::ofstream pf(pid_file_path_);
        if (!pf.is_open()) {
            throw std::runtime_error("cannot write PID file: " + pid_file_path_);
        }
        pf << ::getpid();
    }

    server_ = std::make_unique<SocketServer>(default_socket_path());
    server_->on_connect    = [this](int fd) { handle_connect(fd); };
    server_->on_message    = [this](int fd, std::vector<uint8_t> data) {
        handle_message(fd, std::move(data));
    };
    server_->on_disconnect = [this](int fd) { handle_disconnect(fd); };
    server_->start();

    std::cerr << "slated: started (pid " << ::getpid() << ")" << std::endl;
}

void SlatedDaemon::run() {
    while (!shutdown_requested_.load(std::memory_order_relaxed)) {
        if (!server_->poll(50)) break;

        poll_workers();
        reap_finished_workers();

        // Periodic stale session cleanup every 30s.
        auto now = std::chrono::steady_clock::now();
        if (now - last_cleanup_ > std::chrono::seconds(30)) {
            cleanup_stale_sessions();
            last_cleanup_ = now;
        }
    }
}

void SlatedDaemon::shutdown() {
    // Kill all active workers.
    for (auto& w : workers_) {
        if (w.pid > 0) {
            ::kill(w.pid, SIGTERM);
        }
    }
    // Reap workers.
    for (auto& w : workers_) {
        if (w.pid > 0) {
            int status = 0;
            ::waitpid(w.pid, &status, 0);
        }
        if (w.stdout_fd >= 0) ::close(w.stdout_fd);
        if (w.stderr_fd >= 0) ::close(w.stderr_fd);
    }
    workers_.clear();

    if (server_) {
        server_->stop();
        server_.reset();
    }
    if (!pid_file_path_.empty()) {
        std::filesystem::remove(pid_file_path_);
    }
    sessions_.clear();
    std::cerr << "slated: daemon shutdown complete" << std::endl;
}

// ---------------------------------------------------------------------------
// Connection callbacks
// ---------------------------------------------------------------------------

void SlatedDaemon::handle_connect(int fd) {
    sessions_.emplace(fd, ClientSession(fd));
}

void SlatedDaemon::handle_message(int fd, std::vector<uint8_t> data) {
    auto* msg = slate::ipc::GetMessage(data.data());
    if (!msg) return;
    dispatch_message(fd, msg);
}

void SlatedDaemon::handle_disconnect(int fd) {
    kill_workers_for_client(fd);
    sessions_.erase(fd);
}

// ---------------------------------------------------------------------------
// Message dispatch
// ---------------------------------------------------------------------------

void SlatedDaemon::dispatch_message(int fd, const slate::ipc::Message* msg) {
    using namespace slate::ipc;

    switch (msg->type()) {
    case MessageType_ExecuteCommand: {
        auto* exec = msg->body_as_ExecuteCommandMsg();
        if (exec) {
            start_command(fd, exec);
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
                    env->path()      ? env->path()->str()      : "",
                    env->aliases()   ? env->aliases()->str()   : "",
                    env->functions() ? env->functions()->str() : "");

                if (env->cwd()) {
                    it->second.update_cwd(env->cwd()->str());
                }
                std::cerr << "slated: received env snapshot from client fd=" << fd << std::endl;
            }
        }
        break;
    }

    case MessageType_Heartbeat: {
        auto it = sessions_.find(fd);
        if (it != sessions_.end()) it->second.touch_heartbeat();
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

// ---------------------------------------------------------------------------
// Worker management
// ---------------------------------------------------------------------------

void SlatedDaemon::start_command(int fd, const slate::ipc::ExecuteCommandMsg* exec) {
    auto it = sessions_.find(fd);
    if (it == sessions_.end()) {
        const char* req_id = exec->request_id() ? exec->request_id()->c_str() : "";
        send_error(fd, req_id, "no session for this connection");
        return;
    }

    auto& session = it->second;
    std::string command = exec->command() ? exec->command()->str() : "";
    std::string cwd = exec->cwd() ? exec->cwd()->str() : session.cwd();
    std::string req_id = exec->request_id() ? exec->request_id()->str() : "";

    if (command.empty()) {
        send_error(fd, req_id, "empty command");
        return;
    }

    // Update session cwd if provided.
    if (exec->cwd()) {
        session.update_cwd(exec->cwd()->str());
    }

    auto worker = spawn_worker(command, cwd, session.env_vars(), session.path(), fd, req_id);
    if (worker.pid <= 0) {
        send_error(fd, req_id, "failed to spawn worker");
        return;
    }

    std::cerr << "slated: spawned worker pid=" << worker.pid
              << " for command: " << command << std::endl;
    workers_.push_back(std::move(worker));
}

void SlatedDaemon::poll_workers() {
    static constexpr size_t kReadBufSize = 8192;
    uint8_t buf[kReadBufSize];

    for (auto& w : workers_) {
        // Read stdout.
        if (!w.stdout_eof && w.stdout_fd >= 0) {
            ssize_t n = ::read(w.stdout_fd, buf, kReadBufSize);
            if (n > 0) {
                send_command_output(w.client_fd, w.request_id, buf, static_cast<size_t>(n), false);
            } else if (n == 0) {
                w.stdout_eof = true;
                ::close(w.stdout_fd);
                w.stdout_fd = -1;
            }
            // n < 0 with EAGAIN/EWOULDBLOCK is normal for non-blocking — ignore.
        }

        // Read stderr.
        if (!w.stderr_eof && w.stderr_fd >= 0) {
            ssize_t n = ::read(w.stderr_fd, buf, kReadBufSize);
            if (n > 0) {
                send_command_output(w.client_fd, w.request_id, buf, static_cast<size_t>(n), true);
            } else if (n == 0) {
                w.stderr_eof = true;
                ::close(w.stderr_fd);
                w.stderr_fd = -1;
            }
        }
    }
}

void SlatedDaemon::reap_finished_workers() {
    auto it = workers_.begin();
    while (it != workers_.end()) {
        if (!it->is_done()) {
            ++it;
            continue;
        }

        int status = 0;
        pid_t result = ::waitpid(it->pid, &status, WNOHANG);
        if (result == 0) {
            // Process still running even though pipes closed — wait.
            ++it;
            continue;
        }

        int exit_code = 0;
        if (result > 0) {
            if (WIFEXITED(status)) {
                exit_code = WEXITSTATUS(status);
            } else if (WIFSIGNALED(status)) {
                exit_code = 128 + WTERMSIG(status);
            }
        }

        send_command_complete(it->client_fd, it->request_id, exit_code);
        std::cerr << "slated: worker pid=" << it->pid
                  << " finished (exit_code=" << exit_code << ")" << std::endl;

        it = workers_.erase(it);
    }
}

void SlatedDaemon::cleanup_stale_sessions() {
    auto it = sessions_.begin();
    while (it != sessions_.end()) {
        if (it->second.is_stale(std::chrono::seconds(120))) {
            std::cerr << "slated: cleaning stale session fd=" << it->first << std::endl;
            kill_workers_for_client(it->first);
            // Disconnect via server (closes fd and removes from event loop).
            // We just erase from our map; the socket will be closed on next poll error.
            it = sessions_.erase(it);
        } else {
            ++it;
        }
    }
}

void SlatedDaemon::kill_workers_for_client(int fd) {
    auto it = workers_.begin();
    while (it != workers_.end()) {
        if (it->client_fd == fd) {
            if (it->pid > 0) {
                ::kill(it->pid, SIGTERM);
                int status = 0;
                ::waitpid(it->pid, &status, 0);  // blocking reap
            }
            if (it->stdout_fd >= 0) ::close(it->stdout_fd);
            if (it->stderr_fd >= 0) ::close(it->stderr_fd);
            it = workers_.erase(it);
        } else {
            ++it;
        }
    }
}

// ---------------------------------------------------------------------------
// Response helpers
// ---------------------------------------------------------------------------

void SlatedDaemon::send_heartbeat_response(int fd) {
    flatbuffers::FlatBufferBuilder builder(128);

    auto ts = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::system_clock::now().time_since_epoch()).count();

    auto hb = slate::ipc::CreateHeartbeatMsg(builder, ts);
    auto msg = slate::ipc::CreateMessage(
        builder,
        slate::ipc::MessageType_Heartbeat,
        slate::ipc::MessageBody_HeartbeatMsg,
        hb.Union());
    builder.Finish(msg);

    server_->send(fd, builder.GetBufferPointer(), builder.GetSize());
}

void SlatedDaemon::send_command_output(int fd, const std::string& request_id,
                                        const uint8_t* data, size_t len, bool is_stderr) {
    flatbuffers::FlatBufferBuilder builder(256 + len);

    auto rid = builder.CreateString(request_id);
    auto data_vec = builder.CreateVector(reinterpret_cast<const int8_t*>(data), len);

    auto out = slate::ipc::CreateCommandOutputMsg(builder, rid, data_vec, is_stderr);
    auto msg = slate::ipc::CreateMessage(
        builder,
        slate::ipc::MessageType_CommandOutput,
        slate::ipc::MessageBody_CommandOutputMsg,
        out.Union());
    builder.Finish(msg);

    server_->send(fd, builder.GetBufferPointer(), builder.GetSize());
}

void SlatedDaemon::send_command_complete(int fd, const std::string& request_id, int exit_code) {
    flatbuffers::FlatBufferBuilder builder(128);

    auto comp = slate::ipc::CreateCommandCompleteMsgDirect(
        builder, request_id.c_str(), exit_code);
    auto msg = slate::ipc::CreateMessage(
        builder,
        slate::ipc::MessageType_CommandComplete,
        slate::ipc::MessageBody_CommandCompleteMsg,
        comp.Union());
    builder.Finish(msg);

    server_->send(fd, builder.GetBufferPointer(), builder.GetSize());
}

void SlatedDaemon::send_error(int fd, const std::string& request_id, const std::string& message) {
    flatbuffers::FlatBufferBuilder builder(256);

    auto err = slate::ipc::CreateErrorMsgDirect(
        builder, request_id.c_str(), message.c_str());
    auto msg = slate::ipc::CreateMessage(
        builder,
        slate::ipc::MessageType_Error,
        slate::ipc::MessageBody_ErrorMsg,
        err.Union());
    builder.Finish(msg);

    server_->send(fd, builder.GetBufferPointer(), builder.GetSize());
}

}  // namespace slate
