//! Shared command execution runtime for panes.

pub mod config;
pub mod executor;
mod failure;

pub use executor::{
    CommandExecution, CommandExecutionError, CommandExecutionResult, CommandExecutor,
    UnsupportedWindowReason,
};
pub use failure::CommandFailureLevel;
