use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

pub(super) static READONLY_COMMANDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "ls", "cat", "head", "tail", "grep", "egrep", "fgrep", "rg", "ag", "less", "more",
        "pwd", "echo", "printf", "find", "wc", "printenv", "which", "whereis", "whence",
        "type", "tree", "file", "stat", "du", "df", "date", "uptime", "uname", "hostname",
        "whoami", "id", "groups", "diff", "cmp", "sort", "uniq", "tr", "cut", "paste", "column",
        "fmt", "fold", "nl", "od", "hexdump", "xxd", "md5sum", "sha256sum", "sha1sum",
        "basename", "dirname", "realpath", "readlink", "test", "[", "true", "false",
        "seq", "tput", "clear", "reset",
        // Network read-only
        "ping", "dig", "host", "nslookup", "traceroute", "tracepath",
        "ss", "netstat", "ifconfig", "ip",
        // Process read-only
        "ps", "top", "htop", "free", "lsof", "pgrep", "pidof",
    ]
    .into_iter()
    .collect()
});

pub(super) static BUILD_COMMANDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    ["make", "cmake", "ninja", "meson", "bazel", "buck", "gradle", "mvn", "ant"]
        .into_iter()
        .collect()
});

pub(super) static SAFE_SUBCOMMANDS: LazyLock<HashMap<&'static str, HashSet<&'static str>>> =
    LazyLock::new(|| {
        let mut m = HashMap::new();
        m.insert(
            "git",
            HashSet::from([
                "status", "log", "diff", "show", "branch", "tag", "stash", "describe",
                "shortlog", "blame", "reflog", "ls-files", "ls-tree", "rev-parse",
                "cat-file", "remote",
            ]),
        );
        m.insert(
            "cargo",
            HashSet::from([
                "build", "test", "check", "clippy", "fmt", "bench", "doc", "run", "tree",
                "metadata", "verify-project",
            ]),
        );
        m.insert(
            "npm",
            HashSet::from(["test", "run", "start", "list", "ls", "outdated", "audit", "ci"]),
        );
        m.insert("yarn", HashSet::from(["test", "run", "start", "list", "info"]));
        m.insert("pnpm", HashSet::from(["test", "run", "start", "list"]));
        m.insert("pip", HashSet::from(["list", "show", "freeze", "check"]));
        m.insert("brew", HashSet::from(["list", "info", "search", "doctor", "outdated"]));
        m.insert("apt", HashSet::from(["list", "show", "search"]));
        m.insert("rustup", HashSet::from(["show", "which", "check", "target", "toolchain", "component"]));
        m.insert("docker", HashSet::from(["ps", "images", "inspect", "logs", "stats", "top", "port", "info", "version", "network", "volume"]));
        m.insert("kubectl", HashSet::from(["get", "describe", "logs", "top", "explain", "api-resources", "api-versions", "config", "version"]));
        m
    });

pub(super) static HIGH_COMMANDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "rm", "mv", "wget", "curl", "chmod", "chown", "ln", "kill", "killall", "pkill",
        // Network / remote access
        "nc", "netcat", "ncat", "socat", "telnet", "ssh", "scp", "sftp", "rsync", "ftp", "tftp",
        // Reconnaissance
        "nmap", "masscan", "zmap",
        // Packet capture
        "tcpdump", "tshark",
        // Persistence / background execution
        "nohup", "disown", "setsid", "screen", "tmux",
        // Crypto / encoding (exfiltration enablers)
        "openssl",
        // Secure deletion
        "shred", "wipe",
        // Aliasing (command hijacking)
        "alias", "unalias",
    ]
    .into_iter()
    .collect()
});

