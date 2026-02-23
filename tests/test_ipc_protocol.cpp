#include <gtest/gtest.h>
#include <sys/socket.h>
#include <unistd.h>
#include <arpa/inet.h>
#include <cstring>
#include <vector>
#include "ipc_protocol.h"
#include "types.h"
#include "ipc_generated.h"

using namespace slate;
using namespace slate::ipc;

TEST(IpcProtocol, FrameProducesCorrectHeader) {
    flatbuffers::FlatBufferBuilder builder(256);
    auto err = CreateErrorMsgDirect(builder, "r1", "hello");
    auto msg = CreateMessage(builder, MessageType_Error, MessageBody_ErrorMsg, err.Union());
    builder.Finish(msg);

    auto framed = slate::frame(builder);

    // First 4 bytes are network-order payload length
    ASSERT_GT(framed.size(), kFrameHeaderSize);
    uint32_t net_len;
    std::memcpy(&net_len, framed.data(), 4);
    uint32_t payload_len = ntohl(net_len);
    EXPECT_EQ(payload_len, builder.GetSize());
    EXPECT_EQ(framed.size(), kFrameHeaderSize + payload_len);

    // Payload bytes match builder output
    EXPECT_EQ(std::memcmp(framed.data() + kFrameHeaderSize,
                          builder.GetBufferPointer(),
                          builder.GetSize()), 0);
}

TEST(IpcProtocol, LargeMessageFrameUnframe) {
    // Build a message with ~1MB payload
    flatbuffers::FlatBufferBuilder builder(1024 * 1024 + 256);
    std::vector<int8_t> big_data(1024 * 1024, 42);
    auto out = CreateCommandOutputMsgDirect(builder, "big-req", &big_data, false);
    auto msg = CreateMessage(builder, MessageType_CommandOutput,
                             MessageBody_CommandOutputMsg, out.Union());
    builder.Finish(msg);

    auto framed = slate::frame(builder);

    // Verify header
    uint32_t net_len;
    std::memcpy(&net_len, framed.data(), 4);
    uint32_t payload_len = ntohl(net_len);
    EXPECT_EQ(payload_len, builder.GetSize());
    EXPECT_EQ(framed.size(), kFrameHeaderSize + payload_len);
}

TEST(IpcProtocol, WriteReadMessageViaSocketpair) {
    int fds[2];
    ASSERT_EQ(socketpair(AF_UNIX, SOCK_STREAM, 0, fds), 0);

    flatbuffers::FlatBufferBuilder builder(256);
    auto hb = CreateHeartbeatMsg(builder, 12345);
    auto msg = CreateMessage(builder, MessageType_Heartbeat,
                             MessageBody_HeartbeatMsg, hb.Union());
    builder.Finish(msg);

    ASSERT_TRUE(write_message(fds[0], builder.GetBufferPointer(), builder.GetSize()));

    auto received = read_message(fds[1]);
    ASSERT_TRUE(received.has_value());
    EXPECT_EQ(received->size(), builder.GetSize());

    auto* parsed = GetMessage(received->data());
    ASSERT_NE(parsed, nullptr);
    EXPECT_EQ(parsed->type(), MessageType_Heartbeat);
    auto* hb_parsed = parsed->body_as_HeartbeatMsg();
    ASSERT_NE(hb_parsed, nullptr);
    EXPECT_EQ(hb_parsed->timestamp(), 12345);

    ::close(fds[0]);
    ::close(fds[1]);
}

TEST(IpcProtocol, ReadMessageOnClosedFdReturnsNullopt) {
    int fds[2];
    ASSERT_EQ(socketpair(AF_UNIX, SOCK_STREAM, 0, fds), 0);

    // Close the writing end immediately
    ::close(fds[0]);

    auto result = read_message(fds[1]);
    EXPECT_FALSE(result.has_value());

    ::close(fds[1]);
}

TEST(IpcProtocol, ReadMessageOnInvalidFdReturnsNullopt) {
    auto result = read_message(-1);
    EXPECT_FALSE(result.has_value());
}

TEST(IpcProtocol, WriteMessageExceedingMaxSizeFails) {
    // Attempting to write data larger than kMaxMessageSize should fail
    std::vector<uint8_t> huge(kMaxMessageSize + 1, 0);
    int fds[2];
    ASSERT_EQ(socketpair(AF_UNIX, SOCK_STREAM, 0, fds), 0);

    EXPECT_FALSE(write_message(fds[0], huge));

    ::close(fds[0]);
    ::close(fds[1]);
}
