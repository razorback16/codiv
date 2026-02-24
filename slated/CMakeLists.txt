cmake_minimum_required(VERSION 3.20)
project(slated VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

include(FetchContent)

# FlatBuffers
FetchContent_Declare(
  flatbuffers
  GIT_REPOSITORY https://github.com/google/flatbuffers.git
  GIT_TAG        v24.3.25
)
set(FLATBUFFERS_BUILD_TESTS OFF CACHE BOOL "" FORCE)
set(FLATBUFFERS_BUILD_FLATC ON CACHE BOOL "" FORCE)
FetchContent_MakeAvailable(flatbuffers)

# GoogleTest
FetchContent_Declare(
  googletest
  GIT_REPOSITORY https://github.com/google/googletest.git
  GIT_TAG        v1.14.0
)
set(gtest_force_shared_crt ON CACHE BOOL "" FORCE)
FetchContent_MakeAvailable(googletest)

# Generate FlatBuffers headers from schema
set(FBS_SCHEMA "${CMAKE_SOURCE_DIR}/../schemas/ipc.fbs")
set(FBS_GENERATED_DIR "${CMAKE_BINARY_DIR}/generated")
file(MAKE_DIRECTORY ${FBS_GENERATED_DIR})

add_custom_command(
  OUTPUT ${FBS_GENERATED_DIR}/ipc_generated.h
  COMMAND flatc --cpp -o ${FBS_GENERATED_DIR} ${FBS_SCHEMA}
  DEPENDS ${FBS_SCHEMA} flatc
  COMMENT "Generating FlatBuffers C++ headers from ipc.fbs"
)
add_custom_target(fbs_headers DEPENDS ${FBS_GENERATED_DIR}/ipc_generated.h)

# slated daemon executable
add_executable(slated
  src/main.cpp
  src/daemon.cpp
  src/worker_process.cpp
  src/ipc_protocol.cpp
  src/ipc/socket_server.cpp
  src/ipc/event_loop.cpp
)
target_include_directories(slated PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/src
  ${FBS_GENERATED_DIR}
)
target_link_libraries(slated PRIVATE flatbuffers)
add_dependencies(slated fbs_headers)

enable_testing()
