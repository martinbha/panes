//! Shared command execution runtime for panes.

pub mod executor;

pub use executor::{
    CommandExecution, CommandExecutionError, CommandExecutionResult, CommandExecutor,
    UnsupportedWindowReason,
};
