use std::cell::RefCell;

use panes_core::{Command, LayoutConfig, Point, Rect, WindowId};
use panes_platform::{
    CommandInvocation, CommandSource, HotkeyBinding, MenuEntry, NativePlatform, PlatformResult,
    ScreenId, ScreenInfo, WindowInfo,
};
use panes_runtime::{CommandExecutionError, CommandExecutor, UnsupportedWindowReason};

const WINDOW_ID: WindowId = WindowId(7);

#[derive(Debug)]
struct FakePlatform {
    cursor: Point,
    screens: Vec<ScreenInfo>,
    front_window: Option<WindowInfo>,
    set_calls: RefCell<Vec<(WindowId, Rect)>>,
}

impl FakePlatform {
    fn new() -> Self {
        Self {
            cursor: Point::new(150.0, 150.0),
            screens: vec![screen(1, 0.0), screen(2, 1000.0)],
            front_window: Some(window(Rect::new(100.0, 100.0, 200.0, 100.0))),
            set_calls: RefCell::new(Vec::new()),
        }
    }

    fn with_front_window(front_window: Option<WindowInfo>) -> Self {
        Self {
            front_window,
            ..Self::new()
        }
    }
}

impl NativePlatform for FakePlatform {
    fn platform_name(&self) -> &'static str {
        "fake"
    }

    fn cursor_position(&self) -> PlatformResult<Point> {
        Ok(self.cursor)
    }

    fn screens(&self) -> PlatformResult<Vec<ScreenInfo>> {
        Ok(self.screens.clone())
    }

    fn front_window(&self) -> PlatformResult<Option<WindowInfo>> {
        Ok(self.front_window.clone())
    }

    fn set_window_rect(&self, window_id: WindowId, rect: Rect) -> PlatformResult<Rect> {
        self.set_calls.borrow_mut().push((window_id, rect));
        Ok(rect)
    }

    fn register_hotkeys(&mut self, _bindings: &[HotkeyBinding]) -> PlatformResult<()> {
        Ok(())
    }

    fn show_tray_menu(&mut self, _entries: &[MenuEntry]) -> PlatformResult<()> {
        Ok(())
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
        id: WINDOW_ID,
        app_id: "test.app".to_owned(),
        title: "Test Window".to_owned(),
        rect,
        is_resizable: true,
        is_minimized: false,
        is_hidden: false,
    }
}

fn keyboard(command: Command) -> CommandInvocation {
    CommandInvocation {
        command,
        source: CommandSource::Keyboard,
    }
}

fn menu(command: Command) -> CommandInvocation {
    CommandInvocation {
        command,
        source: CommandSource::Menu,
    }
}

