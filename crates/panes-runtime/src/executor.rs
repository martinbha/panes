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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use panes_core::{Point, WindowId};
    use panes_platform::{CommandSource, HotkeyBinding, MenuEntry, PlatformResult};

    use super::*;

    #[derive(Debug)]
    struct FakePlatform {
        cursor_position: PlatformResult<Point>,
        screens: PlatformResult<Vec<ScreenInfo>>,
        front_window: PlatformResult<Option<WindowInfo>>,
        set_result: RefCell<Option<PlatformResult<Rect>>>,
        set_calls: RefCell<Vec<(WindowId, Rect)>>,
    }

    impl FakePlatform {
        fn new() -> Self {
            Self {
                cursor_position: Ok(Point::new(100.0, 100.0)),
                screens: Ok(vec![screen(1, 0.0), screen(2, 1000.0)]),
                front_window: Ok(Some(window(Rect::new(100.0, 100.0, 200.0, 100.0)))),
                set_result: RefCell::new(None),
                set_calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl NativePlatform for FakePlatform {
        fn platform_name(&self) -> &'static str {
            "fake"
        }

        fn cursor_position(&self) -> PlatformResult<Point> {
            self.cursor_position.clone()
        }

        fn screens(&self) -> PlatformResult<Vec<ScreenInfo>> {
            self.screens.clone()
        }

        fn front_window(&self) -> PlatformResult<Option<WindowInfo>> {
            self.front_window.clone()
        }

        fn set_window_rect(&self, window_id: WindowId, rect: Rect) -> PlatformResult<Rect> {
            self.set_calls.borrow_mut().push((window_id, rect));

            if let Some(result) = self.set_result.borrow_mut().take() {
                return result;
            }

            Ok(rect)
        }

        fn register_hotkeys(&mut self, _bindings: &[HotkeyBinding]) -> PlatformResult<()> {
            Ok(())
        }

        fn show_tray_menu(&mut self, _entries: &[MenuEntry]) -> PlatformResult<()> {
            Ok(())
        }
    }

    #[test]
    fn executes_layout_command_against_focused_window_screen() {
        let mut executor = CommandExecutor::with_default_config(FakePlatform::new());

        let execution = executor.execute(invocation(Command::LeftHalf)).unwrap();

        assert_eq!(execution.screen_id, ScreenId(1));
        assert_eq!(
            execution.previous_rect,
            Rect::new(100.0, 100.0, 200.0, 100.0)
        );
        assert_eq!(execution.requested_rect, Rect::new(0.0, 0.0, 500.0, 800.0));
        assert_eq!(execution.applied_rect, execution.requested_rect);
        assert_eq!(
            executor.platform().set_calls.borrow().as_slice(),
            &[(WindowId(42), Rect::new(0.0, 0.0, 500.0, 800.0))]
        );
        assert_eq!(
            executor.history().restore_rect(WindowId(42)),
            Some(Rect::new(100.0, 100.0, 200.0, 100.0))
        );
    }

    #[test]
    fn uses_cursor_screen_when_window_has_no_screen_overlap() {
        let platform = FakePlatform {
            cursor_position: Ok(Point::new(1200.0, 100.0)),
            front_window: Ok(Some(window(Rect::new(-5000.0, -5000.0, 200.0, 100.0)))),
            ..FakePlatform::new()
        };
        let mut executor = CommandExecutor::with_default_config(platform);

        let execution = executor.execute(invocation(Command::Maximize)).unwrap();

        assert_eq!(execution.screen_id, ScreenId(2));
        assert_eq!(
            execution.requested_rect,
            Rect::new(1000.0, 0.0, 1000.0, 800.0)
        );
    }

    #[test]
    fn moves_display_commands_to_adjacent_screen() {
        let mut executor = CommandExecutor::with_default_config(FakePlatform::new());

        let execution = executor.execute(invocation(Command::NextDisplay)).unwrap();

        assert_eq!(execution.screen_id, ScreenId(2));
        assert_eq!(
            execution.requested_rect,
            Rect::new(1400.0, 350.0, 200.0, 100.0)
        );
    }

    #[test]
    fn previous_display_wraps_to_last_screen() {
        let platform = FakePlatform {
            screens: Ok(vec![screen(1, 0.0), screen(2, 1000.0), screen(3, 2000.0)]),
            ..FakePlatform::new()
        };
        let mut executor = CommandExecutor::with_default_config(platform);

        let execution = executor
            .execute(invocation(Command::PreviousDisplay))
            .unwrap();

        assert_eq!(execution.screen_id, ScreenId(3));
        assert_eq!(
            execution.requested_rect,
            Rect::new(2400.0, 350.0, 200.0, 100.0)
        );
    }

    #[test]
    fn restores_previous_rect_after_successful_command() {
        let mut executor = CommandExecutor::with_default_config(FakePlatform::new());

        executor.execute(invocation(Command::LeftHalf)).unwrap();
        let restore = executor.execute(invocation(Command::Restore)).unwrap();

        assert_eq!(
            restore.requested_rect,
            Rect::new(100.0, 100.0, 200.0, 100.0)
        );
        assert_eq!(executor.history().restore_rect(WindowId(42)), None);
        assert_eq!(
            executor.platform().set_calls.borrow().as_slice(),
            &[
                (WindowId(42), Rect::new(0.0, 0.0, 500.0, 800.0)),
                (WindowId(42), Rect::new(100.0, 100.0, 200.0, 100.0)),
            ]
        );
    }

    #[test]
    fn preserves_first_restore_rect_across_multiple_commands() {
        let mut executor = CommandExecutor::with_default_config(FakePlatform::new());

        executor.execute(invocation(Command::LeftHalf)).unwrap();
        executor.execute(invocation(Command::Maximize)).unwrap();

        assert_eq!(
            executor.history().restore_rect(WindowId(42)),
            Some(Rect::new(100.0, 100.0, 200.0, 100.0))
        );

        let restore = executor.execute(invocation(Command::Restore)).unwrap();

        assert_eq!(
            restore.requested_rect,
            Rect::new(100.0, 100.0, 200.0, 100.0)
        );
        assert_eq!(
            executor.platform().set_calls.borrow().as_slice(),
            &[
                (WindowId(42), Rect::new(0.0, 0.0, 500.0, 800.0)),
                (WindowId(42), Rect::new(0.0, 0.0, 1000.0, 800.0)),
                (WindowId(42), Rect::new(100.0, 100.0, 200.0, 100.0)),
            ]
        );
    }

    #[test]
    fn reports_missing_restore_rect() {
        let mut executor = CommandExecutor::with_default_config(FakePlatform::new());

        let error = executor.execute(invocation(Command::Restore)).unwrap_err();

        assert_eq!(
            error,
            CommandExecutionError::NoRestoreRect {
                window_id: WindowId(42)
            }
        );
        assert!(executor.platform().set_calls.borrow().is_empty());
    }

    #[test]
    fn reports_no_focused_window() {
        let platform = FakePlatform {
            front_window: Ok(None),
            ..FakePlatform::new()
        };
        let mut executor = CommandExecutor::with_default_config(platform);

        let error = executor.execute(invocation(Command::Maximize)).unwrap_err();

        assert_eq!(error, CommandExecutionError::NoFocusedWindow);
    }

    #[test]
    fn reports_no_screens() {
        let platform = FakePlatform {
            screens: Ok(Vec::new()),
            ..FakePlatform::new()
        };
        let mut executor = CommandExecutor::with_default_config(platform);

        let error = executor.execute(invocation(Command::Maximize)).unwrap_err();

        assert_eq!(error, CommandExecutionError::NoScreens);
    }

    #[test]
    fn reports_no_target_screen_when_window_and_cursor_miss_screens() {
        let platform = FakePlatform {
            cursor_position: Ok(Point::new(-100.0, -100.0)),
            front_window: Ok(Some(window(Rect::new(-5000.0, -5000.0, 200.0, 100.0)))),
            ..FakePlatform::new()
        };
        let mut executor = CommandExecutor::with_default_config(platform);

        let error = executor.execute(invocation(Command::Maximize)).unwrap_err();

        assert_eq!(
            error,
            CommandExecutionError::NoTargetScreen {
                window_id: WindowId(42)
            }
        );
    }

    #[test]
    fn reports_unsupported_windows() {
        let platform = FakePlatform {
            front_window: Ok(Some(WindowInfo {
                is_resizable: false,
                ..window(Rect::new(100.0, 100.0, 200.0, 100.0))
            })),
            ..FakePlatform::new()
        };
        let mut executor = CommandExecutor::with_default_config(platform);

        let error = executor.execute(invocation(Command::Maximize)).unwrap_err();

        assert_eq!(
            error,
            CommandExecutionError::UnsupportedWindow {
                window_id: WindowId(42),
                reason: UnsupportedWindowReason::NotResizable,
            }
        );
    }

    #[test]
    fn propagates_platform_errors() {
        let platform = FakePlatform {
            set_result: RefCell::new(Some(Err(PlatformError::Native(
                "cannot move window".to_owned(),
            )))),
            ..FakePlatform::new()
        };
        let mut executor = CommandExecutor::with_default_config(platform);

        let error = executor.execute(invocation(Command::Maximize)).unwrap_err();

        assert_eq!(
            error,
            CommandExecutionError::Platform(PlatformError::Native("cannot move window".to_owned()))
        );
    }

    fn invocation(command: Command) -> CommandInvocation {
        CommandInvocation {
            command,
            source: CommandSource::Keyboard,
        }
    }

    fn screen(id: u64, x: f64) -> ScreenInfo {
        ScreenInfo {
            id: ScreenId(id),
            name: format!("Screen {id}"),
            frame: Rect::new(x, 0.0, 1000.0, 800.0),
            work_area: Rect::new(x, 0.0, 1000.0, 800.0),
        }
    }

    fn window(rect: Rect) -> WindowInfo {
        WindowInfo {
            id: WindowId(42),
            app_id: "test.app".to_owned(),
            title: "Test Window".to_owned(),
            rect,
            is_resizable: true,
            is_minimized: false,
            is_hidden: false,
        }
    }
}
