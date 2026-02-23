#include "stream_view.h"
#include "terminal_ui.h"

#include <algorithm>
#include <sstream>

#include <ftxui/component/event.hpp>

namespace slate {

using namespace ftxui;

StreamView::StreamView(UIDelegate* delegate)
    : delegate_(delegate) {

    InputOption options;
    options.content = &input_content_;
    options.placeholder = "Type a command...";
    options.multiline = false;

    auto input = Input(options);

    auto renderer = Renderer(input, [this, input] {
        std::lock_guard<std::mutex> lock(mutex_);

        int viewport_height = viewport_box_.y_max - viewport_box_.y_min + 1;
        if (viewport_height <= 0) viewport_height = 24;  // initial guess

        // Flatten all entries into styled lines
        auto all_lines = render_lines();
        int total_lines = static_cast<int>(all_lines.size());

        // Reserve space for input prompt (2 lines: completions + prompt, or just prompt)
        int input_lines = (show_completions_ && !completions_.empty())
                              ? static_cast<int>(completions_.size()) + 1
                              : 1;
        int content_height = viewport_height - input_lines;
        if (content_height < 1) content_height = 1;

        // Calculate visible range (bottom-anchored)
        int visible_start;
        if (total_lines <= content_height) {
            visible_start = 0;
        } else {
            visible_start = total_lines - content_height - scroll_offset_;
            visible_start = std::max(0, visible_start);
        }
        int visible_end = std::min(total_lines, visible_start + content_height);

        // Build visible content
        Elements visible;

        // Pad with empty lines if content is shorter than viewport (bottom-anchored)
        if (total_lines < content_height) {
            int pad = content_height - total_lines;
            for (int i = 0; i < pad; ++i) {
                visible.push_back(text(""));
            }
        }

        for (int i = visible_start; i < visible_end; ++i) {
            visible.push_back(std::move(all_lines[i]));
        }

        auto content = vbox(std::move(visible));

        // Build input area
        Elements input_area;

        // Show completions above input if active
        if (show_completions_ && !completions_.empty()) {
            Elements comp_items;
            for (const auto& c : completions_) {
                comp_items.push_back(text("  " + c) | color(Color::GrayLight));
            }
            input_area.push_back(vbox(std::move(comp_items)));
        }

        // Input line with prompt
        input_area.push_back(
            hbox({
                text("$ ") | bold | color(Color::Green),
                input->Render() | flex,
            })
        );

        // Compose final view
        Elements final_view;
        final_view.push_back(content | flex);

        // Scroll-lock indicator
        if (!auto_scroll_ && new_lines_since_detach_ > 0) {
            final_view.push_back(
                text(" ↓ " + std::to_string(new_lines_since_detach_) +
                     " new lines (End to jump)")
                | color(Color::Yellow) | bold
            );
        }

        // Input area at bottom
        for (auto& el : input_area) {
            final_view.push_back(std::move(el));
        }

        return vbox(std::move(final_view)) | reflect(viewport_box_);
    });

    component_ = CatchEvent(renderer, [this, input](Event event) {
        // Enter key: submit command
        if (event == Event::Return) {
            if (!input_content_.empty()) {
                std::string cmd = input_content_;

                // Echo command into scrollback
                echo_command(cmd);

                // Add to history
                history_.push_back(cmd);
                history_index_ = -1;

                // Clear input
                input_content_.clear();
                show_completions_ = false;
                completions_.clear();

                // Submit to delegate
                if (delegate_) {
                    delegate_->on_command_submitted(cmd);
                }
            }
            return true;
        }

        // Up arrow: navigate history backward
        if (event == Event::ArrowUp) {
            if (!history_.empty()) {
                if (history_index_ == -1) {
                    saved_input_ = input_content_;
                    history_index_ = static_cast<int>(history_.size()) - 1;
                } else if (history_index_ > 0) {
                    --history_index_;
                }
                input_content_ = history_[history_index_];
            }
            return true;
        }

        // Down arrow: navigate history forward
        if (event == Event::ArrowDown) {
            if (history_index_ != -1) {
                if (history_index_ < static_cast<int>(history_.size()) - 1) {
                    ++history_index_;
                    input_content_ = history_[history_index_];
                } else {
                    history_index_ = -1;
                    input_content_ = saved_input_;
                }
            }
            return true;
        }

        // Tab: autocomplete
        if (event == Event::Tab) {
            if (delegate_) {
                auto results = delegate_->on_autocomplete(input_content_);
                if (results.size() == 1) {
                    input_content_ = results[0];
                    show_completions_ = false;
                    completions_.clear();
                } else if (results.size() > 1) {
                    completions_ = std::move(results);
                    show_completions_ = true;
                } else {
                    show_completions_ = false;
                    completions_.clear();
                }
            }
            return true;
        }

        // Any typed character hides completions
        if (event.is_character()) {
            show_completions_ = false;
            completions_.clear();
            // Don't return true — let the input component handle it
        }

        // --- Scroll handling ---

        // PageUp: scroll up
        if (event == Event::PageUp) {
            std::lock_guard<std::mutex> lock(mutex_);
            auto all_lines = render_lines();
            int max_offset = std::max(0, static_cast<int>(all_lines.size()) - 1);
            scroll_offset_ = std::min(max_offset,
                                       scroll_offset_ + kScrollPageSize);
            auto_scroll_ = false;
            return true;
        }

        // PageDown: scroll down
        if (event == Event::PageDown) {
            std::lock_guard<std::mutex> lock(mutex_);
            scroll_offset_ = std::max(0, scroll_offset_ - kScrollPageSize);
            if (scroll_offset_ == 0) {
                auto_scroll_ = true;
                new_lines_since_detach_ = 0;
            }
            return true;
        }

        // End: jump to bottom
        if (event == Event::End) {
            std::lock_guard<std::mutex> lock(mutex_);
            scroll_offset_ = 0;
            auto_scroll_ = true;
            new_lines_since_detach_ = 0;
            return true;
        }

        // Home: jump to top
        if (event == Event::Home) {
            std::lock_guard<std::mutex> lock(mutex_);
            auto all_lines = render_lines();
            scroll_offset_ = std::max(0, static_cast<int>(all_lines.size()) - 1);
            auto_scroll_ = false;
            return true;
        }

        // Mouse wheel
        if (event.is_mouse()) {
            auto& mouse = event.mouse();
            if (mouse.button == Mouse::WheelUp) {
                std::lock_guard<std::mutex> lock(mutex_);
                auto all_lines = render_lines();
                int max_offset = std::max(0, static_cast<int>(all_lines.size()) - 1);
                scroll_offset_ = std::min(max_offset,
                                           scroll_offset_ + kMouseScrollLines);
                auto_scroll_ = false;
                return true;
            }
            if (mouse.button == Mouse::WheelDown) {
                std::lock_guard<std::mutex> lock(mutex_);
                scroll_offset_ = std::max(0, scroll_offset_ - kMouseScrollLines);
                if (scroll_offset_ == 0) {
                    auto_scroll_ = true;
                    new_lines_since_detach_ = 0;
                }
                return true;
            }
        }

        return false;
    });
}

void StreamView::echo_command(const std::string& cmd) {
    std::lock_guard<std::mutex> lock(mutex_);
    push_entry({StreamEntry::Type::CommandEcho, "$ " + cmd});
}

void StreamView::append_output(const std::string& text) {
    std::lock_guard<std::mutex> lock(mutex_);
    std::istringstream stream(text);
    std::string line;
    while (std::getline(stream, line)) {
        push_entry({StreamEntry::Type::Output, std::move(line)});
    }
}

void StreamView::append_system(const std::string& text) {
    std::lock_guard<std::mutex> lock(mutex_);
    std::istringstream stream(text);
    std::string line;
    while (std::getline(stream, line)) {
        push_entry({StreamEntry::Type::SystemMessage, std::move(line)});
    }
}

void StreamView::append_separator() {
    std::lock_guard<std::mutex> lock(mutex_);
    push_entry({StreamEntry::Type::Separator, ""});
}

void StreamView::push_entry(StreamEntry entry) {
    // Caller must hold mutex_
    entries_.push_back(std::move(entry));

    // Trim if over max
    if (entries_.size() > kMaxEntries) {
        entries_.erase(entries_.begin(),
                       entries_.begin() + static_cast<long>(entries_.size() - kMaxEntries));
    }

    if (auto_scroll_) {
        scroll_offset_ = 0;
    } else {
        ++new_lines_since_detach_;
    }
}

std::vector<Element> StreamView::render_lines() const {
    // Caller must hold mutex_
    std::vector<Element> lines;
    lines.reserve(entries_.size());

    for (const auto& entry : entries_) {
        lines.push_back(style_entry(entry));
    }

    return lines;
}

Element StreamView::style_entry(const StreamEntry& entry) const {
    switch (entry.type) {
        case StreamEntry::Type::CommandEcho:
            return text(entry.text) | bold | color(Color::Green);
        case StreamEntry::Type::Output:
            return text(entry.text);
        case StreamEntry::Type::SystemMessage:
            return text(entry.text) | dim | color(Color::Yellow);
        case StreamEntry::Type::AgentBlock:
            return text(entry.text) | color(Color::Cyan);
        case StreamEntry::Type::Separator:
            return separator();
    }
    return text(entry.text);  // fallback
}

} // namespace slate
