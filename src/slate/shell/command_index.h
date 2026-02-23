#pragma once

#include <optional>
#include <string>
#include <unordered_map>
#include <vector>

namespace slate {

class BashCoprocess;

enum class CommandType {
    Executable,
    Builtin,
    Alias,
    Function,
    Unknown
};

struct CommandInfo {
    CommandType type = CommandType::Unknown;
    std::string path;         // Full path for executables
    std::string description;  // Optional description
};

class CommandIndex {
public:
    CommandIndex() = default;
    ~CommandIndex() = default;

    /// Build the index by scanning PATH and querying bash for builtins,
    /// aliases, and functions.
    void build(BashCoprocess* bash);

    /// Check if a command name exists in the index.
    bool is_known_command(const std::string& name) const;

    /// Look up full info for a command.
    std::optional<CommandInfo> lookup(const std::string& name) const;

    /// Return all commands starting with the given prefix, sorted.
    std::vector<std::string> complete(const std::string& prefix) const;

    /// Number of indexed commands.
    std::size_t size() const { return index_.size(); }

private:
    void scan_path_directories();
    void query_builtins(BashCoprocess* bash);
    void query_aliases(BashCoprocess* bash);
    void query_functions(BashCoprocess* bash);

    std::unordered_map<std::string, CommandInfo> index_;
};

} // namespace slate
