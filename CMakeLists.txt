cmake_minimum_required(VERSION 3.20)
project(slate-agent VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

enable_testing()

include(cmake/dependencies.cmake)

# Generate FlatBuffers header from schema
add_custom_command(
    OUTPUT ${CMAKE_SOURCE_DIR}/src/generated/ipc_generated.h
    COMMAND flatc --cpp -o ${CMAKE_SOURCE_DIR}/src/generated ${CMAKE_SOURCE_DIR}/schemas/ipc.fbs
    DEPENDS ${CMAKE_SOURCE_DIR}/schemas/ipc.fbs flatc
    COMMENT "Generating FlatBuffers IPC header"
)
add_custom_target(generate_ipc DEPENDS ${CMAKE_SOURCE_DIR}/src/generated/ipc_generated.h)

add_subdirectory(src/common)
add_subdirectory(src/slate)
add_subdirectory(src/slated)
add_subdirectory(tests)
