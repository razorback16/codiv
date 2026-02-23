add_executable(slated
    main.cpp
    daemon.cpp
    session.cpp
    ipc/socket_server.cpp
)

target_include_directories(slated PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}
)

target_link_libraries(slated PRIVATE
    slate_common
)
