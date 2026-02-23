enable_testing()
include(GoogleTest)

# Shell source files needed by shell tests
set(SHELL_SOURCES
    ${CMAKE_SOURCE_DIR}/src/slate/shell/bash_coprocess.cpp
    ${CMAKE_SOURCE_DIR}/src/slate/shell/command_index.cpp
    ${CMAKE_SOURCE_DIR}/src/slate/shell/interactive.cpp
)

# IPC tests (no shell dependency)
set(IPC_TEST_SOURCES
    test_ipc_protocol.cpp
    test_ipc_roundtrip.cpp
)

foreach(test_source ${IPC_TEST_SOURCES})
    get_filename_component(test_name ${test_source} NAME_WE)
    add_executable(${test_name} ${test_source})
    target_link_libraries(${test_name} PRIVATE
        GTest::gtest_main
        slate_common
    )
    gtest_discover_tests(${test_name})
endforeach()

# Shell tests (need shell sources compiled in)
set(SHELL_TEST_SOURCES
    test_bash_coprocess.cpp
    test_command_index.cpp
)

foreach(test_source ${SHELL_TEST_SOURCES})
    get_filename_component(test_name ${test_source} NAME_WE)
    add_executable(${test_name} ${test_source} ${SHELL_SOURCES})
    target_include_directories(${test_name} PRIVATE
        ${CMAKE_SOURCE_DIR}/src/slate/shell
    )
    target_link_libraries(${test_name} PRIVATE
        GTest::gtest_main
        slate_common
    )
    gtest_discover_tests(${test_name})
endforeach()
