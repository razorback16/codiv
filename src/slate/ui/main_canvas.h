#pragma once

#include <ftxui/component/component.hpp>
#include <mutex>
#include <string>
#include <vector>

namespace slate {

class MainCanvas {
public:
    MainCanvas();
    ~MainCanvas() = default;

    /// Thread-safe: append text (may contain newlines) to the output buffer.
    void append(const std::string& text);

    /// Return the FTXUI component for layout composition.
    ftxui::Component component() const { return component_; }

private:
    mutable std::mutex mutex_;
    std::vector<std::string> lines_;
    int scroll_offset_ = 0;      // Index of the first visible line
    bool scroll_locked_ = false;  // True when user has scrolled up
    int new_lines_since_lock_ = 0;

    static constexpr int kScrollPageSize = 20;

    ftxui::Component component_;
};

} // namespace slate
