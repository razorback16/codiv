#pragma once

#include <ftxui/component/component.hpp>
#include <functional>
#include <string>
#include <vector>

namespace slate {

class UIDelegate;

class InputBar {
public:
    explicit InputBar(UIDelegate* delegate);
    ~InputBar() = default;

    ftxui::Component component() const { return component_; }

    /// Clear the input field.
    void clear();

private:
    UIDelegate* delegate_;

    std::string input_content_;
    std::vector<std::string> history_;
    int history_index_ = -1;  // -1 means not browsing history
    std::string saved_input_;  // saved input when browsing history

    // Autocomplete state
    std::vector<std::string> completions_;
    bool show_completions_ = false;

    ftxui::Component component_;
};

} // namespace slate
