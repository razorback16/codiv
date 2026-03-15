# Codiv Agent - Code Examples & Usage Guide

## Quick Start

### Installation

```bash
# Clone the repository
git clone https://github.com/yourusername/codiv.git
cd codiv

# Build the project
cargo build --release

# Install binaries
cargo install --path crates/codiv
cargo install --path crates/codivd
```

### Starting the Daemon

```bash
# Start the codivd daemon in the background
codivd start

# Check daemon status
codivd status

# Stop the daemon
codivd stop
```

## Architecture Examples

### IPC Communication

The codiv client communicates with codivd daemon using binary serialization over Unix sockets:

```rust
use codiv_common::messages::{ClientMessage, ServerMessage};
use tokio::net::UnixStream;

async fn send_command(command: String) -> Result<(), Box<dyn std::error::Error>> {
    // Connect to daemon socket
    let socket_path = "/tmp/codivd.sock";
    let mut stream = UnixStream::connect(socket_path).await?;
    
    // Create and send message
    let message = ClientMessage::ExecuteCommand {
        command,
        working_dir: std::env::current_dir()?,
    };
    
    let encoded = bincode::serialize(&message)?;
    stream.write_all(&encoded).await?;
    
    Ok(())
}
```

### Multi-Agent Task Decomposition

```rust
use codiv_common::agents::{TaskGraph, Task, Agent};

async fn decompose_task(user_request: &str) -> TaskGraph {
    let planner = Agent::new(AgentType::Planner);
    
    // AI decomposes complex request into subtasks
    let graph = planner.plan(user_request).await;
    
    // Example task graph:
    // 1. Research (context gathering)
    // 2. Design (architecture planning)  
    // 3. Implement (code generation)
    // 4. Review (quality check)
    // 5. Test (validation)
    
    graph
}
```

### Instant Command Detection

The terminal uses pattern matching to detect shell commands with <10ms overhead:

```rust
use regex::Regex;

fn is_shell_command(input: &str) -> bool {
    let shell_patterns = vec![
        Regex::new(r"^(ls|cd|pwd|cat|grep|find)").unwrap(),
        Regex::new(r"^git\s+").unwrap(),
        Regex::new(r"^cargo\s+").unwrap(),
        Regex::new(r"^\w+\s*=").unwrap(), // env vars
    ];
    
    shell_patterns.iter().any(|pattern| pattern.is_match(input))
}

fn handle_input(input: &str) {
    if is_shell_command(input) {
        execute_instantly(input); // <10ms, no AI
    } else {
        send_to_ai_agents(input); // Natural language task
    }
}
```

## Usage Examples

### Instant Shell Commands

```bash
# These execute instantly (no AI overhead)
$ ls -la
$ git status
$ cargo build
$ cd src/
```

### AI-Powered Tasks

```bash
# Natural language requests go to AI agents
$ analyze this codebase and find potential bugs

$ refactor the database module to use async/await

$ write comprehensive tests for the authentication system

$ research best practices for Rust error handling and apply them
```

### Multi-Model Intelligence

```rust
use aisdk::{Model, Provider};

// Different models for different tasks
fn select_model(task_complexity: TaskComplexity) -> Model {
    match task_complexity {
        TaskComplexity::Simple => Model::new(Provider::Anthropic, "claude-3-haiku"),
        TaskComplexity::Medium => Model::new(Provider::OpenAI, "gpt-4"),
        TaskComplexity::Complex => Model::new(Provider::Anthropic, "claude-3-opus"),
    }
}
```

### Terminal UI with Ratatui

```rust
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
    widgets::{Block, Borders, Paragraph},
    layout::{Layout, Constraint, Direction},
};

fn render_ui(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),      // Command output
            Constraint::Length(3),   // Input area
            Constraint::Length(1),   // Status bar
        ])
        .split(frame.size());
    
    // Render command output
    let output = Paragraph::new(app.output.clone())
        .block(Block::default().borders(Borders::ALL).title("Output"));
    frame.render_widget(output, chunks[0]);
    
    // Render input prompt
    let input = Paragraph::new(app.input.clone())
        .block(Block::default().borders(Borders::ALL).title("$ "));
    frame.render_widget(input, chunks[1]);
    
    // Render status
    let status = Paragraph::new(format!("Mode: {} | Agents: Active", app.mode));
    frame.render_widget(status, chunks[2]);
}
```

## Agent Workflow Example

```rust
#[derive(Debug, Clone)]
pub enum AgentType {
    Planner,    // Decomposes tasks into execution graph
    Coder,      // Generates and modifies code
    Reviewer,   // Reviews code quality and correctness
    Researcher, // Gathers context and information
    Tester,     // Writes and runs tests
}

pub struct AgentWorkflow {
    agents: HashMap<AgentType, Agent>,
}

impl AgentWorkflow {
    pub async fn execute(&self, task: &str) -> Result<String> {
        // Step 1: Plan
        let plan = self.agents[&AgentType::Planner]
            .execute(task)
            .await?;
        
        // Step 2: Research context
        let context = self.agents[&AgentType::Researcher]
            .execute(&plan.research_query)
            .await?;
        
        // Step 3: Generate code
        let code = self.agents[&AgentType::Coder]
            .execute_with_context(&plan.implementation, &context)
            .await?;
        
        // Step 4: Review
        let review = self.agents[&AgentType::Reviewer]
            .execute(&code)
            .await?;
        
        // Step 5: Test
        if review.approved {
            self.agents[&AgentType::Tester]
                .execute(&code)
                .await?;
        }
        
        Ok(code)
    }
}
```

## Configuration

```toml
# codivd.toml

[daemon]
socket_path = "/tmp/codivd.sock"
log_level = "info"
max_workers = 8

[ai]
default_provider = "anthropic"
fallback_provider = "openai"

[ai.models]
planner = "claude-3-sonnet"
coder = "claude-3-opus"
reviewer = "gpt-4"
researcher = "claude-3-haiku"

[terminal]
shell = "/bin/zsh"
command_timeout_ms = 5000
ai_overhead_threshold_ms = 10
```

## Contributing

```bash
# Fork and clone
git clone https://github.com/yourusername/codiv.git

# Create feature branch
git checkout -b feature/amazing-feature

# Make changes and test
cargo test
cargo clippy

# Commit and push
git commit -m "Add amazing feature"
git push origin feature/amazing-feature
```

## License

MIT License - see LICENSE file for details
