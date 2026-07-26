//! Contracts shared by the runtime and native platform adapters.
//!
//! All [`Point`] and [`Rect`] values crossing this boundary use the panes
//! logical desktop coordinate system: the primary display's lower-left corner
//! is the origin, x increases rightward, and y increases upward. A
//! [`NativePlatform`] implementation must convert positions, screen frames,
//! work areas, and window rectangles from native coordinates on reads and
//! convert requested window rectangles back to native coordinates on writes.

use std::collections::{HashMap, VecDeque};

use panes_core::{Command, Point, Rect, WindowId};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct ScreenId(pub u64);

/// A native display expressed entirely in panes coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenInfo {
    pub id: ScreenId,
    pub name: String,
    pub frame: Rect,
    pub work_area: Rect,
}

/// A native window snapshot whose [`Self::rect`] is in panes coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowInfo {
    pub id: WindowId,
    pub app_id: String,
    /// Identifies the current lifetime of the owning application process.
    ///
    /// Unlike a process id alone, this changes when an operating-system
    /// process identifier is reused after the previous process exits.
    pub app_generation: u64,
    pub title: String,
    pub rect: Rect,
    pub is_resizable: bool,
    pub is_minimized: bool,
    pub is_hidden: bool,
    pub is_fullscreen: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CommandSource {
    Keyboard,
    Menu,
    Cli,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct CommandInvocation {
    pub command: Command,
    pub source: CommandSource,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuEntry {
    pub command: Command,
    pub label: String,
    pub accelerator: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HotkeyBinding {
    pub command: Command,
    pub accelerator: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum LaunchAtLoginStatus {
    Disabled,
    Enabled,
    RequiresApproval,
    Stale,
    Unavailable,
}

impl LaunchAtLoginStatus {
    #[must_use]
    pub const fn is_configured(self) -> bool {
        matches!(self, Self::Enabled | Self::RequiresApproval)
    }

    #[must_use]
    pub const fn has_registration(self) -> bool {
        matches!(self, Self::Enabled | Self::RequiresApproval | Self::Stale)
    }

    #[must_use]
    pub const fn is_available(self) -> bool {
        !matches!(self, Self::Unavailable)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum HotkeyPlatform {
    MacOs,
    Windows,
}

impl HotkeyPlatform {
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::MacOs
        }
    }
}

/// Maximum number of key presses retained while the native event loop is busy.
pub const MAX_PENDING_HOTKEY_PRESSES: usize = 1_024;

/// Bounded, run-length encoded hotkey input shared by native event loops.
///
/// The first accepted press schedules a wake-up. Further presses join the
/// pending batch without posting redundant events. Once the bound is reached,
/// newest presses are ignored until the batch is drained.
#[derive(Debug)]
pub struct PendingHotkeys {
    runs: VecDeque<(u32, usize)>,
    pending_presses: usize,
    max_pending_presses: usize,
    wake_scheduled: bool,
}

impl Default for PendingHotkeys {
    fn default() -> Self {
        Self::with_capacity(MAX_PENDING_HOTKEY_PRESSES)
    }
}

impl PendingHotkeys {
    #[must_use]
    pub fn with_capacity(max_pending_presses: usize) -> Self {
        Self {
            runs: VecDeque::new(),
            pending_presses: 0,
            max_pending_presses: max_pending_presses.max(1),
            wake_scheduled: false,
        }
    }

    /// Adds one press and returns whether the caller must post a wake-up.
    pub fn enqueue(&mut self, hotkey_id: u32) -> bool {
        if self.pending_presses >= self.max_pending_presses {
            return false;
        }

        match self.runs.back_mut() {
            Some((last_id, repeats)) if *last_id == hotkey_id => *repeats += 1,
            _ => self.runs.push_back((hotkey_id, 1)),
        }
        self.pending_presses += 1;

        if self.wake_scheduled {
            false
        } else {
            self.wake_scheduled = true;
            true
        }
    }

    /// Drains pending runs and permits a subsequent press to schedule a wake-up.
    pub fn drain(&mut self) -> Vec<(u32, usize)> {
        self.pending_presses = 0;
        self.wake_scheduled = false;
        self.runs.drain(..).collect()
    }

    #[must_use]
    pub const fn pending_press_count(&self) -> usize {
        self.pending_presses
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum PlatformError {
    Unsupported(&'static str),
    NotFound(&'static str),
    PermissionDenied(&'static str),
    Native(String),
}

pub type PlatformResult<T> = Result<T, PlatformError>;

/// Native desktop operations used by the platform-neutral runtime.
///
/// Every [`Point`] and [`Rect`] returned or accepted by this trait is in panes
/// coordinates, never raw platform coordinates. Implementations own both
/// directions of native conversion.
pub trait NativePlatform {
    fn platform_name(&self) -> &'static str;

    fn cursor_position(&self) -> PlatformResult<Point>;

    fn screens(&self) -> PlatformResult<Vec<ScreenInfo>>;

    fn front_window(&self) -> PlatformResult<Option<WindowInfo>>;

    fn set_window_rect(&self, window_id: WindowId, rect: Rect) -> PlatformResult<Rect>;

    /// Releases any native state retained for a window that no longer needs history.
    fn forget_window(&self, _window_id: WindowId) {}

    fn register_hotkeys(&mut self, bindings: &[HotkeyBinding]) -> PlatformResult<()>;

    fn show_tray_menu(&mut self, entries: &[MenuEntry]) -> PlatformResult<()>;

    fn launch_at_login_status(&self) -> PlatformResult<LaunchAtLoginStatus> {
        Ok(LaunchAtLoginStatus::Unavailable)
    }

    fn set_launch_at_login(&self, _enabled: bool) -> PlatformResult<()> {
        Err(PlatformError::Unsupported(
            "launch at login is unavailable on this platform",
        ))
    }
}

pub fn reconcile_launch_at_login<P: NativePlatform>(
    platform: &P,
    desired: bool,
) -> PlatformResult<LaunchAtLoginStatus> {
    let status = platform.launch_at_login_status()?;
    if !status.is_available() {
        return Ok(status);
    }

    let needs_update = if desired {
        !status.is_configured()
    } else {
        status.has_registration()
    };
    if needs_update {
        platform.set_launch_at_login(desired)?;
        platform.launch_at_login_status()
    } else {
        Ok(status)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum LaunchAtLoginUpdateError {
    Platform(PlatformError),
    StateMismatch {
        desired: bool,
        actual: LaunchAtLoginStatus,
    },
    Persist {
        message: String,
        rollback_error: Option<PlatformError>,
    },
}

impl std::fmt::Display for LaunchAtLoginUpdateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Platform(error) => write!(formatter, "platform update failed: {error:?}"),
            Self::StateMismatch { desired, actual } => write!(
                formatter,
                "platform reported {actual:?} after launch at login was set to {desired}"
            ),
            Self::Persist {
                message,
                rollback_error: None,
            } => write!(
                formatter,
                "could not save the preference; the platform update was rolled back: {message}"
            ),
            Self::Persist {
                message,
                rollback_error: Some(rollback_error),
            } => write!(
                formatter,
                "could not save the preference ({message}) or roll back the platform update: \
                 {rollback_error:?}"
            ),
        }
    }
}

impl std::error::Error for LaunchAtLoginUpdateError {}

pub fn toggle_launch_at_login<P, F>(
    platform: &P,
    persist: &mut F,
) -> Result<LaunchAtLoginStatus, LaunchAtLoginUpdateError>
where
    P: NativePlatform,
    F: FnMut(bool) -> Result<(), String>,
{
    let before = platform
        .launch_at_login_status()
        .map_err(LaunchAtLoginUpdateError::Platform)?;
    if !before.is_available() {
        return Ok(before);
    }

    let desired = !before.is_configured();
    platform
        .set_launch_at_login(desired)
        .map_err(LaunchAtLoginUpdateError::Platform)?;
    let after = platform
        .launch_at_login_status()
        .map_err(LaunchAtLoginUpdateError::Platform)?;
    let reached_desired_state = if desired {
        after.is_configured()
    } else {
        !after.has_registration()
    };
    if !reached_desired_state {
        return Err(LaunchAtLoginUpdateError::StateMismatch {
            desired,
            actual: after,
        });
    }

    if let Err(message) = persist(desired) {
        let rollback_error = platform.set_launch_at_login(before.is_configured()).err();
        return Err(LaunchAtLoginUpdateError::Persist {
            message,
            rollback_error,
        });
    }

    Ok(after)
}

/// Preserves a constrained window's actual size while aligning it within a
/// requested layout zone and the destination work area.
#[must_use]
pub fn align_constrained_rect(actual: Rect, zone: Rect, work_area: Rect) -> Rect {
    Rect::new(
        aligned_axis_origin(
            actual.size.width,
            zone.min_x(),
            zone.size.width,
            work_area.min_x(),
            work_area.size.width,
        ),
        aligned_axis_origin(
            actual.size.height,
            zone.min_y(),
            zone.size.height,
            work_area.min_y(),
            work_area.size.height,
        ),
        actual.size.width,
        actual.size.height,
    )
}

fn aligned_axis_origin(
    actual_size: f64,
    zone_origin: f64,
    zone_size: f64,
    work_area_origin: f64,
    work_area_size: f64,
) -> f64 {
    let zone_max = zone_origin + zone_size;
    let work_area_max = work_area_origin + work_area_size;
    let zone_touches_start = coordinates_match(zone_origin, work_area_origin);
    let zone_touches_end = coordinates_match(zone_max, work_area_max);
    let origin =
        if coordinates_match(actual_size, zone_size) || zone_touches_start && zone_touches_end {
            zone_origin + (zone_size - actual_size) / 2.0
        } else if zone_touches_start {
            zone_origin
        } else if zone_touches_end {
            zone_max - actual_size
        } else {
            zone_origin + (zone_size - actual_size) / 2.0
        };

    if actual_size <= work_area_size {
        origin.clamp(work_area_origin, work_area_max - actual_size)
    } else {
        work_area_origin
    }
}

fn coordinates_match(left: f64, right: f64) -> bool {
    (left - right).abs() < 0.1
}

#[must_use]
pub fn default_menu_entries() -> Vec<MenuEntry> {
    let accelerators: HashMap<Command, String> = default_hotkey_bindings()
        .into_iter()
        .map(|binding| (binding.command, binding.accelerator))
        .collect();

    panes_core::Command::ALL
        .iter()
        .copied()
        .map(|command| MenuEntry {
            command,
            label: command.label().to_owned(),
            accelerator: accelerators.get(&command).cloned(),
        })
        .collect()
}

#[must_use]
pub fn default_hotkey_bindings() -> Vec<HotkeyBinding> {
    default_hotkey_bindings_for(HotkeyPlatform::current())
}

#[must_use]
pub fn default_hotkey_bindings_for(platform: HotkeyPlatform) -> Vec<HotkeyBinding> {
    // Windows reserves shortcuts that use the Windows key, while Control+Alt
    // letter and digit chords collide with AltGr input. Keep directional
    // commands on arrows and place character-key commands on function keys.
    [
        (
            Command::LeftHalf,
            "Control+Alt+ArrowLeft",
            "Control+Alt+ArrowLeft",
        ),
        (
            Command::RightHalf,
            "Control+Alt+ArrowRight",
            "Control+Alt+ArrowRight",
        ),
        (
            Command::TopHalf,
            "Control+Alt+ArrowUp",
            "Control+Alt+ArrowUp",
        ),
        (
            Command::BottomHalf,
            "Control+Alt+ArrowDown",
            "Control+Alt+ArrowDown",
        ),
        (Command::TopLeft, "Control+Alt+U", "Control+Shift+F1"),
        (Command::TopRight, "Control+Alt+I", "Control+Shift+F2"),
        (Command::BottomLeft, "Control+Alt+J", "Control+Shift+F3"),
        (Command::BottomRight, "Control+Alt+K", "Control+Shift+F4"),
        (
            Command::FirstThird,
            "Control+Alt+Digit1",
            "Control+Shift+F5",
        ),
        (
            Command::CenterThird,
            "Control+Alt+Digit2",
            "Control+Shift+F6",
        ),
        (Command::LastThird, "Control+Alt+Digit3", "Control+Shift+F7"),
        (
            Command::FirstTwoThirds,
            "Control+Alt+Digit4",
            "Control+Shift+F8",
        ),
        (
            Command::CenterTwoThirds,
            "Control+Alt+Digit5",
            "Control+Shift+F9",
        ),
        (
            Command::LastTwoThirds,
            "Control+Alt+Digit6",
            "Control+Shift+F10",
        ),
        (Command::Maximize, "Control+Alt+Enter", "Control+Alt+Enter"),
        (
            Command::AlmostMaximize,
            "Control+Alt+A",
            "Control+Shift+F11",
        ),
        (
            Command::MaximizeHeight,
            "Control+Alt+H",
            "Control+Alt+Shift+F1",
        ),
        (Command::Center, "Control+Alt+C", "Control+Alt+Shift+F2"),
        (
            Command::Restore,
            "Control+Alt+Backspace",
            "Control+Alt+Backspace",
        ),
        (
            Command::MoveLeft,
            "Control+Alt+Shift+ArrowLeft",
            "Control+Alt+Shift+ArrowLeft",
        ),
        (
            Command::MoveRight,
            "Control+Alt+Shift+ArrowRight",
            "Control+Alt+Shift+ArrowRight",
        ),
        (
            Command::MoveUp,
            "Control+Alt+Shift+ArrowUp",
            "Control+Alt+Shift+ArrowUp",
        ),
        (
            Command::MoveDown,
            "Control+Alt+Shift+ArrowDown",
            "Control+Alt+Shift+ArrowDown",
        ),
        (Command::Grow, "Control+Alt+Equal", "Control+Alt+Shift+F3"),
        (Command::Shrink, "Control+Alt+Minus", "Control+Alt+Shift+F4"),
    ]
    .into_iter()
    .map(
        |(command, macos_accelerator, windows_accelerator)| HotkeyBinding {
            command,
            accelerator: match platform {
                HotkeyPlatform::MacOs => macos_accelerator,
                HotkeyPlatform::Windows => windows_accelerator,
            }
            .to_owned(),
        },
    )
    .collect()
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        collections::HashSet,
    };

    use global_hotkey::hotkey::HotKey;

    use super::*;

    struct LoginPlatform {
        status: Cell<LaunchAtLoginStatus>,
        updates: RefCell<Vec<bool>>,
    }

    impl LoginPlatform {
        fn new(status: LaunchAtLoginStatus) -> Self {
            Self {
                status: Cell::new(status),
                updates: RefCell::new(Vec::new()),
            }
        }
    }

    impl NativePlatform for LoginPlatform {
        fn platform_name(&self) -> &'static str {
            "test"
        }

        fn cursor_position(&self) -> PlatformResult<Point> {
            unreachable!()
        }

        fn screens(&self) -> PlatformResult<Vec<ScreenInfo>> {
            unreachable!()
        }

        fn front_window(&self) -> PlatformResult<Option<WindowInfo>> {
            unreachable!()
        }

        fn set_window_rect(&self, _window_id: WindowId, _rect: Rect) -> PlatformResult<Rect> {
            unreachable!()
        }

        fn register_hotkeys(&mut self, _bindings: &[HotkeyBinding]) -> PlatformResult<()> {
            unreachable!()
        }

        fn show_tray_menu(&mut self, _entries: &[MenuEntry]) -> PlatformResult<()> {
            unreachable!()
        }

        fn launch_at_login_status(&self) -> PlatformResult<LaunchAtLoginStatus> {
            Ok(self.status.get())
        }

        fn set_launch_at_login(&self, enabled: bool) -> PlatformResult<()> {
            self.updates.borrow_mut().push(enabled);
            self.status.set(if enabled {
                LaunchAtLoginStatus::Enabled
            } else {
                LaunchAtLoginStatus::Disabled
            });
            Ok(())
        }
    }

    #[test]
    fn login_status_distinguishes_registration_from_availability() {
        assert!(LaunchAtLoginStatus::Enabled.is_configured());
        assert!(LaunchAtLoginStatus::RequiresApproval.is_configured());
        assert!(LaunchAtLoginStatus::Stale.has_registration());
        assert!(!LaunchAtLoginStatus::Stale.is_configured());
        assert!(!LaunchAtLoginStatus::Unavailable.is_available());
    }

    #[test]
    fn login_reconciliation_repairs_stale_and_missing_registrations() {
        let stale = LoginPlatform::new(LaunchAtLoginStatus::Stale);
        assert_eq!(
            reconcile_launch_at_login(&stale, true),
            Ok(LaunchAtLoginStatus::Enabled)
        );
        assert_eq!(*stale.updates.borrow(), [true]);

        let unwanted = LoginPlatform::new(LaunchAtLoginStatus::Stale);
        assert_eq!(
            reconcile_launch_at_login(&unwanted, false),
            Ok(LaunchAtLoginStatus::Disabled)
        );
        assert_eq!(*unwanted.updates.borrow(), [false]);

        let unavailable = LoginPlatform::new(LaunchAtLoginStatus::Unavailable);
        assert_eq!(
            reconcile_launch_at_login(&unavailable, true),
            Ok(LaunchAtLoginStatus::Unavailable)
        );
        assert!(unavailable.updates.borrow().is_empty());
    }

    #[test]
    fn login_toggle_updates_native_state_and_persists_the_preference() {
        let platform = LoginPlatform::new(LaunchAtLoginStatus::Disabled);
        let mut persisted = Vec::new();

        let status = toggle_launch_at_login(&platform, &mut |enabled| {
            persisted.push(enabled);
            Ok(())
        });

        assert_eq!(status, Ok(LaunchAtLoginStatus::Enabled));
        assert_eq!(*platform.updates.borrow(), [true]);
        assert_eq!(persisted, [true]);
    }

    #[test]
    fn login_toggle_rolls_back_when_persistence_fails() {
        let platform = LoginPlatform::new(LaunchAtLoginStatus::Disabled);

        let error = toggle_launch_at_login(&platform, &mut |_| Err("disk full".to_owned()))
            .expect_err("persistence should fail");

        assert_eq!(
            error,
            LaunchAtLoginUpdateError::Persist {
                message: "disk full".to_owned(),
                rollback_error: None,
            }
        );
        assert_eq!(*platform.updates.borrow(), [true, false]);
        assert_eq!(platform.status.get(), LaunchAtLoginStatus::Disabled);
    }

    #[test]
    fn menu_entries_cover_every_command_exactly_once() {
        let entries = default_menu_entries();

        let commands: Vec<Command> = entries.iter().map(|entry| entry.command).collect();
        assert_eq!(commands, Command::ALL);

        let bindings: HashMap<Command, String> = default_hotkey_bindings()
            .into_iter()
            .map(|binding| (binding.command, binding.accelerator))
            .collect();

        for entry in &entries {
            assert_eq!(entry.label, entry.command.label());
            assert_eq!(entry.accelerator, bindings.get(&entry.command).cloned());
        }
    }

    #[test]
    fn hotkey_bindings_bind_each_command_at_most_once() {
        for platform in [HotkeyPlatform::MacOs, HotkeyPlatform::Windows] {
            let bindings = default_hotkey_bindings_for(platform);

            let mut bound = HashSet::new();
            for binding in &bindings {
                assert!(
                    bound.insert(binding.command),
                    "{} is bound more than once on {platform:?}",
                    binding.command.label()
                );
            }

            let unbound: Vec<Command> = Command::ALL
                .iter()
                .copied()
                .filter(|command| !bound.contains(command))
                .collect();
            assert_eq!(
                unbound,
                [Command::CenterHalf],
                "only Center Half should ship without a default hotkey on {platform:?}"
            );
        }
    }

    #[test]
    fn platform_hotkey_accelerators_parse_and_are_unique() {
        for platform in [HotkeyPlatform::MacOs, HotkeyPlatform::Windows] {
            let mut accelerators = HashSet::new();
            for binding in default_hotkey_bindings_for(platform) {
                let parsed = binding
                    .accelerator
                    .parse::<HotKey>()
                    .unwrap_or_else(|error| {
                        panic!(
                            "invalid {platform:?} accelerator {} for {}: {error}",
                            binding.accelerator,
                            binding.command.label()
                        )
                    });
                assert!(
                    accelerators.insert(parsed),
                    "duplicate accelerator {} for {} on {platform:?}",
                    binding.accelerator,
                    binding.command.label()
                );
            }
        }
    }

    #[test]
    fn macos_defaults_remain_control_alt_bindings() {
        for binding in default_hotkey_bindings_for(HotkeyPlatform::MacOs) {
            assert!(binding.accelerator.starts_with("Control+Alt+"));
        }
        assert_eq!(
            default_hotkey_bindings_for(HotkeyPlatform::MacOs)
                .into_iter()
                .find(|binding| binding.command == Command::MoveLeft)
                .map(|binding| binding.accelerator),
            Some("Control+Alt+Shift+ArrowLeft".to_owned())
        );
    }

    #[test]
    fn windows_defaults_avoid_altgr_typing_and_windows_key_shortcuts() {
        for binding in default_hotkey_bindings_for(HotkeyPlatform::Windows) {
            assert!(!binding.accelerator.contains("Super"));
            if matches!(
                binding.command,
                Command::TopLeft
                    | Command::TopRight
                    | Command::BottomLeft
                    | Command::BottomRight
                    | Command::FirstThird
                    | Command::CenterThird
                    | Command::LastThird
                    | Command::FirstTwoThirds
                    | Command::CenterTwoThirds
                    | Command::LastTwoThirds
                    | Command::AlmostMaximize
                    | Command::MaximizeHeight
                    | Command::Center
            ) {
                assert!(
                    binding.accelerator.contains("+F"),
                    "{} should use a function key on Windows",
                    binding.command.label()
                );
            }
        }
    }

    #[test]
    fn constrained_move_preserves_size_and_requested_position() {
        let work_area = Rect::new(0.0, 0.0, 1_000.0, 800.0);
        let actual = Rect::new(100.0, 100.0, 200.0, 100.0);
        let requested = Rect::new(800.0, 100.0, 200.0, 100.0);

        assert_eq!(
            align_constrained_rect(actual, requested, work_area),
            requested
        );
    }

    #[test]
    fn constrained_tiling_preserves_size_and_aligns_to_zone() {
        let work_area = Rect::new(0.0, 0.0, 1_000.0, 800.0);
        let actual = Rect::new(100.0, 100.0, 600.0, 400.0);
        let requested = Rect::new(500.0, 0.0, 500.0, 800.0);

        assert_eq!(
            align_constrained_rect(actual, requested, work_area),
            Rect::new(400.0, 200.0, 600.0, 400.0)
        );
    }

    #[test]
    fn oversized_constrained_window_stays_at_work_area_start() {
        let work_area = Rect::new(0.0, 0.0, 1_000.0, 800.0);
        let actual = Rect::new(-500.0, 0.0, 1_200.0, 100.0);
        let requested = Rect::new(0.0, 0.0, 500.0, 800.0);

        assert_eq!(
            align_constrained_rect(actual, requested, work_area),
            Rect::new(0.0, 350.0, 1_200.0, 100.0)
        );
    }

    #[test]
    fn pending_hotkeys_schedule_one_wake_and_preserve_mixed_order() {
        let mut pending = PendingHotkeys::default();

        assert!(pending.enqueue(1));
        assert!(!pending.enqueue(1));
        assert!(!pending.enqueue(2));
        assert!(!pending.enqueue(1));
        assert_eq!(pending.pending_press_count(), 4);
        assert_eq!(pending.drain(), [(1, 2), (2, 1), (1, 1)]);
    }

    #[test]
    fn pending_hotkeys_bound_input_and_reschedule_after_drain() {
        let mut pending = PendingHotkeys::with_capacity(3);

        assert!(pending.enqueue(1));
        assert!(!pending.enqueue(1));
        assert!(!pending.enqueue(2));
        assert!(!pending.enqueue(3));
        assert_eq!(pending.pending_press_count(), 3);
        assert_eq!(pending.drain(), [(1, 2), (2, 1)]);

        assert!(pending.enqueue(3));
        assert_eq!(pending.drain(), [(3, 1)]);
    }
}
