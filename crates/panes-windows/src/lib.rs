mod coordinates;

#[cfg(target_os = "windows")]
mod native;

#[cfg(target_os = "windows")]
pub use native::{WindowsPlatform, run_keyboard_menu_app_with_handler};

// Keep the crate buildable for the workspace's non-Windows development and
// test targets. The actual adapter is compiled only where the Win32 APIs are
// available.
#[cfg(not(target_os = "windows"))]
mod unsupported {
    use panes_core::{Point, Rect, WindowId};
    use panes_platform::{
        HotkeyBinding, MenuEntry, NativePlatform, PlatformError, PlatformResult, ScreenInfo,
        WindowInfo,
    };

    #[derive(Debug, Default)]
    pub struct WindowsPlatform;

    impl WindowsPlatform {
        #[must_use]
        pub const fn new() -> Self {
            Self
        }
    }

    impl NativePlatform for WindowsPlatform {
        fn platform_name(&self) -> &'static str {
            "windows"
        }

        fn cursor_position(&self) -> PlatformResult<Point> {
            Err(PlatformError::Unsupported(
                "Windows APIs are only available on Windows",
            ))
        }

        fn screens(&self) -> PlatformResult<Vec<ScreenInfo>> {
            Err(PlatformError::Unsupported(
                "Windows APIs are only available on Windows",
            ))
        }

        fn front_window(&self) -> PlatformResult<Option<WindowInfo>> {
            Err(PlatformError::Unsupported(
                "Windows APIs are only available on Windows",
            ))
        }

        fn set_window_rect(&self, _window_id: WindowId, _rect: Rect) -> PlatformResult<Rect> {
            Err(PlatformError::Unsupported(
                "Windows APIs are only available on Windows",
            ))
        }

        fn register_hotkeys(&mut self, _bindings: &[HotkeyBinding]) -> PlatformResult<()> {
            Err(PlatformError::Unsupported(
                "Windows hotkeys are not implemented yet",
            ))
        }

        fn show_tray_menu(&mut self, _entries: &[MenuEntry]) -> PlatformResult<()> {
            Err(PlatformError::Unsupported(
                "Windows tray menu is not implemented yet",
            ))
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub use unsupported::WindowsPlatform;
