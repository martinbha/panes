use panes_platform::PlatformError;

use crate::CommandExecutionError;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CommandFailureLevel {
    /// Expected transient desktop state; trace it in development builds only.
    Debug,
    /// An actionable platform or display failure; keep it in release logs too.
    Error,
}

impl CommandExecutionError {
    #[must_use]
    pub const fn failure_level(&self) -> CommandFailureLevel {
        match self {
            Self::Platform(PlatformError::PermissionDenied(_) | PlatformError::Native(_))
            | Self::NoScreens
            | Self::InvalidScreenGeometry { .. }
            | Self::NoTargetScreen { .. } => CommandFailureLevel::Error,
            Self::Platform(PlatformError::Unsupported(_) | PlatformError::NotFound(_))
            | Self::NoFocusedWindow
            | Self::NoRestoreRect { .. }
            | Self::UnsupportedWindow { .. } => CommandFailureLevel::Debug,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separates_transient_state_from_actionable_errors() {
        assert_eq!(
            CommandExecutionError::NoFocusedWindow.failure_level(),
            CommandFailureLevel::Debug
        );
        assert_eq!(
            CommandExecutionError::Platform(PlatformError::PermissionDenied("grant access"))
                .failure_level(),
            CommandFailureLevel::Error
        );
        assert_eq!(
            CommandExecutionError::NoScreens.failure_level(),
            CommandFailureLevel::Error
        );
    }
}
