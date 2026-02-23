#pragma once

#include <ftxui/component/component.hpp>
#include <ftxui/dom/elements.hpp>
#include <mutex>
#include <string>
#include <vector>

namespace slate {

class UIDelegate;

/// A single entry in the terminal scrollback buffer.
struct StreamEntry {
    enum class Type {
        CommandEcho,    // "$ command" — user's echoed command
        Output,         // Normal command output
        SystemMessage,  // Dim status messages (exit codes, errors)
        AgentBlock,     // Phase 2 placeholder
        Separator,      // Visual separator line
    };

    Type type;
    std::string text;
};

/// Unified terminal stream: scrollback + inline input prompt.
///
/// Replaces the old MainCanvas + InputBar with a single component
/// that renders a continuous terminal stream with the input prompt
/// at the bottom, like a real terminal.
class StreamView {
public:
    explicit StreamView(UIDelegate* delegate);
    ~StreamView() = default;

    /// Echo a command into the scrollback (green bold "$ cmd").
    void echo_command(const std::string& cmd);

    /// Append normal output text (may contain newlines).
    void append_output(const std::string& text);

    /// Append a system/status message (dim yellow).
    void append_system(const std::string& text);

    /// Append a visual separator line.
    void append_separator();

    /// Return the FTXUI component for layout composition.
    ftxui::Component component() const { return component_; }

private:
    UIDelegate* delegate_;

    mutable std::mutex mutex_;
    std::vector<StreamEntry> entries_;
    static constexpr size_t kMaxEntries = 10000;

    // Scroll state (offset measured from bottom: 0 = at bottom)
    int scroll_offset_ = 0;
    bool auto_scroll_ = true;
    int new_lines_since_detach_ = 0;

    static constexpr int kScrollPageSize = 20;
    static constexpr int kMouseScrollLines = 3;

    // Input state (absorbed from InputBar)
    std::string input_content_;
    std::vector<std::string> history_;
    int history_index_ = -1;
    std::string saved_input_;

    // Autocomplete
    std::vector<std::string> completions_;
    bool show_completions_ = false;

    // Viewport size captured via reflect()
    ftxui::Box viewport_box_;

    ftxui::Component component_;

    /// Flatten entries into styled FTXUI Elements (one per line).
    std::vector<ftxui::Element> render_lines() const;

    /// Style a single entry based on its type.
    ftxui::Element style_entry(const StreamEntry& entry) const;

    /// Add an entry and manage buffer size.
    void push_entry(StreamEntry entry);
};

} // namespace slate