/// Expected rect for each command on a 1000x800 screen at the origin with the
/// default config and a 200x100 window at (100, 100).
///
/// Commands that depend on history return `None` and are covered by
/// dedicated tests below. New commands must be added here, which the
/// exhaustive match enforces at compile time.
fn expected_rect(command: Command) -> Option<Rect> {
    let rect = match command {
        Command::LeftHalf => Rect::new(0.0, 0.0, 500.0, 800.0),
        Command::RightHalf => Rect::new(500.0, 0.0, 500.0, 800.0),
        Command::CenterHalf => Rect::new(250.0, 0.0, 500.0, 800.0),
        Command::TopHalf => Rect::new(0.0, 400.0, 1000.0, 400.0),
        Command::BottomHalf => Rect::new(0.0, 0.0, 1000.0, 400.0),
        Command::TopLeft => Rect::new(0.0, 400.0, 500.0, 400.0),
        Command::TopRight => Rect::new(500.0, 400.0, 500.0, 400.0),
        Command::BottomLeft => Rect::new(0.0, 0.0, 500.0, 400.0),
        Command::BottomRight => Rect::new(500.0, 0.0, 500.0, 400.0),
        Command::FirstThird => Rect::new(0.0, 0.0, 333.0, 800.0),
        Command::CenterThird => Rect::new(333.0, 0.0, 333.0, 800.0),
        Command::LastThird => Rect::new(666.0, 0.0, 333.0, 800.0),
        Command::FirstTwoThirds => Rect::new(0.0, 0.0, 666.0, 800.0),
        Command::CenterTwoThirds => Rect::new(167.0, 0.0, 666.0, 800.0),
        Command::LastTwoThirds => Rect::new(333.0, 0.0, 666.0, 800.0),
        Command::Maximize => Rect::new(0.0, 0.0, 1000.0, 800.0),
        Command::AlmostMaximize => Rect::new(50.0, 40.0, 900.0, 720.0),
        Command::MaximizeHeight => Rect::new(100.0, 0.0, 200.0, 800.0),
        Command::Center => Rect::new(400.0, 350.0, 200.0, 100.0),
        Command::MoveLeft => Rect::new(0.0, 100.0, 200.0, 100.0),
        Command::MoveRight => Rect::new(800.0, 100.0, 200.0, 100.0),
        Command::MoveUp => Rect::new(100.0, 700.0, 200.0, 100.0),
        Command::MoveDown => Rect::new(100.0, 0.0, 200.0, 100.0),
        Command::Grow => Rect::new(85.0, 85.0, 230.0, 130.0),
        // The 200x100 window is already below the 25%-of-work-area shrink floor
        // (250x200 here), so Shrink holds its size and only re-centers.
        Command::Shrink => Rect::new(100.0, 100.0, 200.0, 100.0),
        Command::Restore => return None,
    };
    Some(rect)
}

#[test]
fn every_command_produces_its_expected_rect() {
    for &command in Command::ALL {
        let Some(expected) = expected_rect(command) else {
            continue;
        };

        let mut executor = CommandExecutor::with_default_config(FakePlatform::new());
        let execution = executor
            .execute(keyboard(command))
            .unwrap_or_else(|error| panic!("{} failed: {error}", command.label()));

        assert_eq!(
            execution.requested_rect,
            expected,
            "unexpected rect for {}",
            command.label()
        );
        assert_eq!(execution.applied_rect, expected);
        assert_eq!(execution.screen_id, ScreenId(1));
        assert_eq!(
            executor.platform().set_calls.borrow().as_slice(),
            &[(WINDOW_ID, expected)]
        );
    }
}

#[test]
fn repeated_execution_iterates_layout_but_applies_one_frame_change() {
    let mut executor = CommandExecutor::with_default_config(FakePlatform::new());

    let execution = executor
        .execute_repeated(keyboard(Command::Grow), 2)
        .unwrap();

    // Two Grow steps from the 200x100 window at (100, 100): 30px per step on
    // each dimension, re-centered each time, exactly as two sequential
    // executions would produce.
    let expected = Rect::new(70.0, 70.0, 260.0, 160.0);
    assert_eq!(execution.requested_rect, expected);
    assert_eq!(
        executor.platform().set_calls.borrow().as_slice(),
        &[(WINDOW_ID, expected)]
    );
}

#[test]
fn zero_repeats_executes_once() {
    let mut executor = CommandExecutor::with_default_config(FakePlatform::new());

    let execution = executor
        .execute_repeated(keyboard(Command::Grow), 0)
        .unwrap();

    assert_eq!(Some(execution.requested_rect), expected_rect(Command::Grow));
}

#[test]
fn repeated_restore_runs_once_against_history() {
    let original = Rect::new(100.0, 100.0, 200.0, 100.0);
    let mut executor = CommandExecutor::with_default_config(FakePlatform::new());

    executor.execute(keyboard(Command::LeftHalf)).unwrap();
    let restore = executor
        .execute_repeated(keyboard(Command::Restore), 5)
        .unwrap();

    assert_eq!(restore.requested_rect, original);
}

