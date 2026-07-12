use std::{fmt::Display, mem::size_of, sync::Once};

use panes_core::{Point, Rect, WindowId};
use panes_platform::{
    HotkeyBinding, MenuEntry, NativePlatform, PlatformError, PlatformResult, ScreenId, ScreenInfo,
    WindowInfo,
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, POINT, RECT},
        Graphics::Gdi::{
            EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
        },
        UI::{
            HiDpi::{
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE,
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
            },
            WindowsAndMessaging::{
                GWL_EXSTYLE, GWL_STYLE, GetCursorPos, GetDesktopWindow, GetForegroundWindow,
                GetShellWindow, GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW,
                GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible,
                IsZoomed, SW_RESTORE, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER,
                SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE, WS_CHILD,
                WS_EX_TOOLWINDOW, WS_THICKFRAME,
            },
        },
    },
    core::BOOL,
};

use crate::coordinates::{rect_from_edges, rounded_i32};

#[derive(Debug, Default)]
pub struct WindowsPlatform;

impl WindowsPlatform {
    #[must_use]
    pub fn new() -> Self {
        enable_per_monitor_dpi_awareness();
        Self
    }
}

impl NativePlatform for WindowsPlatform {
    fn platform_name(&self) -> &'static str {
        "windows"
    }

    fn cursor_position(&self) -> PlatformResult<Point> {
        let mut point = POINT::default();
        // SAFETY: `point` points to initialized writable storage for the documented Win32 call.
        unsafe { GetCursorPos(&mut point) }
            .map_err(|error| native_error("failed to read Windows cursor position", error))?;

        Ok(Point::new(f64::from(point.x), f64::from(point.y)))
    }

    fn screens(&self) -> PlatformResult<Vec<ScreenInfo>> {
        let mut collector = MonitorCollector::default();
        // SAFETY: the callback receives the valid pointer to `collector` for the duration of this
        // synchronous enumeration, and does not retain it.
        let result = unsafe {
            EnumDisplayMonitors(
                None,
                None,
                Some(collect_monitor),
                LPARAM(&mut collector as *mut MonitorCollector as isize),
            )
        };

        if let Some(error) = collector.error {
            return Err(error);
        }
        if !result.as_bool() {
            return Err(native_error(
                "failed to enumerate Windows monitors",
                windows::core::Error::from_win32(),
            ));
        }

        if collector.screens.is_empty() {
            return Err(PlatformError::NotFound("no Windows monitors found"));
        }

        Ok(collector.screens)
    }

    fn front_window(&self) -> PlatformResult<Option<WindowInfo>> {
        // SAFETY: this reads the system foreground window without dereferencing application memory.
        let window = unsafe { GetForegroundWindow() };
        if window.0.is_null() || is_shell_or_tool_window(window) {
            return Ok(None);
        }

        window_info(window).map(Some)
    }

    fn set_window_rect(&self, window_id: WindowId, rect: Rect) -> PlatformResult<Rect> {
        let window = window_from_id(window_id)?;
        // SAFETY: `window` was reconstructed from an opaque HWND and is checked before use.
        if !unsafe { IsWindow(Some(window)).as_bool() } {
            return Err(PlatformError::NotFound("Windows window no longer exists"));
        }
        if is_shell_or_tool_window(window) {
            return Err(PlatformError::Unsupported(
                "Windows desktop, shell, child, and tool windows cannot be moved",
            ));
        }
        if !unsafe { IsWindowVisible(window).as_bool() } {
            return Err(PlatformError::Unsupported(
                "hidden Windows windows cannot be moved",
            ));
        }
        if unsafe { IsIconic(window).as_bool() } {
            return Err(PlatformError::Unsupported(
                "minimized Windows windows cannot be moved",
            ));
        }
        if window_style(window) & WS_THICKFRAME != WS_THICKFRAME {
            return Err(PlatformError::Unsupported(
                "non-resizable Windows windows cannot be moved",
            ));
        }

        let (x, y, width, height) = native_rect(rect)?;
        // A maximized window ignores ordinary sizing. Restore it first so the requested frame
        // becomes the normal placement rect instead of an invisible restore target.
        // SAFETY: `window` was validated immediately above.
        if unsafe { IsZoomed(window).as_bool() } {
            // ShowWindow's return value reports the prior visibility state, not success.
            let _ = unsafe { ShowWindow(window, SW_RESTORE) };
        }

        // SAFETY: `window` is valid and the integer coordinates were range checked above.
        unsafe {
            SetWindowPos(
                window,
                None,
                x,
                y,
                width,
                height,
                SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOZORDER,
            )
        }
        .map_err(|error| native_error("failed to move or resize Windows window", error))?;

        window_rect(window)
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

#[derive(Default)]
struct MonitorCollector {
    screens: Vec<ScreenInfo>,
    error: Option<PlatformError>,
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _device_context: HDC,
    _clip_rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    // SAFETY: `data` is created from a unique mutable reference in `screens` and the callback is
    // invoked synchronously by EnumDisplayMonitors.
    let collector = unsafe { &mut *(data.0 as *mut MonitorCollector) };
    match screen_info(monitor) {
        Ok(screen) => {
            collector.screens.push(screen);
            true.into()
        }
        Err(error) => {
            collector.error = Some(error);
            false.into()
        }
    }
}

fn screen_info(monitor: HMONITOR) -> PlatformResult<ScreenInfo> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: `info` has the required cbSize and is valid writable storage for GetMonitorInfoW.
    if !unsafe {
        GetMonitorInfoW(
            monitor,
            &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
        )
    }
    .as_bool()
    {
        return Err(native_error(
            "failed to read Windows monitor information",
            windows::core::Error::from_win32(),
        ));
    }

    let device_name_end = info
        .szDevice
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(info.szDevice.len());
    let name = String::from_utf16_lossy(&info.szDevice[..device_name_end]);

    Ok(ScreenInfo {
        id: ScreenId(monitor.0 as usize as u64),
        name,
        frame: rect_from_win32(info.monitorInfo.rcMonitor),
        work_area: rect_from_win32(info.monitorInfo.rcWork),
    })
}

