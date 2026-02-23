#pragma once

#include <ftxui/component/component.hpp>
#include <string>

namespace slate {

class StatusBar {
public:
    StatusBar();
    ~StatusBar() = default;

    void set_cwd(const std::string& cwd);
    void set_git_branch(const std::string& branch);
    void set_daemon_status(bool connected);

    ftxui::Component component() const { return component_; }

private:
    std::string cwd_ = "~";
    std::string git_branch_ = "";
    bool daemon_connected_ = false;

    ftxui::Component component_;
};

} // namespace slate
