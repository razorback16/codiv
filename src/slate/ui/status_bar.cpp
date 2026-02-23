#include "status_bar.h"

#include <ftxui/dom/elements.hpp>
#include <ftxui/screen/color.hpp>

namespace slate {

using namespace ftxui;

StatusBar::StatusBar() {
    component_ = Renderer([this] {
        Elements parts;

        // CWD in cyan
        parts.push_back(text(" " + cwd_ + " ") | color(Color::Cyan));

        // Git branch in green (if available)
        if (!git_branch_.empty()) {
            parts.push_back(text(" "));
            parts.push_back(text(" " + git_branch_ + " ") | color(Color::Green));
        }

        // Daemon status
        parts.push_back(text(" "));
        if (daemon_connected_) {
            parts.push_back(
                text(" daemon: connected ") | color(Color::Green));
        } else {
            parts.push_back(
                text(" daemon: disconnected ") | color(Color::Red));
        }

        // Fill remaining space
        parts.push_back(filler());

        return hbox(std::move(parts)) | bgcolor(Color::GrayDark);
    });
}

void StatusBar::set_cwd(const std::string& cwd) {
    cwd_ = cwd;
}

void StatusBar::set_git_branch(const std::string& branch) {
    git_branch_ = branch;
}

void StatusBar::set_daemon_status(bool connected) {
    daemon_connected_ = connected;
}

} // namespace slate
