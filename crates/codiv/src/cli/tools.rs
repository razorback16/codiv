use clap::Args;

use crate::cli::Commands;

#[derive(Args)]
pub struct ReadArgs {
    /// Path to the file to read
    #[arg(long)]
    pub path: Option<String>,
    /// Line number to start reading from (1-based)
    #[arg(long)]
    pub offset: Option<usize>,
    /// Number of lines to read
    #[arg(long)]
    pub limit: Option<usize>,
    /// Read JSON input from stdin
    #[arg(long)]
    pub json_in: bool,
    /// Output result as JSON
    #[arg(long)]
    pub json_out: bool,
    /// Print agent guide and exit
    #[arg(long)]
    pub agent_guide: bool,
}

#[derive(Args)]
pub struct WriteArgs {
    /// Path to the file to write
    #[arg(long)]
    pub path: Option<String>,
    /// Content to write
    #[arg(long)]
    pub content: Option<String>,
    /// Read JSON input from stdin
    #[arg(long)]
    pub json_in: bool,
    /// Output result as JSON
    #[arg(long)]
    pub json_out: bool,
    /// Print agent guide and exit
    #[arg(long)]
    pub agent_guide: bool,
}

#[derive(Args)]
pub struct EditArgs {
    /// Path to the file to edit
    #[arg(long)]
    pub path: Option<String>,
    /// String to find (must be unique)
    #[arg(long)]
    pub old_string: Option<String>,
    /// String to replace with
    #[arg(long)]
    pub new_string: Option<String>,
    /// Read JSON input from stdin
    #[arg(long)]
    pub json_in: bool,
    /// Output result as JSON
    #[arg(long)]
    pub json_out: bool,
    /// Print agent guide and exit
    #[arg(long)]
    pub agent_guide: bool,
}

#[derive(Args)]
pub struct GlobArgs {
    /// Glob pattern (e.g. "**/*.rs")
    #[arg(long)]
    pub pattern: Option<String>,
    /// Directory to search in
    #[arg(long)]
    pub path: Option<String>,
    /// Read JSON input from stdin
    #[arg(long)]
    pub json_in: bool,
    /// Output result as JSON
    #[arg(long)]
    pub json_out: bool,
    /// Print agent guide and exit
    #[arg(long)]
    pub agent_guide: bool,
}

#[derive(Args)]
pub struct GrepArgs {
    /// Regex pattern to search for
    #[arg(long)]
    pub pattern: Option<String>,
    /// File or directory to search in
    #[arg(long)]
    pub path: Option<String>,
    /// Glob pattern to filter files
    #[arg(long)]
    pub include: Option<String>,
    /// Maximum number of matches (default 500, -1 for unlimited)
    #[arg(long, allow_negative_numbers = true)]
    pub max_matches: Option<i64>,
    /// Read JSON input from stdin
    #[arg(long)]
    pub json_in: bool,
    /// Output result as JSON
    #[arg(long)]
    pub json_out: bool,
    /// Print agent guide and exit
    #[arg(long)]
    pub agent_guide: bool,
}

#[derive(Args)]
pub struct BashArgs {
    /// Command to execute
    #[arg(long)]
    pub command: Option<String>,
    /// Timeout in milliseconds
    #[arg(long)]
    pub timeout: Option<u64>,
    /// Read JSON input from stdin
    #[arg(long)]
    pub json_in: bool,
    /// Output result as JSON
    #[arg(long)]
    pub json_out: bool,
    /// Print agent guide and exit
    #[arg(long)]
    pub agent_guide: bool,
}

fn require<T>(opt: Option<T>, name: &str) -> T {
    match opt {
        Some(v) => v,
        None => {
            eprintln!("error: --{name} is required (or use --json-in)");
            std::process::exit(2);
        }
    }
}

fn cwd() -> String {
    std::env::current_dir()
        .expect("failed to get current directory")
        .to_string_lossy()
        .to_string()
}

fn read_json_stdin() -> serde_json::Value {
    let mut input = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)
        .expect("failed to read stdin");
    serde_json::from_str(&input).expect("invalid JSON on stdin")
}

fn output_result(result: Result<String, String>, json_out: bool) {
    if json_out {
        match result {
            Ok(r) => println!("{}", serde_json::json!({"result": r, "error": null})),
            Err(e) => println!("{}", serde_json::json!({"result": null, "error": e})),
        }
    } else {
        match result {
            Ok(r) => print!("{}", r),
            Err(e) => {
                eprintln!("{}", e);
                std::process::exit(1);
            }
        }
    }
}

