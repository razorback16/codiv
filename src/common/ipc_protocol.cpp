#include "ipc_protocol.h"
#include "types.h"
#include <arpa/inet.h>
#include <unistd.h>
#include <cerrno>

namespace slate {

std::vector<uint8_t> frame(const flatbuffers::FlatBufferBuilder& builder) {
    auto size = builder.GetSize();
    auto buf = builder.GetBufferPointer();

    std::vector<uint8_t> framed(kFrameHeaderSize + size);
    uint32_t net_size = htonl(static_cast<uint32_t>(size));
    std::memcpy(framed.data(), &net_size, kFrameHeaderSize);
    std::memcpy(framed.data() + kFrameHeaderSize, buf, size);
    return framed;
}

static bool read_exact(int fd, uint8_t* buf, size_t count) {
    size_t total = 0;
    while (total < count) {
        auto n = ::read(fd, buf + total, count - total);
        if (n <= 0) return false;
        total += static_cast<size_t>(n);
    }
    return true;
}

static bool write_exact(int fd, const uint8_t* buf, size_t count) {
    size_t total = 0;
    while (total < count) {
        auto n = ::write(fd, buf + total, count - total);
        if (n <= 0) return false;
        total += static_cast<size_t>(n);
    }
    return true;
}

std::optional<std::vector<uint8_t>> read_message(int fd) {
    uint8_t header[kFrameHeaderSize];
    if (!read_exact(fd, header, kFrameHeaderSize)) {
        return std::nullopt;
    }

    uint32_t net_size;
    std::memcpy(&net_size, header, kFrameHeaderSize);
    uint32_t payload_size = ntohl(net_size);

    if (payload_size > kMaxMessageSize) {
        return std::nullopt;
    }

    std::vector<uint8_t> payload(payload_size);
    if (!read_exact(fd, payload.data(), payload_size)) {
        return std::nullopt;
    }

    return payload;
}

bool write_message(int fd, const std::vector<uint8_t>& data) {
    return write_message(fd, data.data(), data.size());
}

bool write_message(int fd, const uint8_t* data, size_t size) {
    if (size > kMaxMessageSize) return false;

    uint32_t net_size = htonl(static_cast<uint32_t>(size));
    uint8_t header[kFrameHeaderSize];
    std::memcpy(header, &net_size, kFrameHeaderSize);

    if (!write_exact(fd, header, kFrameHeaderSize)) return false;
    if (!write_exact(fd, data, size)) return false;
    return true;
}

} // namespace slate
