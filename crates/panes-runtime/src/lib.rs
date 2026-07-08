//! Shared command execution runtime for panes.

pub mod config;
pub mod executor;

pub use executor::{
    CommandExecution, CommandExecutionError, CommandExecutionResult, CommandExecutor,
    UnsupportedWindowReason,
};
