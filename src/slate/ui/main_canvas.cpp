#include "main_canvas.h"

#include <algorithm>
#include <sstream>

#include <ftxui/component/event.hpp>
#include <ftxui/dom/elements.hpp>

namespace slate {

using namespace ftxui;

MainCanvas::MainCanvas() {
    auto renderer = Renderer([this] {
        std::lock_guard<std::mutex> lock(mutex_);

        Elements visible;

        if (lines_.empty()) {
            visible.push_back(text(""));
        } else {
            // Render all lines; use focusPosition to control scroll
            for (size_t i = 0; i < lines_.size(); ++i) {
                auto line_element = text(lines_[i]);
                if (static_cast<int>(i) == scroll_offset_) {
                    line_element = line_element | focus;
                }
                visible.push_back(std::move(line_element));
            }
        }

        auto content = vbox(std::move(visible)) | yframe | flex;

        // If scroll locked and there are new lines, show indicator
        if (scroll_locked_ && new_lines_since_lock_ > 0) {
            auto indicator = text(
                " ↓ " + std::to_string(new_lines_since_lock_) + " new lines"
            ) | color(Color::Yellow) | bold;
            return vbox({
                content,
                indicator,
            });
        }

        return content;
    });

    component_ = CatchEvent(renderer, [this](Event event) {
        std::lock_guard<std::mutex> lock(mutex_);

        if (lines_.empty()) {
            return false;
        }

        int max_offset = std::max(0, static_cast<int>(lines_.size()) - 1);

        if (event == Event::PageUp) {
            scroll_offset_ = std::max(0, scroll_offset_ - kScrollPageSize);
            scroll_locked_ = true;
            return true;
        }

        if (event == Event::PageDown) {
            scroll_offset_ = std::min(max_offset,
                                      scroll_offset_ + kScrollPageSize);
            if (scroll_offset_ >= max_offset) {
                scroll_locked_ = false;
                new_lines_since_lock_ = 0;
            }
            return true;
        }

        if (event == Event::End) {
            scroll_offset_ = max_offset;
            scroll_locked_ = false;
            new_lines_since_lock_ = 0;
            return true;
        }

        if (event == Event::Home) {
            scroll_offset_ = 0;
            scroll_locked_ = true;
            return true;
        }

        // Mouse scroll handling
        if (event.is_mouse()) {
            auto& mouse = event.mouse();
            if (mouse.button == Mouse::WheelUp) {
                scroll_offset_ = std::max(0, scroll_offset_ - 3);
                scroll_locked_ = true;
                return true;
            }
            if (mouse.button == Mouse::WheelDown) {
                scroll_offset_ = std::min(max_offset, scroll_offset_ + 3);
                if (scroll_offset_ >= max_offset) {
                    scroll_locked_ = false;
                    new_lines_since_lock_ = 0;
                }
                return true;
            }
        }

        return false;
    });
}

void MainCanvas::append(const std::string& text) {
    std::lock_guard<std::mutex> lock(mutex_);

    std::istringstream stream(text);
    std::string line;
    int added = 0;

    while (std::getline(stream, line)) {
        lines_.push_back(std::move(line));
        ++added;
    }

    // Handle trailing newline producing an empty final line
    if (!text.empty() && text.back() == '\n') {
        // std::getline already handled this correctly
    }

    if (scroll_locked_) {
        new_lines_since_lock_ += added;
    } else {
        // Auto-scroll to bottom
        scroll_offset_ = std::max(0, static_cast<int>(lines_.size()) - 1);
    }
}

} // namespace slate
