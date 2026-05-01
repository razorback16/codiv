// Re-export submodule items to preserve the existing `crate::agent::config::*` API.

pub use super::models::{
    AppConfig, ConfigWatcherGuard, ModelAssignment, ModelCatalog,
    ProviderConfig, add_permission_to_config,
};
pub use super::streaming::{
    simple_text_completion, spawn_token_refresh_task, stream_from_config,
    streaming_text_completion,
};
