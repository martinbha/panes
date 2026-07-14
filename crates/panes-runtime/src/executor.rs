use std::collections::{HashMap, VecDeque};

use panes_core::{
    Command, LayoutConfig, LayoutRequest, MAX_TRACKED_WINDOWS, Rect, WindowHistory, WindowId,
    calculate,
};
use panes_platform::{
    CommandInvocation, NativePlatform, PlatformError, ScreenId, ScreenInfo, WindowInfo,
};

pub type CommandExecutionResult<T> = Result<T, CommandExecutionError>;

#[derive(Debug)]
pub struct CommandExecutor<P> {
    platform: P,
    config: LayoutConfig,
    history: WindowHistory,
    window_identities: HashMap<WindowId, WindowIdentity>,
    recent_windows: VecDeque<WindowId>,
    screen_configuration: Option<Vec<ScreenGeometry>>,
}

impl<P> CommandExecutor<P> {
    #[must_use]
    pub fn new(platform: P, config: LayoutConfig) -> Self {
        Self {
            platform,
            config,
            history: WindowHistory::default(),
            window_identities: HashMap::new(),
            recent_windows: VecDeque::new(),
            screen_configuration: None,
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
        self.execute_repeated(invocation, 1)
    }

    /// Executes `invocation` as if it were invoked `repeats` times in a row,
    /// but applies only one native frame change. The layout math is iterated
    /// in memory, so the result is exactly what `repeats` sequential
    /// executions would produce — this lets hotkey presses that queued up
    /// while an earlier command was executing collapse into a single
    /// synchronous window update. `Restore` is idempotent against history and
    /// always runs once.
    pub fn execute_repeated(
        &mut self,
        invocation: CommandInvocation,
        repeats: usize,
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
        if let Some(screen) = screens.iter().find(|screen| {
            !screen.frame.is_valid()
                || !screen.work_area.is_valid()
                || !rect_contains_with_tolerance(screen.frame, screen.work_area)
        }) {
            return Err(CommandExecutionError::InvalidScreenGeometry {
                screen_id: screen.id,
            });
        }

        self.refresh_history_context(&window, &screens);

        let mut screen = self.current_screen(&window, &screens)?;
        let mut resulting_command = invocation.command;
        let requested_rect = if invocation.command == Command::Restore {
            let restore_rect = self.history.restore_rect(window.id).ok_or(
                CommandExecutionError::NoRestoreRect {
                    window_id: window.id,
                },
            )?;
            if !self
                .history
                .last_command(window.id)
                .is_some_and(|record| rects_match(record.rect, window.rect))
            {
                self.history.clear_window(window.id);
                return Err(CommandExecutionError::StaleWindowHistory {
                    window_id: window.id,
                });
            }
            if let Some(restore_screen) = screen_with_largest_window_overlap(restore_rect, &screens)
            {
                screen = restore_screen;
            }
            fit_rect_to_work_area(restore_rect, screen.work_area)
        } else {
            let matching_record = self
                .history
                .last_command(window.id)
                .filter(|record| rects_match(record.rect, window.rect));
            let mut rect = matching_record
                .filter(|record| record.command == invocation.command)
                .map_or(window.rect, |record| record.requested_rect);
            let mut previous_command = matching_record.map(|record| {
                let previous_rect = if record.command == invocation.command {
                    record.requested_rect
                } else {
                    record.rect
                };
                (record.command, previous_rect)
            });

            for _ in 0..repeats.max(1) {
                let mut layout_command = invocation.command;
                if navigates_between_screens(invocation.command)
                    && repeats_command(previous_command, rect, invocation.command)
                {
                    if let Some(next) = next_screen(screen, &screens, invocation.command) {
                        screen = next;
                        layout_command = destination_edge_command(invocation.command);
                    }
                }

                rect = calculate(
                    LayoutRequest {
                        command: layout_command,
                        window: rect,
                        screen: screen.work_area,
                    },
                    &self.config,
                )
                .rect;
                resulting_command = layout_command;
                previous_command = Some((layout_command, rect));
            }
            fit_rect_to_work_area(rect, screen.work_area)
        };

        let applied_rect = match self.platform.set_window_rect(window.id, requested_rect) {
            Ok(rect) => rect,
            Err(error) => {
                if matches!(error, PlatformError::NotFound(_)) {
                    self.forget_window(window.id);
                }
                return Err(CommandExecutionError::Platform(error));
            }
        };

        if invocation.command == Command::Restore {
            self.forget_window(window.id);
        } else if self.history.restore_rect(window.id).is_none() {
            self.history.set_restore_rect(window.id, window.rect);
        }

        if invocation.command != Command::Restore {
            self.history.record_applied_command(
                window.id,
                resulting_command,
                requested_rect,
                applied_rect,
            );
        }

        Ok(CommandExecution {
            invocation,
            window_id: window.id,
            screen_id: screen.id,
            previous_rect: window.rect,
            requested_rect,
            applied_rect,
        })
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

    fn refresh_history_context(&mut self, window: &WindowInfo, screens: &[ScreenInfo]) {
        let identity = WindowIdentity::from(window);
        if self
            .window_identities
            .insert(window.id, identity.clone())
            .is_some_and(|previous| previous != identity)
        {
            self.history.clear_window(window.id);
        }
        self.touch_window(window.id);

        let configuration = screen_configuration(screens);
        if self
            .screen_configuration
            .replace(configuration.clone())
            .is_some_and(|previous| !screen_configurations_match(&previous, &configuration))
        {
            self.history.clear();
        }
    }

    fn touch_window(&mut self, window_id: WindowId) {
        self.remove_recent_window(window_id);
        self.recent_windows.push_back(window_id);

        while self.recent_windows.len() > MAX_TRACKED_WINDOWS {
            if let Some(evicted) = self.recent_windows.pop_front() {
                self.window_identities.remove(&evicted);
                self.history.clear_window(evicted);
                self.platform.forget_window(evicted);
            }
        }
    }

    fn forget_window(&mut self, window_id: WindowId) {
        self.history.clear_window(window_id);
        self.window_identities.remove(&window_id);
        self.remove_recent_window(window_id);
        self.platform.forget_window(window_id);
    }

    fn remove_recent_window(&mut self, window_id: WindowId) {
        if let Some(index) = self
            .recent_windows
            .iter()
            .position(|candidate| *candidate == window_id)
        {
            self.recent_windows.remove(index);
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct WindowIdentity {
    app_id: String,
    title: String,
}

impl From<&WindowInfo> for WindowIdentity {
    fn from(window: &WindowInfo) -> Self {
        Self {
            app_id: window.app_id.clone(),
            title: window.title.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ScreenGeometry {
    id: ScreenId,
    frame: Rect,
    work_area: Rect,
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
    InvalidScreenGeometry {
        screen_id: ScreenId,
    },
    NoTargetScreen {
        window_id: WindowId,
    },
    NoRestoreRect {
        window_id: WindowId,
    },
    StaleWindowHistory {
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
            Self::InvalidScreenGeometry { screen_id } => {
                write!(formatter, "invalid geometry for screen {screen_id:?}")
            }
            Self::NoTargetScreen { window_id } => {
                write!(formatter, "no target screen for window {window_id:?}")
            }
            Self::NoRestoreRect { window_id } => {
                write!(formatter, "no restore rect for window {window_id:?}")
            }
            Self::StaleWindowHistory { window_id } => {
                write!(formatter, "stale history for window {window_id:?}")
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
    Fullscreen,
    InvalidGeometry,
    NotResizable,
}

fn validate_window(window: &WindowInfo) -> CommandExecutionResult<()> {
    let reason = if window.is_hidden {
        Some(UnsupportedWindowReason::Hidden)
    } else if window.is_minimized {
        Some(UnsupportedWindowReason::Minimized)
    } else if window.is_fullscreen {
        Some(UnsupportedWindowReason::Fullscreen)
    } else if !window.rect.is_valid() {
        Some(UnsupportedWindowReason::InvalidGeometry)
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
        .min_by(|(left_screen, left_area), (right_screen, right_area)| {
            right_area
                .total_cmp(left_area)
                .then_with(|| left_screen.id.0.cmp(&right_screen.id.0))
        })
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

fn fit_rect_to_work_area(rect: Rect, work_area: Rect) -> Rect {
    let width = rect.size.width.min(work_area.size.width);
    let height = rect.size.height.min(work_area.size.height);
    let x = rect.origin.x.clamp(
        work_area.min_x(),
        work_area.origin.x + (work_area.size.width - width),
    );
    let y = rect.origin.y.clamp(
        work_area.min_y(),
        work_area.origin.y + (work_area.size.height - height),
    );

    Rect::new(x, y, width, height)
}

fn screen_configuration(screens: &[ScreenInfo]) -> Vec<ScreenGeometry> {
    let mut configuration: Vec<_> = screens
        .iter()
        .map(|screen| ScreenGeometry {
            id: screen.id,
            frame: screen.frame,
            work_area: screen.work_area,
        })
        .collect();
    configuration.sort_by_key(|screen| screen.id.0);
    configuration
}

fn screen_configurations_match(left: &[ScreenGeometry], right: &[ScreenGeometry]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.id == right.id
                && geometry_rects_match(left.frame, right.frame)
                && geometry_rects_match(left.work_area, right.work_area)
        })
}

fn geometry_rects_match(left: Rect, right: Rect) -> bool {
    const TOLERANCE: f64 = 0.1;

    (left.origin.x - right.origin.x).abs() <= TOLERANCE
        && (left.origin.y - right.origin.y).abs() <= TOLERANCE
        && (left.size.width - right.size.width).abs() <= TOLERANCE
        && (left.size.height - right.size.height).abs() <= TOLERANCE
}

fn rect_contains_with_tolerance(container: Rect, candidate: Rect) -> bool {
    const TOLERANCE: f64 = 0.1;

    candidate.min_x() >= container.min_x() - TOLERANCE
        && candidate.min_y() >= container.min_y() - TOLERANCE
        && candidate.max_x() <= container.max_x() + TOLERANCE
        && candidate.max_y() <= container.max_y() + TOLERANCE
}

fn navigates_between_screens(command: Command) -> bool {
    matches!(
        command,
        Command::LeftHalf
            | Command::RightHalf
            | Command::MoveLeft
            | Command::MoveRight
            | Command::MoveUp
            | Command::MoveDown
    )
}

fn repeats_command(previous: Option<(Command, Rect)>, rect: Rect, command: Command) -> bool {
    previous.is_some_and(|(previous_command, previous_rect)| {
        previous_command == command && rects_match(previous_rect, rect)
    })
}

fn destination_edge_command(command: Command) -> Command {
    match command {
        Command::LeftHalf => Command::RightHalf,
        Command::RightHalf => Command::LeftHalf,
        Command::MoveLeft => Command::MoveRight,
        Command::MoveRight => Command::MoveLeft,
        Command::MoveUp => Command::MoveDown,
        Command::MoveDown => Command::MoveUp,
        _ => unreachable!("only display-navigation commands reach this function"),
    }
}

fn rects_match(left: Rect, right: Rect) -> bool {
    const TOLERANCE: f64 = 1.0;

    (left.origin.x - right.origin.x).abs() <= TOLERANCE
        && (left.origin.y - right.origin.y).abs() <= TOLERANCE
        && (left.size.width - right.size.width).abs() <= TOLERANCE
        && (left.size.height - right.size.height).abs() <= TOLERANCE
}

fn adjacent_screen<'a>(
    current: &ScreenInfo,
    screens: &'a [ScreenInfo],
    command: Command,
) -> Option<&'a ScreenInfo> {
    screens
        .iter()
        .filter_map(|candidate| {
            let is_in_direction = match command {
                Command::LeftHalf | Command::MoveLeft => {
                    candidate.frame.max_x() <= current.frame.min_x()
                }
                Command::RightHalf | Command::MoveRight => {
                    candidate.frame.min_x() >= current.frame.max_x()
                }
                Command::MoveUp => candidate.frame.min_y() >= current.frame.max_y(),
                Command::MoveDown => candidate.frame.max_y() <= current.frame.min_y(),
                _ => return None,
            };
            let cross_axis_overlap = cross_axis_overlap(current, candidate, command);

            if !is_in_direction || cross_axis_overlap == 0.0 {
                return None;
            }

            let gap = match command {
                Command::LeftHalf | Command::MoveLeft => {
                    current.frame.min_x() - candidate.frame.max_x()
                }
                Command::RightHalf | Command::MoveRight => {
                    candidate.frame.min_x() - current.frame.max_x()
                }
                Command::MoveUp => candidate.frame.min_y() - current.frame.max_y(),
                Command::MoveDown => current.frame.min_y() - candidate.frame.max_y(),
                _ => unreachable!("only display-navigation commands reach this function"),
            };

            Some((candidate, gap, cross_axis_overlap))
        })
        .min_by(
            |(left, left_gap, left_overlap), (right, right_gap, right_overlap)| {
                left_gap
                    .total_cmp(right_gap)
                    .then_with(|| right_overlap.total_cmp(left_overlap))
                    .then_with(|| left.id.0.cmp(&right.id.0))
            },
        )
        .map(|(screen, _, _)| screen)
}

fn next_screen<'a>(
    current: &ScreenInfo,
    screens: &'a [ScreenInfo],
    command: Command,
) -> Option<&'a ScreenInfo> {
    adjacent_screen(current, screens, command)
        .or_else(|| wrapping_screen(current, screens, command))
}

fn wrapping_screen<'a>(
    current: &ScreenInfo,
    screens: &'a [ScreenInfo],
    command: Command,
) -> Option<&'a ScreenInfo> {
    screens
        .iter()
        .filter(|candidate| candidate.id != current.id)
        .map(|candidate| {
            (
                candidate,
                wrap_boundary(candidate, command),
                cross_axis_overlap(current, candidate, command),
            )
        })
        .min_by(
            |(left, left_boundary, left_overlap), (right, right_boundary, right_overlap)| {
                let boundary_order = match command {
                    Command::RightHalf | Command::MoveRight | Command::MoveUp => {
                        left_boundary.total_cmp(right_boundary)
                    }
                    Command::LeftHalf | Command::MoveLeft | Command::MoveDown => {
                        right_boundary.total_cmp(left_boundary)
                    }
                    _ => unreachable!("only display-navigation commands reach this function"),
                };

                boundary_order
                    .then_with(|| right_overlap.total_cmp(left_overlap))
                    .then_with(|| left.id.0.cmp(&right.id.0))
            },
        )
        .map(|(screen, _, _)| screen)
}

fn cross_axis_overlap(current: &ScreenInfo, candidate: &ScreenInfo, command: Command) -> f64 {
    match command {
        Command::LeftHalf | Command::RightHalf | Command::MoveLeft | Command::MoveRight => {
            (candidate.frame.max_y().min(current.frame.max_y())
                - candidate.frame.min_y().max(current.frame.min_y()))
            .max(0.0)
        }
        Command::MoveUp | Command::MoveDown => (candidate.frame.max_x().min(current.frame.max_x())
            - candidate.frame.min_x().max(current.frame.min_x()))
        .max(0.0),
        _ => unreachable!("only display-navigation commands reach this function"),
    }
}

fn wrap_boundary(screen: &ScreenInfo, command: Command) -> f64 {
    match command {
        Command::RightHalf | Command::MoveRight => screen.frame.min_x(),
        Command::LeftHalf | Command::MoveLeft => screen.frame.max_x(),
        Command::MoveUp => screen.frame.min_y(),
        Command::MoveDown => screen.frame.max_y(),
        _ => unreachable!("only display-navigation commands reach this function"),
    }
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
        forgotten_windows: RefCell<Vec<WindowId>>,
    }

    impl FakePlatform {
        fn new() -> Self {
            Self {
                cursor_position: Ok(Point::new(100.0, 100.0)),
                screens: Ok(vec![screen(1, 0.0), screen(2, 1000.0)]),
                front_window: Ok(Some(window(Rect::new(100.0, 100.0, 200.0, 100.0)))),
                set_result: RefCell::new(None),
                set_calls: RefCell::new(Vec::new()),
                forgotten_windows: RefCell::new(Vec::new()),
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

        fn forget_window(&self, window_id: WindowId) {
            self.forgotten_windows.borrow_mut().push(window_id);
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
    fn equal_screen_overlap_uses_stable_id_tiebreaker() {
        let screens = vec![screen(2, 1000.0), screen(1, 0.0)];
        let spanning = Rect::new(750.0, 100.0, 500.0, 200.0);

        let selected = screen_with_largest_window_overlap(spanning, &screens).unwrap();

        assert_eq!(selected.id, ScreenId(1));
    }

    #[test]
    fn subpixel_work_area_fitting_does_not_panic() {
        let work_area = Rect::new(10.0, 20.0, 0.5, 0.25);

        let fitted = fit_rect_to_work_area(Rect::new(100.0, 100.0, 200.0, 100.0), work_area);

        assert_eq!(fitted, work_area);
    }

    #[test]
    fn repeated_right_half_enters_the_adjacent_display_at_its_left_half() {
        let mut executor = CommandExecutor::with_default_config(FakePlatform::new());

        let execution = executor
            .execute_repeated(invocation(Command::RightHalf), 2)
            .unwrap();

        assert_eq!(execution.screen_id, ScreenId(2));
        assert_eq!(
            execution.requested_rect,
            Rect::new(1000.0, 0.0, 500.0, 800.0)
        );
        assert_eq!(
            executor
                .history()
                .last_command(WindowId(42))
                .unwrap()
                .command,
            Command::LeftHalf
        );
    }

    #[test]
    fn half_navigation_advances_one_section_per_press() {
        let mut executor = CommandExecutor::with_default_config(FakePlatform::new());

        let first = executor.execute(invocation(Command::LeftHalf)).unwrap();
        executor.platform_mut().front_window = Ok(Some(window(first.applied_rect)));

        let second = executor.execute(invocation(Command::RightHalf)).unwrap();
        assert_eq!(second.screen_id, ScreenId(1));
        assert_eq!(second.requested_rect, Rect::new(500.0, 0.0, 500.0, 800.0));
        executor.platform_mut().front_window = Ok(Some(window(second.applied_rect)));

        let third = executor.execute(invocation(Command::RightHalf)).unwrap();
        assert_eq!(third.screen_id, ScreenId(2));
        assert_eq!(third.requested_rect, Rect::new(1000.0, 0.0, 500.0, 800.0));
        assert_eq!(
            executor
                .history()
                .last_command(WindowId(42))
                .unwrap()
                .command,
            Command::LeftHalf
        );
        executor.platform_mut().front_window = Ok(Some(window(third.applied_rect)));

        let fourth = executor.execute(invocation(Command::RightHalf)).unwrap();
        assert_eq!(fourth.screen_id, ScreenId(2));
        assert_eq!(fourth.requested_rect, Rect::new(1500.0, 0.0, 500.0, 800.0));
    }

    #[test]
    fn a_matching_half_without_history_stays_on_the_current_display() {
        let platform = FakePlatform {
            front_window: Ok(Some(window(Rect::new(500.0, 0.0, 500.0, 800.0)))),
            ..FakePlatform::new()
        };
        let mut executor = CommandExecutor::with_default_config(platform);

        let execution = executor.execute(invocation(Command::RightHalf)).unwrap();

        assert_eq!(execution.screen_id, ScreenId(1));
        assert_eq!(
            execution.requested_rect,
            Rect::new(500.0, 0.0, 500.0, 800.0)
        );
    }

    #[test]
    fn a_matching_left_half_without_history_stays_on_the_current_display() {
        let platform = FakePlatform {
            front_window: Ok(Some(window(Rect::new(1000.0, 0.0, 500.0, 800.0)))),
            ..FakePlatform::new()
        };
        let mut executor = CommandExecutor::with_default_config(platform);

        let execution = executor.execute(invocation(Command::LeftHalf)).unwrap();

        assert_eq!(execution.screen_id, ScreenId(2));
        assert_eq!(
            execution.requested_rect,
            Rect::new(1000.0, 0.0, 500.0, 800.0)
        );
    }

    #[test]
    fn repeated_move_right_enters_the_adjacent_display_at_its_left_edge() {
        let mut executor = CommandExecutor::with_default_config(FakePlatform::new());

        let execution = executor
            .execute_repeated(invocation(Command::MoveRight), 2)
            .unwrap();

        assert_eq!(execution.screen_id, ScreenId(2));
        assert_eq!(
            execution.requested_rect,
            Rect::new(1000.0, 100.0, 200.0, 100.0)
        );
        assert_eq!(
            executor
                .history()
                .last_command(WindowId(42))
                .unwrap()
                .command,
            Command::MoveLeft
        );
    }

    #[test]
    fn repeated_move_up_enters_an_adjacent_display_at_its_bottom_edge() {
        let platform = FakePlatform {
            screens: Ok(vec![
                screen_with_frame(1, Rect::new(0.0, 0.0, 1000.0, 800.0)),
                screen_with_frame(2, Rect::new(0.0, 800.0, 1000.0, 800.0)),
            ]),
            front_window: Ok(Some(window(Rect::new(100.0, 700.0, 200.0, 100.0)))),
            ..FakePlatform::new()
        };
        let mut executor = CommandExecutor::with_default_config(platform);

        let execution = executor
            .execute_repeated(invocation(Command::MoveUp), 2)
            .unwrap();

        assert_eq!(execution.screen_id, ScreenId(2));
        assert_eq!(
            execution.requested_rect,
            Rect::new(100.0, 800.0, 200.0, 100.0)
        );
    }

    #[test]
    fn outermost_display_wraps_to_the_first_display() {
        let platform = FakePlatform {
            front_window: Ok(Some(window(Rect::new(1500.0, 0.0, 500.0, 800.0)))),
            ..FakePlatform::new()
        };
        let mut executor = CommandExecutor::with_default_config(platform);
        executor.history_mut().record_command(
            WindowId(42),
            Command::RightHalf,
            Rect::new(1500.0, 0.0, 500.0, 800.0),
        );

        let execution = executor.execute(invocation(Command::RightHalf)).unwrap();

        assert_eq!(execution.screen_id, ScreenId(1));
        assert_eq!(execution.requested_rect, Rect::new(0.0, 0.0, 500.0, 800.0));
    }

    #[test]
    fn move_right_wraps_from_the_outermost_display() {
        let platform = FakePlatform {
            front_window: Ok(Some(window(Rect::new(1800.0, 100.0, 200.0, 100.0)))),
            ..FakePlatform::new()
        };
        let mut executor = CommandExecutor::with_default_config(platform);
        executor.history_mut().record_command(
            WindowId(42),
            Command::MoveRight,
            Rect::new(1800.0, 100.0, 200.0, 100.0),
        );

        let execution = executor.execute(invocation(Command::MoveRight)).unwrap();

        assert_eq!(execution.screen_id, ScreenId(1));
        assert_eq!(
            execution.requested_rect,
            Rect::new(0.0, 100.0, 200.0, 100.0)
        );
    }

    #[test]
    fn matching_half_command_does_not_wrap_without_an_adjacent_display() {
        let platform = FakePlatform {
            screens: Ok(vec![screen(1, 0.0)]),
            front_window: Ok(Some(window(Rect::new(0.0, 0.0, 500.0, 800.0)))),
            ..FakePlatform::new()
        };
        let mut executor = CommandExecutor::with_default_config(platform);

        let execution = executor.execute(invocation(Command::LeftHalf)).unwrap();

        assert_eq!(execution.screen_id, ScreenId(1));
        assert_eq!(execution.requested_rect, Rect::new(0.0, 0.0, 500.0, 800.0));
    }

    #[test]
    fn adjacent_screen_prefers_the_largest_vertical_overlap_after_distance() {
        let screens = vec![
            screen_with_frame(1, Rect::new(0.0, 0.0, 1000.0, 800.0)),
            screen_with_frame(2, Rect::new(1000.0, 400.0, 1000.0, 800.0)),
            screen_with_frame(3, Rect::new(1000.0, 0.0, 1000.0, 800.0)),
        ];

        let adjacent = adjacent_screen(&screens[0], &screens, Command::RightHalf)
            .expect("a right-hand display should be selected");

        assert_eq!(adjacent.id, ScreenId(3));
    }

    #[test]
    fn adjacent_screen_ignores_displays_without_vertical_overlap() {
        let screens = vec![
            screen_with_frame(1, Rect::new(0.0, 0.0, 1000.0, 800.0)),
            screen_with_frame(2, Rect::new(1000.0, 800.0, 1000.0, 800.0)),
        ];

        assert_eq!(
            adjacent_screen(&screens[0], &screens, Command::RightHalf),
            None
        );
    }

    #[test]
    fn wrapping_screen_can_reach_an_offset_display() {
        let screens = vec![
            screen_with_frame(1, Rect::new(0.0, 0.0, 1000.0, 800.0)),
            screen_with_frame(2, Rect::new(-1000.0, 800.0, 1000.0, 800.0)),
        ];

        let wrapped = wrapping_screen(&screens[0], &screens, Command::RightHalf)
            .expect("the other display should be used as the wrap target");

        assert_eq!(wrapped.id, ScreenId(2));
    }

    #[test]
    fn restores_previous_rect_after_successful_command() {
        let mut executor = CommandExecutor::with_default_config(FakePlatform::new());

        let moved = executor.execute(invocation(Command::LeftHalf)).unwrap();
        executor.platform_mut().front_window = Ok(Some(window(moved.applied_rect)));
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

        let tiled = executor.execute(invocation(Command::LeftHalf)).unwrap();
        executor.platform_mut().front_window = Ok(Some(window(tiled.applied_rect)));
        let maximized = executor.execute(invocation(Command::Maximize)).unwrap();
        executor.platform_mut().front_window = Ok(Some(window(maximized.applied_rect)));

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
    fn reports_invalid_screen_geometry_before_layout() {
        let platform = FakePlatform {
            screens: Ok(vec![screen_with_frame(
                1,
                Rect::new(0.0, 0.0, f64::NAN, 800.0),
            )]),
            ..FakePlatform::new()
        };
        let mut executor = CommandExecutor::with_default_config(platform);

        let error = executor.execute(invocation(Command::Maximize)).unwrap_err();

        assert_eq!(
            error,
            CommandExecutionError::InvalidScreenGeometry {
                screen_id: ScreenId(1)
            }
        );
        assert!(executor.platform().set_calls.borrow().is_empty());
    }

    #[test]
    fn reports_work_area_outside_its_screen_frame() {
        let platform = FakePlatform {
            screens: Ok(vec![ScreenInfo {
                id: ScreenId(1),
                name: "Invalid Screen".to_owned(),
                frame: Rect::new(0.0, 0.0, 1000.0, 800.0),
                work_area: Rect::new(2000.0, 0.0, 1000.0, 800.0),
            }]),
            ..FakePlatform::new()
        };
        let mut executor = CommandExecutor::with_default_config(platform);

        let error = executor.execute(invocation(Command::Maximize)).unwrap_err();

        assert_eq!(
            error,
            CommandExecutionError::InvalidScreenGeometry {
                screen_id: ScreenId(1)
            }
        );
        assert!(executor.platform().set_calls.borrow().is_empty());
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
                is_hidden: true,
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
                reason: UnsupportedWindowReason::Hidden,
            }
        );
    }

    #[test]
    fn lets_the_platform_handle_windows_that_report_nonresizable() {
        let platform = FakePlatform {
            front_window: Ok(Some(WindowInfo {
                is_resizable: false,
                ..window(Rect::new(100.0, 100.0, 200.0, 100.0))
            })),
            ..FakePlatform::new()
        };
        let mut executor = CommandExecutor::with_default_config(platform);

        let execution = executor.execute(invocation(Command::Maximize)).unwrap();

        assert_eq!(execution.requested_rect, Rect::new(0.0, 0.0, 1000.0, 800.0));
    }

    #[test]
    fn repeats_a_half_command_after_an_app_enforces_a_different_size() {
        let constrained_rect = Rect::new(400.0, 0.0, 600.0, 800.0);
        let platform = FakePlatform {
            front_window: Ok(Some(window(constrained_rect))),
            ..FakePlatform::new()
        };
        let mut executor = CommandExecutor::with_default_config(platform);
        executor
            .history_mut()
            .record_command(WindowId(42), Command::RightHalf, constrained_rect);

        let execution = executor.execute(invocation(Command::RightHalf)).unwrap();

        assert_eq!(execution.screen_id, ScreenId(2));
        assert_eq!(
            execution.requested_rect,
            Rect::new(1000.0, 0.0, 500.0, 800.0)
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

    #[test]
    fn reused_window_id_cannot_restore_a_different_window() {
        let mut executor = CommandExecutor::with_default_config(FakePlatform::new());
        let moved = executor.execute(invocation(Command::LeftHalf)).unwrap();
        executor.platform_mut().front_window = Ok(Some(WindowInfo {
            app_id: "different.app".to_owned(),
            title: "Different Window".to_owned(),
            ..window(moved.applied_rect)
        }));

        let error = executor.execute(invocation(Command::Restore)).unwrap_err();

        assert_eq!(
            error,
            CommandExecutionError::NoRestoreRect {
                window_id: WindowId(42)
            }
        );
        assert_eq!(executor.platform().set_calls.borrow().len(), 1);
    }

    #[test]
    fn bounds_window_state_and_keeps_recent_restore_history() {
        let mut executor = CommandExecutor::with_default_config(FakePlatform::new());
        let original = Rect::new(100.0, 100.0, 200.0, 100.0);
        let mut last_moved = original;

        for raw_id in 0..=MAX_TRACKED_WINDOWS as u64 {
            executor.platform_mut().front_window = Ok(Some(WindowInfo {
                id: WindowId(raw_id),
                app_id: format!("app.{raw_id}"),
                title: format!("Window {raw_id}"),
                ..window(original)
            }));
            last_moved = executor
                .execute(invocation(Command::Maximize))
                .unwrap()
                .applied_rect;
        }

        assert_eq!(executor.recent_windows.len(), MAX_TRACKED_WINDOWS);
        assert_eq!(executor.window_identities.len(), MAX_TRACKED_WINDOWS);
        assert_eq!(
            executor.history().tracked_window_count(),
            MAX_TRACKED_WINDOWS
        );
        assert!(!executor.window_identities.contains_key(&WindowId(0)));
        assert_eq!(executor.history().restore_rect(WindowId(0)), None);
        assert_eq!(
            executor.platform().forgotten_windows.borrow().as_slice(),
            &[WindowId(0)]
        );

        let most_recent = WindowId(MAX_TRACKED_WINDOWS as u64);
        executor.platform_mut().front_window = Ok(Some(WindowInfo {
            id: most_recent,
            app_id: format!("app.{}", most_recent.0),
            title: format!("Window {}", most_recent.0),
            ..window(last_moved)
        }));
        let restored = executor.execute(invocation(Command::Restore)).unwrap();

        assert_eq!(restored.applied_rect, original);
        assert_eq!(executor.history().restore_rect(most_recent), None);
        assert!(!executor.window_identities.contains_key(&most_recent));
        assert!(!executor.recent_windows.contains(&most_recent));
        assert_eq!(
            executor.platform().forgotten_windows.borrow().last(),
            Some(&most_recent)
        );
    }

    #[test]
    fn restore_rejects_reused_identity_when_the_frame_does_not_match() {
        let mut executor = CommandExecutor::with_default_config(FakePlatform::new());
        executor.execute(invocation(Command::LeftHalf)).unwrap();
        executor.platform_mut().front_window =
            Ok(Some(window(Rect::new(250.0, 100.0, 300.0, 200.0))));

        let error = executor.execute(invocation(Command::Restore)).unwrap_err();

        assert_eq!(
            error,
            CommandExecutionError::StaleWindowHistory {
                window_id: WindowId(42)
            }
        );
        assert_eq!(executor.history().restore_rect(WindowId(42)), None);
        assert_eq!(executor.history().last_command(WindowId(42)), None);
        assert_eq!(executor.platform().set_calls.borrow().len(), 1);
    }

    #[test]
    fn display_geometry_changes_invalidate_restore_history() {
        let mut executor = CommandExecutor::with_default_config(FakePlatform::new());
        let moved = executor.execute(invocation(Command::LeftHalf)).unwrap();
        executor.platform_mut().front_window = Ok(Some(window(moved.applied_rect)));
        executor.platform_mut().screens = Ok(vec![screen_with_frame(
            1,
            Rect::new(0.0, 0.0, 1200.0, 900.0),
        )]);

        let error = executor.execute(invocation(Command::Restore)).unwrap_err();

        assert_eq!(
            error,
            CommandExecutionError::NoRestoreRect {
                window_id: WindowId(42)
            }
        );
        assert_eq!(executor.platform().set_calls.borrow().len(), 1);
    }

    #[test]
    fn restore_rect_is_fitted_inside_the_current_work_area() {
        let original = Rect::new(-100.0, -50.0, 300.0, 200.0);
        let platform = FakePlatform {
            front_window: Ok(Some(window(original))),
            ..FakePlatform::new()
        };
        let mut executor = CommandExecutor::with_default_config(platform);
        let moved = executor.execute(invocation(Command::Maximize)).unwrap();
        executor.platform_mut().front_window = Ok(Some(window(moved.applied_rect)));

        let restored = executor.execute(invocation(Command::Restore)).unwrap();

        assert_eq!(restored.requested_rect, Rect::new(0.0, 0.0, 300.0, 200.0));
    }

    #[test]
    fn repeated_resize_uses_logical_rect_instead_of_native_rounding() {
        let logical = Rect::new(84.5, 84.5, 231.0, 131.0);
        let applied = Rect::new(85.0, 85.0, 231.0, 131.0);
        let platform = FakePlatform {
            front_window: Ok(Some(window(applied))),
            ..FakePlatform::new()
        };
        let mut executor = CommandExecutor::with_default_config(platform);
        executor.history_mut().record_applied_command(
            WindowId(42),
            Command::Grow,
            logical,
            applied,
        );

        let execution = executor.execute(invocation(Command::Grow)).unwrap();

        assert_eq!(
            execution.requested_rect,
            Rect::new(69.5, 69.5, 261.0, 161.0)
        );
    }

    #[test]
    fn screen_enumeration_order_does_not_invalidate_history() {
        let mut executor = CommandExecutor::with_default_config(FakePlatform::new());
        let moved = executor.execute(invocation(Command::LeftHalf)).unwrap();
        executor.platform_mut().front_window = Ok(Some(window(moved.applied_rect)));
        executor.platform_mut().screens = Ok(vec![screen(2, 1000.0), screen(1, 0.0)]);

        let restored = executor.execute(invocation(Command::Restore)).unwrap();

        assert_eq!(
            restored.requested_rect,
            Rect::new(100.0, 100.0, 200.0, 100.0)
        );
    }

    fn invocation(command: Command) -> CommandInvocation {
        CommandInvocation {
            command,
            source: CommandSource::Keyboard,
        }
    }

    fn screen(id: u64, x: f64) -> ScreenInfo {
        screen_with_frame(id, Rect::new(x, 0.0, 1000.0, 800.0))
    }

    fn screen_with_frame(id: u64, frame: Rect) -> ScreenInfo {
        ScreenInfo {
            id: ScreenId(id),
            name: format!("Screen {id}"),
            frame,
            work_area: frame,
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
            is_fullscreen: false,
        }
    }
}
