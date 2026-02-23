#pragma once
#include <cstdint>
#include <cstddef>
#include <vector>
#include <optional>
#include <string>
#include "flatbuffers/flatbuffers.h"

namespace slate {

// Frame a FlatBuffer builder into length-prefixed bytes
std::vector<uint8_t> frame(const flatbuffers::FlatBufferBuilder& builder);

// Read one framed message from fd. Returns nullopt on EOF/error.
std::optional<std::vector<uint8_t>> read_message(int fd);

// Write one framed message to fd. Returns true on success.
bool write_message(int fd, const std::vector<uint8_t>& data);
bool write_message(int fd, const uint8_t* data, size_t size);

} // namespace slate
