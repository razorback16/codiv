#include "command_index.h"
#include "bash_coprocess.h"

#include <algorithm>
#include <cstdlib>
#include <filesystem>
#include <sstream>

namespace fs = std::filesystem;

namespace slate {

void CommandIndex::scan_path_directories() {
    const char* path_env = std::getenv("PATH");
    if (!path_env) return;

    std::istringstream iss(path_env);
    std::string dir;
    while (std::getline(iss, dir, ':')) {
        if (dir.empty()) continue;

        std::error_code ec;
        if (!fs::is_directory(dir, ec)) continue;

        for (auto& entry : fs::directory_iterator(dir, ec)) {
            if (ec) break;
            if (!entry.is_regular_file(ec) && !entry.is_symlink(ec)) continue;

            auto perms = entry.status(ec).permissions();
            if (ec) continue;
            // Check if executable
            if ((perms & fs::perms::owner_exec) == fs::perms::none &&
                (perms & fs::perms::group_exec) == fs::perms::none &&
                (perms & fs::perms::others_exec) == fs::perms::none) {
                continue;
            }

            std::string name = entry.path().filename().string();
            // Don't overwrite — first in PATH wins
            if (index_.find(name) == index_.end()) {
                index_[name] = CommandInfo{
                    CommandType::Executable,
                    entry.path().string(),
                    ""
                };
            }
        }
    }
}

void CommandIndex::query_builtins(BashCoprocess* bash) {
    auto result = bash->execute("compgen -b");
    std::istringstream iss(result.output);
    std::string name;
    while (std::getline(iss, name)) {
        while (!name.empty() && (name.back() == '\r' || name.back() == '\n'))
            name.pop_back();
        if (name.empty()) continue;
        // Builtins take priority over executables
        index_[name] = CommandInfo{CommandType::Builtin, "", "shell builtin"};
    }
}

void CommandIndex::query_aliases(BashCoprocess* bash) {
    auto result = bash->execute("alias 2>/dev/null");
    std::istringstream iss(result.output);
    std::string line;
    while (std::getline(iss, line)) {
        while (!line.empty() && (line.back() == '\r' || line.back() == '\n'))
            line.pop_back();
        // Format: alias name='value'
        auto pos = line.find("alias ");
        if (pos == std::string::npos) continue;
        auto rest = line.substr(pos + 6);
        auto eq = rest.find('=');
        if (eq == std::string::npos) continue;
        std::string name = rest.substr(0, eq);
        std::string value = rest.substr(eq + 1);
        // Remove surrounding quotes from value
        if (value.size() >= 2 && value.front() == '\'' && value.back() == '\'') {
            value = value.substr(1, value.size() - 2);
        }
        index_[name] = CommandInfo{CommandType::Alias, "", value};
    }
}

void CommandIndex::query_functions(BashCoprocess* bash) {
    auto result = bash->execute("compgen -A function");
    std::istringstream iss(result.output);
    std::string name;
    while (std::getline(iss, name)) {
        while (!name.empty() && (name.back() == '\r' || name.back() == '\n'))
            name.pop_back();
        if (name.empty()) continue;
        // Don't overwrite builtins or executables with functions
        if (index_.find(name) == index_.end()) {
            index_[name] = CommandInfo{CommandType::Function, "", "shell function"};
        }
    }
}

void CommandIndex::build(BashCoprocess* bash) {
    index_.clear();

    // Scan filesystem PATH directories first
    scan_path_directories();

    // Then query bash for builtins (overwrite executables), aliases, functions
    if (bash) {
        query_builtins(bash);
        query_aliases(bash);
        query_functions(bash);
    }
}

bool CommandIndex::is_known_command(const std::string& name) const {
    return index_.find(name) != index_.end();
}

std::optional<CommandInfo> CommandIndex::lookup(const std::string& name) const {
    auto it = index_.find(name);
    if (it != index_.end()) {
        return it->second;
    }
    return std::nullopt;
}

std::vector<std::string> CommandIndex::complete(const std::string& prefix) const {
    std::vector<std::string> results;
    for (const auto& [name, info] : index_) {
        if (prefix.empty() || name.substr(0, prefix.size()) == prefix) {
            results.push_back(name);
        }
    }
    std::sort(results.begin(), results.end());
    return results;
}

} // namespace slate
