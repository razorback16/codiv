#pragma once
#include <string>
#include <cstdlib>
#include <unistd.h>

namespace slate {

constexpr const char* kVersion = "0.1.0";

inline std::string default_socket_path() {
    return "/tmp/slated-" + std::to_string(getuid()) + ".sock";
}

inline std::string pid_file_path() {
    const char* home = std::getenv("HOME");
    return std::string(home ? home : "/tmp") + "/.slate-agent/slated.pid";
}

constexpr size_t kMaxMessageSize = 16 * 1024 * 1024; // 16 MB
constexpr size_t kFrameHeaderSize = 4;

} // namespace slate
