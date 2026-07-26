use std::collections::{HashMap, VecDeque};

use panes_core::{Command, Point, Rect, WindowId};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct ScreenId(pub u64);

#[derive(Debug, Clone, PartialEq)]
pub struct ScreenInfo {
    pub id: ScreenId,
    pub name: String,
    pub frame: Rect,
    pub work_area: Rect,
}

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
    use std::collections::HashSet;

    use global_hotkey::hotkey::HotKey;

    use super::*;

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
