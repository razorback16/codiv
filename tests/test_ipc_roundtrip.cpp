#include <gtest/gtest.h>
#include <sys/socket.h>
#include <unistd.h>
#include <arpa/inet.h>
#include "ipc_protocol.h"
#include "types.h"
#include "ipc_generated.h"

using namespace slate;
using namespace slate::ipc;

class IpcRoundtripTest : public ::testing::Test {
protected:
    int fds_[2] = {-1, -1};

    void SetUp() override {
        ASSERT_EQ(socketpair(AF_UNIX, SOCK_STREAM, 0, fds_), 0);
    }

    void TearDown() override {
        if (fds_[0] >= 0) ::close(fds_[0]);
        if (fds_[1] >= 0) ::close(fds_[1]);
    }

    // Helper: build, frame, write, read, return parsed Message
    // write_message already adds the length prefix, so pass raw flatbuffer data
    void send_and_receive(flatbuffers::FlatBufferBuilder& builder,
                          std::vector<uint8_t>& out_payload) {
        // write_message adds the 4-byte length prefix itself
        auto* buf = builder.GetBufferPointer();
        auto size = builder.GetSize();
        ASSERT_TRUE(slate::write_message(fds_[0], buf, size));

        auto received = slate::read_message(fds_[1]);
        ASSERT_TRUE(received.has_value());
        out_payload = std::move(*received);
    }
};

TEST_F(IpcRoundtripTest, ExecuteCommand) {
    flatbuffers::FlatBufferBuilder builder(256);
    auto cmd = CreateExecuteCommandMsgDirect(builder, "ls -la", "/home/user", "req-001");
    auto msg = CreateMessage(builder, MessageType_ExecuteCommand, MessageBody_ExecuteCommandMsg, cmd.Union());
    builder.Finish(msg);

    std::vector<uint8_t> payload;
    send_and_receive(builder, payload);

    auto* parsed = GetMessage(payload.data());
    ASSERT_NE(parsed, nullptr);
    EXPECT_EQ(parsed->type(), MessageType_ExecuteCommand);
    EXPECT_EQ(parsed->body_type(), MessageBody_ExecuteCommandMsg);

    auto* exec = parsed->body_as_ExecuteCommandMsg();
    ASSERT_NE(exec, nullptr);
    EXPECT_STREQ(exec->command()->c_str(), "ls -la");
    EXPECT_STREQ(exec->cwd()->c_str(), "/home/user");
    EXPECT_STREQ(exec->request_id()->c_str(), "req-001");
}

TEST_F(IpcRoundtripTest, CommandOutput) {
    flatbuffers::FlatBufferBuilder builder(256);
    std::vector<int8_t> data = {'h', 'e', 'l', 'l', 'o'};
    auto out = CreateCommandOutputMsgDirect(builder, "req-002", &data, true);
    auto msg = CreateMessage(builder, MessageType_CommandOutput, MessageBody_CommandOutputMsg, out.Union());
    builder.Finish(msg);

    std::vector<uint8_t> payload;
    send_and_receive(builder, payload);

    auto* parsed = GetMessage(payload.data());
    ASSERT_NE(parsed, nullptr);
    EXPECT_EQ(parsed->type(), MessageType_CommandOutput);

    auto* output = parsed->body_as_CommandOutputMsg();
    ASSERT_NE(output, nullptr);
    EXPECT_STREQ(output->request_id()->c_str(), "req-002");
    EXPECT_TRUE(output->is_stderr());
    ASSERT_EQ(output->data()->size(), 5u);
    EXPECT_EQ((*output->data())[0], 'h');
    EXPECT_EQ((*output->data())[4], 'o');
}

TEST_F(IpcRoundtripTest, CommandComplete) {
    flatbuffers::FlatBufferBuilder builder(256);
    auto comp = CreateCommandCompleteMsgDirect(builder, "req-003", 42);
    auto msg = CreateMessage(builder, MessageType_CommandComplete, MessageBody_CommandCompleteMsg, comp.Union());
    builder.Finish(msg);

    std::vector<uint8_t> payload;
    send_and_receive(builder, payload);

    auto* parsed = GetMessage(payload.data());
    ASSERT_NE(parsed, nullptr);
    EXPECT_EQ(parsed->type(), MessageType_CommandComplete);

    auto* complete = parsed->body_as_CommandCompleteMsg();
    ASSERT_NE(complete, nullptr);
    EXPECT_STREQ(complete->request_id()->c_str(), "req-003");
    EXPECT_EQ(complete->exit_code(), 42);
}

TEST_F(IpcRoundtripTest, ErrorMsg) {
    flatbuffers::FlatBufferBuilder builder(256);
    auto err = CreateErrorMsgDirect(builder, "req-004", "something went wrong");
    auto msg = CreateMessage(builder, MessageType_Error, MessageBody_ErrorMsg, err.Union());
    builder.Finish(msg);

    std::vector<uint8_t> payload;
    send_and_receive(builder, payload);

    auto* parsed = GetMessage(payload.data());
    ASSERT_NE(parsed, nullptr);
    EXPECT_EQ(parsed->type(), MessageType_Error);

    auto* error = parsed->body_as_ErrorMsg();
    ASSERT_NE(error, nullptr);
    EXPECT_STREQ(error->request_id()->c_str(), "req-004");
    EXPECT_STREQ(error->message()->c_str(), "something went wrong");
}

TEST_F(IpcRoundtripTest, FrameHelper) {
    // Test that frame() produces correct length-prefixed output
    flatbuffers::FlatBufferBuilder builder(256);
    auto err = CreateErrorMsgDirect(builder, "req-005", "test frame");
    auto msg = CreateMessage(builder, MessageType_Error, MessageBody_ErrorMsg, err.Union());
    builder.Finish(msg);

    auto framed = slate::frame(builder);
    ASSERT_GT(framed.size(), kFrameHeaderSize);

    // Verify the length header matches the payload size
    uint32_t net_len;
    std::memcpy(&net_len, framed.data(), 4);
    uint32_t payload_len = ntohl(net_len);
    EXPECT_EQ(payload_len, builder.GetSize());
    EXPECT_EQ(framed.size(), kFrameHeaderSize + payload_len);

    // Write the framed data directly (raw write, no extra framing)
    auto written = ::write(fds_[0], framed.data(), framed.size());
    ASSERT_EQ(static_cast<size_t>(written), framed.size());

    // read_message should parse the length prefix and return just the payload
    auto received = slate::read_message(fds_[1]);
    ASSERT_TRUE(received.has_value());

    auto* parsed = GetMessage(received->data());
    ASSERT_NE(parsed, nullptr);
    auto* error = parsed->body_as_ErrorMsg();
    ASSERT_NE(error, nullptr);
    EXPECT_STREQ(error->request_id()->c_str(), "req-005");
}