pub(super) static HIGH_SUBCOMMANDS: LazyLock<HashMap<&'static str, HashSet<&'static str>>> =
    LazyLock::new(|| {
        let mut m = HashMap::new();
        m.insert(
            "git",
            HashSet::from([
                "push", "pull", "merge", "rebase", "checkout", "switch", "reset", "clean",
                "cherry-pick", "revert", "fetch", "clone", "init", "commit", "add", "rm",
                "mv", "restore",
            ]),
        );
        m.insert("npm", HashSet::from(["install", "uninstall", "update", "link", "publish"]));
        m.insert("yarn", HashSet::from(["add", "remove", "upgrade", "install"]));
        m.insert("pnpm", HashSet::from(["add", "remove", "update", "install"]));
        m.insert("pip", HashSet::from(["install", "uninstall", "download"]));
        m.insert("cargo", HashSet::from(["install", "uninstall", "publish", "add", "remove"]));
        m.insert("brew", HashSet::from(["install", "uninstall", "upgrade", "remove", "link", "unlink"]));
        m.insert("apt", HashSet::from(["install", "remove", "purge", "upgrade", "update"]));
        m.insert("docker", HashSet::from(["run", "exec", "build", "pull", "push", "rm", "rmi", "stop", "kill", "restart"]));
        m.insert("kubectl", HashSet::from(["apply", "delete", "create", "edit", "patch", "scale", "rollout"]));
        m
    });

pub(super) static CRITICAL_COMMANDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "sudo", "su", "dd", "shutdown", "reboot", "poweroff", "halt", "init",
        "systemctl", "service", "mount", "umount", "fdisk", "parted", "mkswap",
        "swapon", "swapoff", "iptables", "nft", "ip6tables", "modprobe", "insmod",
        "rmmod", "mknod", "losetup",
        // Arbitrary code execution builtins
        "eval", "exec",
        // Persistence / scheduling
        "crontab", "at", "batch",
        // Privilege / capability escalation
        "setfacl", "setcap", "chattr", "visudo",
    ]
    .into_iter()
    .collect()
});

pub(super) static SENSITIVE_PATHS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        // System directories
        "/", "/etc", "/usr", "/bin", "/sbin", "/boot", "/dev", "/proc", "/sys",
        "/var", "/lib", "/lib64", "/opt", "/root",
        // Home directory references
        "~/", "$HOME",
        // Credential / secret directories
        ".ssh", ".gnupg", ".aws", ".kube", ".config", ".docker",
        // Credential / secret files
        ".env", ".npmrc", ".pypirc", ".netrc", ".pgpass",
        // Shell config (persistence vectors)
        ".bashrc", ".bash_profile", ".profile", ".zshrc", ".zprofile",
        // History files (credential leakage)
        ".bash_history", ".zsh_history", ".node_repl_history", ".python_history",
        // Git config
        ".gitconfig",
        // Curl config (redirection attacks)
        ".curlrc",
    ]
    .into_iter()
    .collect()
});

/// Subcommands that are truly read-only (no side effects at all).
/// This is a subset of SAFE_SUBCOMMANDS -- excludes build/test/run type commands.
pub(super) static READONLY_SUBCOMMANDS: LazyLock<HashMap<&'static str, HashSet<&'static str>>> =
    LazyLock::new(|| {
        let mut m = HashMap::new();
        m.insert(
            "git",
            HashSet::from([
                "status", "log", "diff", "show", "branch", "tag", "describe",
                "shortlog", "blame", "reflog", "ls-files", "ls-tree", "rev-parse",
                "cat-file", "remote", "stash",
            ]),
        );
        m.insert("cargo", HashSet::from(["metadata", "verify-project", "tree"]));
        m.insert("npm", HashSet::from(["list", "ls", "outdated"]));
        m.insert("pip", HashSet::from(["list", "show", "freeze", "check"]));
        m.insert("brew", HashSet::from(["list", "info", "search", "outdated"]));
        m.insert("rustup", HashSet::from(["show", "which", "check"]));
        m.insert("docker", HashSet::from(["ps", "images", "inspect", "logs", "stats", "top", "port", "info", "version"]));
        m.insert("kubectl", HashSet::from(["get", "describe", "logs", "top", "explain", "api-resources", "api-versions", "version"]));
        m
    });

pub(super) static MEDIUM_COMMANDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "touch", "mkdir", "cp", "tee", "sed", "awk", "patch", "install",
        // Script interpreters (arbitrary code execution)
        "python", "python3", "ruby", "perl", "node", "php", "lua", "tclsh", "wish",
        // Shell builtins with side effects
        "source", ".", "trap", "export", "unset",
        // Can auto-confirm destructive prompts
        "yes",
        // Can execute arbitrary commands via args
        "xargs",
        // Interactive editors
        "nano", "vim", "vi", "emacs",
        // Compression (benign alone, used in exfiltration chains)
        "tar", "gzip", "gunzip", "zip", "unzip", "bzip2", "xz",
    ]
    .into_iter()
    .collect()
});
