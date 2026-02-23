#include "terminal_ui.h"
#include "status_bar.h"
#include "main_canvas.h"
#include "input_bar.h"

#include <ftxui/component/component.hpp>
#include <ftxui/dom/elements.hpp>

namespace slate {

using namespace ftxui;

TerminalUI::TerminalUI(UIDelegate* delegate)
    : delegate_(delegate),
      screen_(ScreenInteractive::Fullscreen()),
      status_bar_(std::make_unique<StatusBar>()),
      main_canvas_(std::make_unique<MainCanvas>()),
      input_bar_(std::make_unique<InputBar>(delegate)) {
    build_layout();
}

TerminalUI::~TerminalUI() = default;

void TerminalUI::build_layout() {
    // Compose layout: StatusBar | sep | MainCanvas | sep | InputBar
    auto container = Container::Vertical({
        status_bar_->component(),
        main_canvas_->component(),
        input_bar_->component(),
    });

    layout_ = Renderer(container, [this, container] {
        return vbox({
            status_bar_->component()->Render(),
            separator(),
            main_canvas_->component()->Render() | flex,
            separator(),
            input_bar_->component()->Render(),
        });
    });

    // Catch Ctrl+C / Ctrl+D to exit
    layout_ = CatchEvent(layout_, [this](Event event) {
        if (event == Event::Character('\x03') ||  // Ctrl+C
            event == Event::Character('\x04')) {   // Ctrl+D
            screen_.Exit();
            return true;
        }
        return false;
    });
}

void TerminalUI::run() {
    screen_.Loop(layout_);
}

void TerminalUI::append_output(const std::string& text) {
    // Thread-safe: post the update to the UI thread
    screen_.Post([this, text] {
        main_canvas_->append(text);
    });
    screen_.PostEvent(Event::Custom);
}

void TerminalUI::set_status(const std::string& cwd,
                             const std::string& git_branch,
                             bool daemon_connected) {
    screen_.Post([this, cwd, git_branch, daemon_connected] {
        status_bar_->set_cwd(cwd);
        status_bar_->set_git_branch(git_branch);
        status_bar_->set_daemon_status(daemon_connected);
    });
    screen_.PostEvent(Event::Custom);
}

void TerminalUI::suspend() {
    screen_.Exit();
}

void TerminalUI::resume() {
    screen_.Loop(layout_);
}

void TerminalUI::stop() {
    screen_.Exit();
}

} // namespace slate