fn window_info(window: HWND) -> PlatformResult<WindowInfo> {
    let style = window_style(window);
    let mut process_id = 0;
    // SAFETY: `window` is returned by GetForegroundWindow and `process_id` is writable storage.
    unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };

    Ok(WindowInfo {
        id: WindowId(window.0 as usize as u64),
        app_id: format!("pid:{process_id}"),
        title: window_title(window),
        rect: window_rect(window)?,
        is_resizable: style & WS_THICKFRAME == WS_THICKFRAME,
        // The runtime turns these states into clear unsupported-window errors rather than trying
        // to move a window the operating system or application has made unavailable.
        is_minimized: unsafe { IsIconic(window).as_bool() },
        is_hidden: !unsafe { IsWindowVisible(window).as_bool() },
    })
}

fn is_shell_or_tool_window(window: HWND) -> bool {
    // SAFETY: these are system-owned HWND comparisons and style lookups for a foreground handle.
    unsafe {
        window == GetDesktopWindow()
            || window == GetShellWindow()
            || window_style(window) & WS_CHILD == WS_CHILD
            || window_ex_style(window) & WS_EX_TOOLWINDOW == WS_EX_TOOLWINDOW
    }
}

fn window_rect(window: HWND) -> PlatformResult<Rect> {
    let mut rect = RECT::default();
    // SAFETY: `rect` is valid writable storage and the caller supplies a valid HWND.
    unsafe { GetWindowRect(window, &mut rect) }
        .map_err(|error| native_error("failed to read Windows window rectangle", error))?;
    Ok(rect_from_win32(rect))
}

fn window_title(window: HWND) -> String {
    // SAFETY: GetWindowTextLengthW reads only system-managed metadata for this HWND.
    let length = unsafe { GetWindowTextLengthW(window) }.max(0) as usize;
    let mut buffer = vec![0_u16; length + 1];
    // SAFETY: `buffer` has enough writable UTF-16 code units including a terminator.
    let copied = unsafe { GetWindowTextW(window, &mut buffer) }.max(0) as usize;
    String::from_utf16_lossy(&buffer[..copied.min(length)])
}

fn window_style(window: HWND) -> WINDOW_STYLE {
    // SAFETY: this reads the documented style word for an HWND obtained from Windows.
    WINDOW_STYLE(unsafe { GetWindowLongPtrW(window, GWL_STYLE) } as u32)
}

fn window_ex_style(window: HWND) -> WINDOW_EX_STYLE {
    // SAFETY: this reads the documented extended style word for an HWND obtained from Windows.
    WINDOW_EX_STYLE(unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) } as u32)
}

fn rect_from_win32(rect: RECT) -> Rect {
    rect_from_edges(rect.left, rect.top, rect.right, rect.bottom)
}

fn native_rect(rect: Rect) -> PlatformResult<(i32, i32, i32, i32)> {
    let values = [
        rect.origin.x,
        rect.origin.y,
        rect.size.width,
        rect.size.height,
    ];
    let [Some(x), Some(y), Some(width), Some(height)] = values.map(rounded_i32) else {
        return Err(PlatformError::Native(
            "Windows window rectangle is outside the native coordinate range".to_owned(),
        ));
    };

    if width <= 0 || height <= 0 {
        return Err(PlatformError::Native(
            "Windows window rectangle must have positive dimensions".to_owned(),
        ));
    }

    Ok((x, y, width, height))
}

fn window_from_id(id: WindowId) -> PlatformResult<HWND> {
    let raw = usize::try_from(id.0).map_err(|_| {
        PlatformError::NotFound("Windows window identifier is not valid on this architecture")
    })?;
    Ok(HWND(raw as *mut _))
}

fn native_error(context: &str, error: impl Display) -> PlatformError {
    PlatformError::Native(format!("{context}: {error}"))
}

fn enable_per_monitor_dpi_awareness() {
    static ENABLE_DPI_AWARENESS: Once = Once::new();
    ENABLE_DPI_AWARENESS.call_once(|| {
        // The process may already have a DPI context (for example when panes is hosted by
        // another GUI runtime). That failure is harmless: keep the existing context.
        // SAFETY: both calls only configure the current process before any panes windows exist.
        unsafe {
            if SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).is_err() {
                let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE);
            }
        }
    });
}
