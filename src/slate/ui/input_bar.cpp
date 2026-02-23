#include "input_bar.h"
#include "terminal_ui.h"

#include <ftxui/component/event.hpp>
#include <ftxui/dom/elements.hpp>

namespace slate {

using namespace ftxui;

InputBar::InputBar(UIDelegate* delegate)
    : delegate_(delegate) {

    InputOption options;
    options.content = &input_content_;
    options.placeholder = "Type a command...";
    options.multiline = false;

    auto input = Input(options);

    auto renderer = Renderer(input, [this, input] {
        Elements rows;

        // Show completions above input if active
        if (show_completions_ && !completions_.empty()) {
            Elements comp_items;
            for (const auto& c : completions_) {
                comp_items.push_back(text("  " + c) | color(Color::GrayLight));
            }
            rows.push_back(vbox(std::move(comp_items)));
        }

        // Input line with prompt
        rows.push_back(
            hbox({
                text("$ ") | bold | color(Color::Green),
                input->Render() | flex,
            })
        );

        return vbox(std::move(rows));
    });

    component_ = CatchEvent(renderer, [this, input](Event event) {
        // Enter key: submit command
        if (event == Event::Return) {
            if (!input_content_.empty()) {
                // Add to history
                history_.push_back(input_content_);
                history_index_ = -1;

                // Submit to delegate
                if (delegate_) {
                    delegate_->on_command_submitted(input_content_);
                }

                // Clear input
                input_content_.clear();
                show_completions_ = false;
                completions_.clear();
            }
            return true;
        }

        // Up arrow: navigate history backward
        if (event == Event::ArrowUp) {
            if (!history_.empty()) {
                if (history_index_ == -1) {
                    // Save current input and start browsing
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
                    // Restore saved input
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
                    // Single match: auto-complete
                    input_content_ = results[0];
                    show_completions_ = false;
                    completions_.clear();
                } else if (results.size() > 1) {
                    // Multiple matches: show completions
                    completions_ = std::move(results);
                    show_completions_ = true;
                } else {
                    show_completions_ = false;
                    completions_.clear();
                }
            }
            return true;
        }

        // Any other character typed hides completions
        if (event.is_character()) {
            show_completions_ = false;
            completions_.clear();
        }

        return false;
    });
}

void InputBar::clear() {
    input_content_.clear();
}

} // namespace slate
