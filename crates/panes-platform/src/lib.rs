use std::collections::HashMap;

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
    pub title: String,
    pub rect: Rect,
    pub is_resizable: bool,
    pub is_minimized: bool,
    pub is_hidden: bool,
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
    [
        (Command::LeftHalf, "Control+Alt+ArrowLeft"),
        (Command::RightHalf, "Control+Alt+ArrowRight"),
        (Command::TopHalf, "Control+Alt+ArrowUp"),
        (Command::BottomHalf, "Control+Alt+ArrowDown"),
        (Command::TopLeft, "Control+Alt+U"),
        (Command::TopRight, "Control+Alt+I"),
        (Command::BottomLeft, "Control+Alt+J"),
        (Command::BottomRight, "Control+Alt+K"),
        (Command::FirstThird, "Control+Alt+Digit1"),
        (Command::CenterThird, "Control+Alt+Digit2"),
        (Command::LastThird, "Control+Alt+Digit3"),
        (Command::FirstTwoThirds, "Control+Alt+Digit4"),
        (Command::CenterTwoThirds, "Control+Alt+Digit5"),
        (Command::LastTwoThirds, "Control+Alt+Digit6"),
        (Command::Maximize, "Control+Alt+Enter"),
        (Command::AlmostMaximize, "Control+Alt+A"),
        (Command::MaximizeHeight, "Control+Alt+H"),
        (Command::Center, "Control+Alt+C"),
        (Command::Restore, "Control+Alt+Backspace"),
        (Command::MoveLeft, "Control+Alt+Shift+ArrowLeft"),
        (Command::MoveRight, "Control+Alt+Shift+ArrowRight"),
        (Command::MoveUp, "Control+Alt+Shift+ArrowUp"),
        (Command::MoveDown, "Control+Alt+Shift+ArrowDown"),
        (Command::Grow, "Control+Alt+Equal"),
        (Command::Shrink, "Control+Alt+Minus"),
    ]
    .into_iter()
    .map(|(command, accelerator)| HotkeyBinding {
        command,
        accelerator: accelerator.to_owned(),
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

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
        let bindings = default_hotkey_bindings();

        let mut bound = HashSet::new();
        for binding in &bindings {
            assert!(
                bound.insert(binding.command),
                "{} is bound more than once",
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
            "only Center Half should ship without a default hotkey"
        );
    }

    #[test]
    fn hotkey_accelerators_are_unique() {
        let bindings = default_hotkey_bindings();

        let mut accelerators = HashSet::new();
        for binding in &bindings {
            assert!(
                accelerators.insert(binding.accelerator.as_str().to_owned()),
                "duplicate accelerator {} for {}",
                binding.accelerator,
                binding.command.label()
            );
        }
    }
}
