#include <gtest/gtest.h>
#include "../src/slate/shell/bash_coprocess.h"
#include "../src/slate/shell/command_index.h"

using namespace slate;

class CommandIndexTest : public ::testing::Test {
protected:
    void SetUp() override {
        bash_ = std::make_unique<BashCoprocess>();
        index_ = std::make_unique<CommandIndex>();
        index_->build(bash_.get());
    }

    std::unique_ptr<BashCoprocess> bash_;
    std::unique_ptr<CommandIndex> index_;
};

TEST_F(CommandIndexTest, LsIsKnown) {
    EXPECT_TRUE(index_->is_known_command("ls"));
}

TEST_F(CommandIndexTest, UnknownCommandNotKnown) {
    EXPECT_FALSE(index_->is_known_command("xyznotreal123"));
}

TEST_F(CommandIndexTest, LookupLsIsExecutable) {
    auto info = index_->lookup("ls");
    ASSERT_TRUE(info.has_value());
    EXPECT_EQ(info->type, CommandType::Executable);
    EXPECT_FALSE(info->path.empty());
}

TEST_F(CommandIndexTest, CompleteGi) {
    auto results = index_->complete("gi");
    // git should be present if installed
    bool has_git = false;
    for (auto& r : results) {
        if (r == "git") { has_git = true; break; }
    }
    // This is a best-effort test — git is typically available
    EXPECT_TRUE(has_git) << "Expected 'git' in completion results for 'gi'";
}

TEST_F(CommandIndexTest, CompleteEmptyReturnsNonEmpty) {
    auto results = index_->complete("");
    EXPECT_FALSE(results.empty());
}

TEST_F(CommandIndexTest, IndexHasReasonableSize) {
    // Any system should have at least a few hundred commands
    EXPECT_GT(index_->size(), 50u);
}

TEST_F(CommandIndexTest, CdIsBuiltin) {
    auto info = index_->lookup("cd");
    ASSERT_TRUE(info.has_value());
    EXPECT_EQ(info->type, CommandType::Builtin);
}
