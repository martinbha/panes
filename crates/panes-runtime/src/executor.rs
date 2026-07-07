use panes_core::{Command, LayoutConfig, LayoutRequest, Rect, WindowHistory, WindowId, calculate};
use panes_platform::{
    CommandInvocation, NativePlatform, PlatformError, ScreenId, ScreenInfo, WindowInfo,
};

pub type CommandExecutionResult<T> = Result<T, CommandExecutionError>;

#[derive(Debug)]
pub struct CommandExecutor<P> {
    platform: P,
    config: LayoutConfig,
    history: WindowHistory,
}

impl<P> CommandExecutor<P> {
    #[must_use]
    pub fn new(platform: P, config: LayoutConfig) -> Self {
        Self {
            platform,
            config,
            history: WindowHistory::default(),
        }
    }

    #[must_use]
    pub fn with_default_config(platform: P) -> Self {
        Self::new(platform, LayoutConfig::default())
    }

    #[must_use]
    pub const fn platform(&self) -> &P {
        &self.platform
    }

    #[must_use]
    pub const fn platform_mut(&mut self) -> &mut P {
        &mut self.platform
    }

    #[must_use]
    pub const fn config(&self) -> &LayoutConfig {
        &self.config
    }

    pub fn set_config(&mut self, config: LayoutConfig) {
        self.config = config;
    }

    #[must_use]
    pub const fn history(&self) -> &WindowHistory {
        &self.history
    }

    #[must_use]
    pub const fn history_mut(&mut self) -> &mut WindowHistory {
        &mut self.history
    }
}

impl<P: NativePlatform> CommandExecutor<P> {
    pub fn execute(
        &mut self,
        invocation: CommandInvocation,
    ) -> CommandExecutionResult<CommandExecution> {
        let window = self
            .platform
            .front_window()
            .map_err(CommandExecutionError::Platform)?
            .ok_or(CommandExecutionError::NoFocusedWindow)?;
        validate_window(&window)?;

        let screens = self
            .platform
            .screens()
            .map_err(CommandExecutionError::Platform)?;
        if screens.is_empty() {
            return Err(CommandExecutionError::NoScreens);
        }

        let screen = self.target_screen(invocation.command, &window, &screens)?;
        let requested_rect = if invocation.command == Command::Restore {
            self.history
                .restore_rect(window.id)
                .ok_or(CommandExecutionError::NoRestoreRect {
                    window_id: window.id,
                })?
        } else {
            calculate(
                LayoutRequest {
                    command: invocation.command,
                    window: window.rect,
                    screen: screen.work_area,
                },
                &self.config,
            )
            .rect
        };

        let applied_rect = self
            .platform
            .set_window_rect(window.id, requested_rect)
            .map_err(CommandExecutionError::Platform)?;

        if invocation.command == Command::Restore {
            self.history.clear_restore_rect(window.id);
        } else if self.history.restore_rect(window.id).is_none() {
            self.history.set_restore_rect(window.id, window.rect);
        }

        self.history
            .record_command(window.id, invocation.command, applied_rect);

        Ok(CommandExecution {
            invocation,
            window_id: window.id,
            screen_id: screen.id,
            previous_rect: window.rect,
            requested_rect,
            applied_rect,
        })
    }

    fn target_screen<'a>(
        &self,
        command: Command,
        window: &WindowInfo,
        screens: &'a [ScreenInfo],
    ) -> CommandExecutionResult<&'a ScreenInfo> {
        let current = self.current_screen(window, screens)?;

        match command {
            Command::NextDisplay => Ok(adjacent_screen(current.id, screens, 1)),
            Command::PreviousDisplay => Ok(adjacent_screen(current.id, screens, -1)),
            _ => Ok(current),
        }
    }

    fn current_screen<'a>(
        &self,
        window: &WindowInfo,
        screens: &'a [ScreenInfo],
    ) -> CommandExecutionResult<&'a ScreenInfo> {
        if let Some(screen) = screen_with_largest_window_overlap(window.rect, screens) {
            return Ok(screen);
        }

        let cursor = self
            .platform
            .cursor_position()
            .map_err(CommandExecutionError::Platform)?;

        screen_containing_point(cursor, screens).ok_or(CommandExecutionError::NoTargetScreen {
            window_id: window.id,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CommandExecution {
    pub invocation: CommandInvocation,
    pub window_id: WindowId,
    pub screen_id: ScreenId,
    pub previous_rect: Rect,
    pub requested_rect: Rect,
    pub applied_rect: Rect,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CommandExecutionError {
    Platform(PlatformError),
    NoFocusedWindow,
    NoScreens,
    NoTargetScreen {
        window_id: WindowId,
    },
    NoRestoreRect {
        window_id: WindowId,
    },
    UnsupportedWindow {
        window_id: WindowId,
        reason: UnsupportedWindowReason,
    },
}

impl std::fmt::Display for CommandExecutionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Platform(error) => write!(formatter, "platform error: {error:?}"),
            Self::NoFocusedWindow => write!(formatter, "no focused window"),
            Self::NoScreens => write!(formatter, "no screens available"),
            Self::NoTargetScreen { window_id } => {
                write!(formatter, "no target screen for window {window_id:?}")
            }
            Self::NoRestoreRect { window_id } => {
                write!(formatter, "no restore rect for window {window_id:?}")
            }
            Self::UnsupportedWindow { window_id, reason } => {
                write!(formatter, "unsupported window {window_id:?}: {reason:?}")
            }
        }
    }
}

impl std::error::Error for CommandExecutionError {}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum UnsupportedWindowReason {
    Hidden,
    Minimized,
    NotResizable,
}

fn validate_window(window: &WindowInfo) -> CommandExecutionResult<()> {
    let reason = if window.is_hidden {
        Some(UnsupportedWindowReason::Hidden)
    } else if window.is_minimized {
        Some(UnsupportedWindowReason::Minimized)
    } else if !window.is_resizable {
        Some(UnsupportedWindowReason::NotResizable)
    } else {
        None
    };

    if let Some(reason) = reason {
        return Err(CommandExecutionError::UnsupportedWindow {
            window_id: window.id,
            reason,
        });
    }

    Ok(())
}

fn screen_with_largest_window_overlap(
    window_rect: Rect,
    screens: &[ScreenInfo],
) -> Option<&ScreenInfo> {
    screens
        .iter()
        .filter_map(|screen| {
            window_rect
                .intersection(screen.frame)
                .map(|intersection| (screen, intersection.area()))
        })
        .max_by(|(_, left_area), (_, right_area)| left_area.total_cmp(right_area))
        .map(|(screen, _)| screen)
}

fn screen_containing_point(
    point: panes_core::Point,
    screens: &[ScreenInfo],
) -> Option<&ScreenInfo> {
    screens
        .iter()
        .find(|screen| screen.frame.contains_point(point))
}

fn adjacent_screen(current_id: ScreenId, screens: &[ScreenInfo], direction: isize) -> &ScreenInfo {
    let mut ordered = screens.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        left.frame
            .min_x()
            .total_cmp(&right.frame.min_x())
            .then_with(|| left.frame.min_y().total_cmp(&right.frame.min_y()))
            .then_with(|| left.id.0.cmp(&right.id.0))
    });

    let current_index = ordered
        .iter()
        .position(|screen| screen.id == current_id)
        .unwrap_or(0);
    let target_index =
        (current_index as isize + direction).rem_euclid(ordered.len() as isize) as usize;

    ordered[target_index]
}
