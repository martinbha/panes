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

#[derive(Debug, Clone, PartialEq)]
pub struct MenuEntry {
    pub command: Command,
    pub label: String,
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
    panes_core::Command::ALL
        .iter()
        .copied()
        .map(|command| MenuEntry {
            command,
            label: command.label().to_owned(),
        })
        .collect()
}
