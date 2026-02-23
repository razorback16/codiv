#include <gtest/gtest.h>
#include "../src/slate/shell/bash_coprocess.h"

using namespace slate;

TEST(BashCoprocess, IsAliveAfterConstruction) {
    BashCoprocess bash;
    EXPECT_TRUE(bash.is_alive());
}

TEST(BashCoprocess, EchoHello) {
    BashCoprocess bash;
    auto result = bash.execute("echo hello");
    EXPECT_EQ(result.exit_code, 0);
    EXPECT_NE(result.output.find("hello"), std::string::npos);
}

TEST(BashCoprocess, StatePreserved) {
    BashCoprocess bash;
    bash.execute("export FOO=bar");
    auto result = bash.execute("echo $FOO");
    EXPECT_EQ(result.exit_code, 0);
    EXPECT_NE(result.output.find("bar"), std::string::npos);
}

TEST(BashCoprocess, FalseExitCode) {
    BashCoprocess bash;
    auto result = bash.execute("false");
    EXPECT_EQ(result.exit_code, 1);
}

TEST(BashCoprocess, CustomExitCode) {
    BashCoprocess bash;
    auto result = bash.execute("bash -c 'exit 42'");
    EXPECT_EQ(result.exit_code, 42);
}

TEST(BashCoprocess, CaptureCwd) {
    BashCoprocess bash;
    auto cwd = bash.capture_cwd();
    EXPECT_FALSE(cwd.empty());
    EXPECT_EQ(cwd.front(), '/');  // Absolute path
}

TEST(BashCoprocess, CaptureEnv) {
    BashCoprocess bash;
    bash.execute("export SLATE_TEST_VAR=hello123");
    auto env = bash.capture_env();
    bool found = false;
    for (auto& [key, val] : env) {
        if (key == "SLATE_TEST_VAR" && val == "hello123") {
            found = true;
            break;
        }
    }
    EXPECT_TRUE(found);
}
