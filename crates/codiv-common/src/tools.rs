/// Canonical tool name constants shared across `codiv` and `codivd`.
///
/// Using these instead of raw string literals prevents silent breakage
/// when a tool name is changed or a typo slips in.
pub mod tool_names {
    pub const BASH: &str = "bash";
    pub const READ: &str = "read";
    pub const WRITE: &str = "write";
    pub const EDIT: &str = "edit";
    pub const GLOB: &str = "glob";
    pub const GREP: &str = "grep";
}
