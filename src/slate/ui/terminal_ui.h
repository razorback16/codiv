#pragma once

#include <memory>
#include <string>
#include <vector>

#include <ftxui/component/screen_interactive.hpp>

namespace slate {

class StatusBar;
class MainCanvas;
class InputBar;

/// Interface for receiving UI events. Nullable for standalone testing.
class UIDelegate {
public:
    virtual ~UIDelegate() = default;
    virtual void on_command_submitted(const std::string& input) = 0;
    virtual std::vector<std::string> on_autocomplete(const std::string& prefix) = 0;
};

/// Full-screen terminal UI built on FTXUI.
/// Layout (top to bottom):
///   StatusBar  (1 line)
///   separator
///   MainCanvas (flex, remaining space)
///   separator
///   InputBar   (1-2 lines at bottom)
class TerminalUI {
public:
    /// @param delegate  Optional callback receiver (may be nullptr).
    explicit TerminalUI(UIDelegate* delegate = nullptr);
    ~TerminalUI();

    /// Enter the FTXUI main loop (blocks until stop/suspend).
    void run();

    /// Thread-safe: append output text to the main canvas.
    void append_output(const std::string& text);

    /// Thread-safe: update the status bar fields.
    void set_status(const std::string& cwd,
                    const std::string& git_branch,
                    bool daemon_connected);

    /// Exit the loop temporarily (for interactive passthrough).
    void suspend();

    /// Re-enter the FTXUI loop after suspend.
    void resume();

    /// Exit the UI loop permanently.
    void stop();

private:
    UIDelegate* delegate_;

    ftxui::ScreenInteractive screen_;

    std::unique_ptr<StatusBar> status_bar_;
    std::unique_ptr<MainCanvas> main_canvas_;
    std::unique_ptr<InputBar> input_bar_;

    ftxui::Component layout_;

    void build_layout();
};

} // namespace slate
