add_library(slate_common
    ipc_protocol.cpp
)

add_dependencies(slate_common generate_ipc)

target_include_directories(slate_common PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${CMAKE_SOURCE_DIR}/src/generated
)

target_link_libraries(slate_common PUBLIC
    flatbuffers
)
