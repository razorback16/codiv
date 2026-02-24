#pragma once

#include <cstdint>
#include <cstddef>
#include <vector>
#include <optional>
#include <string>

#include "flatbuffers/flatbuffers.h"

namespace slate {

/// Wrap a FlatBufferBuilder's contents in a length-prefixed frame.
std::vector<uint8_t> frame(const flatbuffers::FlatBufferBuilder& builder);

/// Read one framed message from a file descriptor.  Returns nullopt on
/// EOF, short read, or over-size payload.
std::optional<std::vector<uint8_t>> read_message(int fd);

/// Write a framed message to a file descriptor.
bool write_message(int fd, const std::vector<uint8_t>& data);
bool write_message(int fd, const uint8_t* data, size_t size);

}  // namespace slate