pub fn handle(cmd: Commands) {
    match cmd {
        Commands::Read(args) => {
            if args.agent_guide {
                println!("{}", codiv_tools::agent_guide::agent_guide("read"));
                return;
            }
            let value = if args.json_in {
                read_json_stdin()
            } else {
                let path = require(args.path, "path");
                let mut map = serde_json::Map::new();
                map.insert("file_path".into(), path.into());
                if let Some(o) = args.offset {
                    map.insert("offset".into(), o.into());
                }
                if let Some(l) = args.limit {
                    map.insert("limit".into(), l.into());
                }
                serde_json::Value::Object(map)
            };
            output_result(codiv_tools::tools::read::execute(value), args.json_out);
        }
        Commands::Write(args) => {
            if args.agent_guide {
                println!("{}", codiv_tools::agent_guide::agent_guide("write"));
                return;
            }
            let value = if args.json_in {
                read_json_stdin()
            } else {
                let path = require(args.path, "path");
                let content = require(args.content, "content");
                let mut map = serde_json::Map::new();
                map.insert("file_path".into(), path.into());
                map.insert("content".into(), content.into());
                serde_json::Value::Object(map)
            };
            output_result(codiv_tools::tools::write::execute(value), args.json_out);
        }
        Commands::Edit(args) => {
            if args.agent_guide {
                println!("{}", codiv_tools::agent_guide::agent_guide("edit"));
                return;
            }
            let value = if args.json_in {
                read_json_stdin()
            } else {
                let path = require(args.path, "path");
                let old = require(args.old_string, "old-string");
                let new = require(args.new_string, "new-string");
                let mut map = serde_json::Map::new();
                map.insert("file_path".into(), path.into());
                map.insert("old_string".into(), old.into());
                map.insert("new_string".into(), new.into());
                serde_json::Value::Object(map)
            };
            output_result(codiv_tools::tools::edit::execute(value), args.json_out);
        }
        Commands::Glob(args) => {
            if args.agent_guide {
                println!("{}", codiv_tools::agent_guide::agent_guide("glob"));
                return;
            }
            let value = if args.json_in {
                read_json_stdin()
            } else {
                let pattern = require(args.pattern, "pattern");
                let mut map = serde_json::Map::new();
                map.insert("pattern".into(), pattern.into());
                if let Some(p) = args.path {
                    map.insert("path".into(), p.into());
                }
                serde_json::Value::Object(map)
            };
            output_result(
                codiv_tools::tools::glob::execute(value, &cwd()),
                args.json_out,
            );
        }
        Commands::Grep(args) => {
            if args.agent_guide {
                println!("{}", codiv_tools::agent_guide::agent_guide("grep"));
                return;
            }
            let value = if args.json_in {
                read_json_stdin()
            } else {
                let pattern = require(args.pattern, "pattern");
                let mut map = serde_json::Map::new();
                map.insert("pattern".into(), pattern.into());
                if let Some(p) = args.path {
                    map.insert("path".into(), p.into());
                }
                if let Some(i) = args.include {
                    map.insert("include".into(), i.into());
                }
                if let Some(m) = args.max_matches {
                    map.insert("max_matches".into(), m.into());
                }
                serde_json::Value::Object(map)
            };
            output_result(
                codiv_tools::tools::grep::execute(value, &cwd()),
                args.json_out,
            );
        }
        Commands::Bash(args) => {
            if args.agent_guide {
                println!("{}", codiv_tools::agent_guide::agent_guide("bash"));
                return;
            }
            let value = if args.json_in {
                read_json_stdin()
            } else {
                let command = require(args.command, "command");
                let mut map = serde_json::Map::new();
                map.insert("command".into(), command.into());
                if let Some(t) = args.timeout {
                    map.insert("timeout_ms".into(), t.into());
                }
                serde_json::Value::Object(map)
            };
            output_result(
                codiv_tools::tools::bash::execute(value, &cwd(), &[]),
                args.json_out,
            );
        }
        Commands::Login(args) => {
            super::login::handle_login(args);
        }
        Commands::MigrateEnv(args) => {
            super::migrate::handle_migrate_env(args);
        }
    }
}
