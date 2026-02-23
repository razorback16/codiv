#include "terminal_ui.h"
#include "status_bar.h"
#include "stream_view.h"

#include <ftxui/component/component.hpp>
#include <ftxui/dom/elements.hpp>

namespace slate {

using namespace ftxui;

TerminalUI::TerminalUI(UIDelegate* delegate)
    : delegate_(delegate),
      screen_(ScreenInteractive::Fullscreen()),
      status_bar_(std::make_unique<StatusBar>()),
      stream_view_(std::make_unique<StreamView>(delegate)) {
    build_layout();
}

TerminalUI::~TerminalUI() = default;

void TerminalUI::build_layout() {
    // Two-zone layout: StatusBar | separator | StreamView
    auto container = Container::Vertical({
        status_bar_->component(),
        stream_view_->component(),
    });

    // Default focus to StreamView (index 1)
    container->SetActiveChild(stream_view_->component());

    layout_ = Renderer(container, [this, container] {
        return vbox({
            status_bar_->component()->Render(),
            separator(),
            stream_view_->component()->Render() | flex,
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
    screen_.Post([this, text] {
        stream_view_->append_output(text);
    });
    screen_.PostEvent(Event::Custom);
}

void TerminalUI::echo_command(const std::string& cmd) {
    screen_.Post([this, cmd] {
        stream_view_->echo_command(cmd);
    });
    screen_.PostEvent(Event::Custom);
}

void TerminalUI::append_system(const std::string& text) {
    screen_.Post([this, text] {
        stream_view_->append_system(text);
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
