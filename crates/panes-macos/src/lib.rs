use panes_core::{Point, Rect, WindowId};
use panes_platform::{
    HotkeyBinding, MenuEntry, NativePlatform, PlatformError, PlatformResult, ScreenInfo, WindowInfo,
};

#[derive(Debug, Default)]
pub struct MacOsPlatform;

impl MacOsPlatform {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl NativePlatform for MacOsPlatform {
    fn platform_name(&self) -> &'static str {
        "macos"
    }

    fn cursor_position(&self) -> PlatformResult<Point> {
        Err(PlatformError::Unsupported(
            "macOS cursor integration is not implemented yet",
        ))
    }

    fn screens(&self) -> PlatformResult<Vec<ScreenInfo>> {
        Err(PlatformError::Unsupported(
            "macOS screen integration is not implemented yet",
        ))
    }

    fn front_window(&self) -> PlatformResult<Option<WindowInfo>> {
        Err(PlatformError::Unsupported(
            "macOS accessibility integration is not implemented yet",
        ))
    }

    fn set_window_rect(&self, _window_id: WindowId, _rect: Rect) -> PlatformResult<Rect> {
        Err(PlatformError::Unsupported(
            "macOS window movement is not implemented yet",
        ))
    }

    fn register_hotkeys(&mut self, _bindings: &[HotkeyBinding]) -> PlatformResult<()> {
        Err(PlatformError::Unsupported(
            "macOS hotkeys are not implemented yet",
        ))
    }

    fn show_tray_menu(&mut self, _entries: &[MenuEntry]) -> PlatformResult<()> {
        Err(PlatformError::Unsupported(
            "macOS tray menu is not implemented yet",
        ))
    }
}