#[test]
fn menu_and_keyboard_invocations_share_history_for_restore() {
    let original = Rect::new(100.0, 100.0, 200.0, 100.0);
    let mut executor = CommandExecutor::with_default_config(FakePlatform::new());

    executor.execute(menu(Command::LeftHalf)).unwrap();
    executor.execute(keyboard(Command::Maximize)).unwrap();
    let restore = executor.execute(menu(Command::Restore)).unwrap();

    assert_eq!(restore.requested_rect, original);
    assert_eq!(executor.history().restore_rect(WINDOW_ID), None);
}

#[test]
fn window_spanning_two_displays_uses_the_largest_overlap() {
    let platform = FakePlatform {
        front_window: Some(window(Rect::new(900.0, 100.0, 600.0, 200.0))),
        ..FakePlatform::new()
    };
    let mut executor = CommandExecutor::with_default_config(platform);

    let execution = executor.execute(keyboard(Command::Maximize)).unwrap();

    assert_eq!(execution.screen_id, ScreenId(2));
    assert_eq!(
        execution.requested_rect,
        Rect::new(1000.0, 0.0, 1000.0, 800.0)
    );
}

#[test]
fn window_already_at_the_target_rect_still_succeeds() {
    let target = Rect::new(0.0, 0.0, 500.0, 800.0);
    let platform = FakePlatform {
        front_window: Some(window(target)),
        ..FakePlatform::new()
    };
    let mut executor = CommandExecutor::with_default_config(platform);

    let execution = executor.execute(keyboard(Command::LeftHalf)).unwrap();

    assert_eq!(execution.requested_rect, target);
    assert_eq!(executor.history().restore_rect(WINDOW_ID), Some(target));
}

#[test]
fn gaps_inset_tiled_commands_but_not_positioning_commands() {
    let config = LayoutConfig {
        gap: 10.0,
        ..LayoutConfig::default()
    };

    let mut executor = CommandExecutor::new(FakePlatform::new(), config.clone());
    let tiled = executor.execute(keyboard(Command::LeftHalf)).unwrap();
    assert_eq!(tiled.requested_rect, Rect::new(10.0, 10.0, 480.0, 780.0));

    let mut executor = CommandExecutor::new(FakePlatform::new(), config);
    let positioned = executor.execute(keyboard(Command::Center)).unwrap();
    assert_eq!(
        positioned.requested_rect,
        Rect::new(400.0, 350.0, 200.0, 100.0)
    );
}

#[test]
fn no_command_panics_without_a_focused_window() {
    for &command in Command::ALL {
        let mut executor =
            CommandExecutor::with_default_config(FakePlatform::with_front_window(None));

        let error = executor.execute(keyboard(command)).unwrap_err();

        assert_eq!(
            error,
            CommandExecutionError::NoFocusedWindow,
            "unexpected error for {}",
            command.label()
        );
    }
}

#[test]
fn no_command_panics_on_unsupported_windows() {
    let unsupported = [
        (
            WindowInfo {
                is_hidden: true,
                ..window(Rect::new(100.0, 100.0, 200.0, 100.0))
            },
            UnsupportedWindowReason::Hidden,
        ),
        (
            WindowInfo {
                is_minimized: true,
                ..window(Rect::new(100.0, 100.0, 200.0, 100.0))
            },
            UnsupportedWindowReason::Minimized,
        ),
        (
            WindowInfo {
                is_resizable: false,
                ..window(Rect::new(100.0, 100.0, 200.0, 100.0))
            },
            UnsupportedWindowReason::NotResizable,
        ),
    ];

    for (window, reason) in unsupported {
        for &command in Command::ALL {
            let mut executor = CommandExecutor::with_default_config(
                FakePlatform::with_front_window(Some(window.clone())),
            );

            let error = executor.execute(keyboard(command)).unwrap_err();

            assert_eq!(
                error,
                CommandExecutionError::UnsupportedWindow {
                    window_id: WINDOW_ID,
                    reason,
                },
                "unexpected error for {}",
                command.label()
            );

            assert!(executor.platform().set_calls.borrow().is_empty());
        }
    }
}
