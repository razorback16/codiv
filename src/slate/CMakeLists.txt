add_executable(slate
    main.cpp
    app.cpp
    ui/terminal_ui.cpp
    ui/status_bar.cpp
    ui/stream_view.cpp
    shell/bash_coprocess.cpp
    shell/command_index.cpp
    shell/interactive.cpp
    ipc/daemon_client.cpp
)

target_include_directories(slate PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}
)

target_link_libraries(slate PRIVATE
    slate_common
    ftxui::screen
    ftxui::dom
    ftxui::component
)
