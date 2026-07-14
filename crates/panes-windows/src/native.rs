use std::{
    cell::RefCell,
    collections::HashMap,
    fmt::Display,
    mem::size_of,
    sync::{Arc, Mutex, Once},
};

use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{HotKey, HotKeyParseError},
};
use panes_core::{Command, CommandCategory, Point, Rect, WindowId};
use panes_platform::{
    CommandInvocation, CommandSource, HotkeyBinding, MenuEntry, NativePlatform, PendingHotkeys,
    PlatformError, PlatformResult, ScreenId, ScreenInfo, WindowInfo,
};
use tao::{
    event::{Event, StartCause},
    event_loop::{ControlFlow, EventLoopBuilder},
};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{
        Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu, accelerator::Accelerator,
    },
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, POINT, RECT},
        Graphics::Gdi::{
            EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST,
            MONITORINFO, MONITORINFOEXW, MonitorFromWindow,
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
                IsZoomed, MONITORINFOF_PRIMARY, SW_RESTORE, SWP_NOACTIVATE, SWP_NOOWNERZORDER,
                SWP_NOZORDER, SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE, WS_CHILD,
                WS_EX_TOOLWINDOW, WS_THICKFRAME,
            },
        },
    },
    core::BOOL,
};

use crate::coordinates::{CoordinateSpace, rect_from_edges, rounded_i32};

const QUIT_MENU_ID: &str = "panes.quit";

pub struct WindowsPlatform {
    tray: Option<TrayState>,
    hotkeys: Option<RegisteredHotkeys>,
    coordinate_space: RefCell<Option<CoordinateSpace>>,
}

impl WindowsPlatform {
    #[must_use]
    pub fn new() -> Self {
        enable_per_monitor_dpi_awareness();
        Self {
            tray: None,
            hotkeys: None,
            coordinate_space: RefCell::new(None),
        }
    }

    fn invocation_for_menu_id(&self, menu_id: &MenuId) -> Option<CommandInvocation> {
        self.tray
            .as_ref()?
            .command_by_menu_id
            .get(menu_id.as_ref())
            .copied()
            .map(|command| CommandInvocation {
                command,
                source: CommandSource::Menu,
            })
    }

    fn invocation_for_hotkey_id(&self, hotkey_id: u32) -> Option<CommandInvocation> {
        self.hotkeys
            .as_ref()?
            .command_by_id
            .get(&hotkey_id)
            .copied()
            .map(|command| CommandInvocation {
                command,
                source: CommandSource::Keyboard,
            })
    }
}

impl Default for WindowsPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl NativePlatform for WindowsPlatform {
    fn platform_name(&self) -> &'static str {
        "windows"
    }

    fn cursor_position(&self) -> PlatformResult<Point> {
        let space = self.coordinate_space()?;
        let mut point = POINT::default();
        // SAFETY: `point` points to initialized writable storage for the documented Win32 call.
        unsafe { GetCursorPos(&mut point) }
            .map_err(|error| native_error("failed to read Windows cursor position", error))?;

        Ok(space.native_point_to_panes(Point::new(f64::from(point.x), f64::from(point.y))))
    }

    fn screens(&self) -> PlatformResult<Vec<ScreenInfo>> {
        let screens = native_screens()?;
        let space = coordinate_space_for(&screens)?;
        self.coordinate_space.replace(Some(space));

        Ok(screens
            .into_iter()
            .map(|screen| screen.into_panes_space(space))
            .collect())
    }

    fn front_window(&self) -> PlatformResult<Option<WindowInfo>> {
        // SAFETY: this reads the system foreground window without dereferencing application memory.
        let window = unsafe { GetForegroundWindow() };
        if window.0.is_null() || is_shell_or_tool_window(window) {
            return Ok(None);
        }

        window_info(window, self.coordinate_space()?).map(Some)
    }

    fn set_window_rect(&self, window_id: WindowId, rect: Rect) -> PlatformResult<Rect> {
        let space = self.coordinate_space()?;
        let result = (|| {
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

            let (x, y, width, height) = native_rect(space.panes_rect_to_native(rect))?;
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

            window_rect(window, space)
        })();
        self.coordinate_space.take();
        result
    }

    fn register_hotkeys(&mut self, bindings: &[HotkeyBinding]) -> PlatformResult<()> {
        let manager = GlobalHotKeyManager::new()
            .map_err(|error| native_error("failed to create Windows hotkey manager", error))?;
        let mut keys = Vec::with_capacity(bindings.len());
        let mut command_by_id = HashMap::with_capacity(bindings.len());

        for binding in bindings {
            let hotkey = match parse_hotkey(binding) {
                Ok(hotkey) => hotkey,
                Err(error) => {
                    eprintln!("panes skipped a Windows hotkey: {error:?}");
                    continue;
                }
            };
            if let Err(error) = manager.register(hotkey) {
                eprintln!(
                    "panes could not register hotkey {} for {}: {error}",
                    binding.accelerator,
                    binding.command.label()
                );
                continue;
            }
            command_by_id.insert(hotkey.id(), binding.command);
            keys.push(hotkey);
        }

        self.hotkeys = Some(RegisteredHotkeys {
            _manager: manager,
            _keys: keys,
            command_by_id,
        });
        Ok(())
    }

    fn show_tray_menu(&mut self, entries: &[MenuEntry]) -> PlatformResult<()> {
        let (menu, command_by_menu_id) = build_tray_menu(entries)?;
        let tray_icon = TrayIconBuilder::new()
            .with_tooltip("panes")
            .with_icon(panes_icon()?)
            .with_menu(Box::new(menu.clone()))
            .build()
            .map_err(|error| native_error("failed to build Windows tray icon", error))?;

        self.tray = Some(TrayState {
            _tray_icon: tray_icon,
            _menu: menu,
            command_by_menu_id,
        });
        Ok(())
    }
}

impl WindowsPlatform {
    fn coordinate_space(&self) -> PlatformResult<CoordinateSpace> {
        if let Some(space) = *self.coordinate_space.borrow() {
            return Ok(space);
        }

        let space = coordinate_space()?;
        self.coordinate_space.replace(Some(space));
        Ok(space)
    }
}

/// Runs the Windows tray and global-hotkey message loop without creating an
/// application window. Each command is forwarded to `handle_command`, with
/// consecutive queued hotkeys coalesced into one native frame update.
pub fn run_keyboard_menu_app_with_handler<F>(
    menu_entries: Vec<MenuEntry>,
    hotkey_bindings: Vec<HotkeyBinding>,
    mut handle_command: F,
) -> !
where
    F: FnMut(CommandInvocation, usize) + 'static,
{
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Menu(event));
    }));

    let proxy = event_loop.create_proxy();
    let pending_hotkeys = Arc::new(Mutex::new(PendingHotkeys::default()));
    let hotkey_queue = Arc::clone(&pending_hotkeys);
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        if event.state() != HotKeyState::Pressed {
            return;
        }
        let should_wake = hotkey_queue
            .lock()
            .is_ok_and(|mut queue| queue.enqueue(event.id()));
        if should_wake {
            let _ = proxy.send_event(UserEvent::HotkeysReady);
        }
    }));

    let mut platform = WindowsPlatform::new();
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::NewEvents(StartCause::Init) => {
                if let Err(error) = platform
                    .show_tray_menu(&menu_entries)
                    .and_then(|()| platform.register_hotkeys(&hotkey_bindings))
                {
                    eprintln!("panes failed to start the Windows input runtime: {error:?}");
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::UserEvent(UserEvent::Menu(event)) => {
                if event.id() == QUIT_MENU_ID {
                    *control_flow = ControlFlow::Exit;
                    return;
                }

                if let Some(invocation) = platform.invocation_for_menu_id(event.id()) {
                    handle_command(invocation, 1);
                }
            }
            Event::UserEvent(UserEvent::HotkeysReady) => {
                let runs = pending_hotkeys
                    .lock()
                    .map(|mut queue| queue.drain())
                    .unwrap_or_default();
                for (id, repeats) in runs {
                    if let Some(invocation) = platform.invocation_for_hotkey_id(id) {
                        handle_command(invocation, repeats);
                    }
                }
            }
            _ => {}
        }
    })
}

struct TrayState {
    _tray_icon: TrayIcon,
    _menu: Menu,
    command_by_menu_id: HashMap<String, Command>,
}

struct RegisteredHotkeys {
    _manager: GlobalHotKeyManager,
    _keys: Vec<HotKey>,
    command_by_id: HashMap<u32, Command>,
}

#[derive(Debug)]
enum UserEvent {
    Menu(MenuEvent),
    HotkeysReady,
}

fn build_tray_menu(entries: &[MenuEntry]) -> PlatformResult<(Menu, HashMap<String, Command>)> {
    let menu = Menu::new();
    let mut command_by_menu_id = HashMap::with_capacity(entries.len());

    for category in CommandCategory::ALL {
        let submenu = Submenu::new(category.label(), true);
        let mut has_items = false;

        for entry in entries
            .iter()
            .filter(|entry| entry.command.category() == *category)
        {
            let menu_id = command_menu_id(entry.command);
            let accelerator = entry
                .accelerator
                .as_deref()
                .and_then(|accelerator| accelerator.parse::<Accelerator>().ok());
            let item = MenuItem::with_id(menu_id.clone(), &entry.label, true, accelerator);
            submenu.append(&item).map_err(|error| {
                native_error(
                    format!("failed to append {} menu item", entry.command.label()),
                    error,
                )
            })?;
            command_by_menu_id.insert(menu_id, entry.command);
            has_items = true;
        }

        if has_items {
            menu.append(&submenu).map_err(|error| {
                native_error(
                    format!("failed to append {} submenu", category.label()),
                    error,
                )
            })?;
        }
    }

    menu.append(&PredefinedMenuItem::separator())
        .map_err(|error| native_error("failed to append menu separator", error))?;
    menu.append(&MenuItem::with_id(QUIT_MENU_ID, "Quit Panes", true, None))
        .map_err(|error| native_error("failed to append quit menu item", error))?;

    Ok((menu, command_by_menu_id))
}

fn parse_hotkey(binding: &HotkeyBinding) -> Result<HotKey, PlatformError> {
    binding
        .accelerator
        .parse::<HotKey>()
        .map_err(|error: HotKeyParseError| {
            PlatformError::Native(format!(
                "invalid hotkey {} for {}: {error}",
                binding.accelerator,
                binding.command.label()
            ))
        })
}

fn command_menu_id(command: Command) -> String {
    format!("panes.command.{}", command.id())
}

fn panes_icon() -> PlatformResult<Icon> {
    const SIZE: usize = 32;
    let mut rgba = vec![0; SIZE * SIZE * 4];

    for y in 0..SIZE {
        for x in 0..SIZE {
            let offset = (y * SIZE + x) * 4;
            rgba[offset] = 41;
            rgba[offset + 1] = 128;
            rgba[offset + 2] = 185;
            rgba[offset + 3] = icon_alpha(x, y);
        }
    }

    Icon::from_rgba(rgba, SIZE as u32, SIZE as u32)
        .map_err(|error| native_error("failed to create tray icon", error))
}

fn icon_alpha(x: usize, y: usize) -> u8 {
    const SAMPLES_PER_AXIS: usize = 4;
    let mut covered_samples = 0;

    for sample_y in 0..SAMPLES_PER_AXIS {
        for sample_x in 0..SAMPLES_PER_AXIS {
            let point_x = x as f32 + (sample_x as f32 + 0.5) / SAMPLES_PER_AXIS as f32;
            let point_y = y as f32 + (sample_y as f32 + 0.5) / SAMPLES_PER_AXIS as f32;
            if icon_contains(point_x, point_y) {
                covered_samples += 1;
            }
        }
    }

    (covered_samples * u8::MAX as usize / (SAMPLES_PER_AXIS * SAMPLES_PER_AXIS)) as u8
}

fn icon_contains(x: f32, y: f32) -> bool {
    let outer_frame = rounded_rect_contains(x, y, 4.0, 4.0, 28.0, 28.0, 5.5)
        && !rounded_rect_contains(x, y, 7.0, 7.0, 25.0, 25.0, 2.5);
    let vertical_divider = (14.5..=17.5).contains(&x) && (7.0..=25.0).contains(&y);
    let horizontal_divider = (14.5..=17.5).contains(&y) && (7.0..=25.0).contains(&x);

    outer_frame || vertical_divider || horizontal_divider
}

fn rounded_rect_contains(
    x: f32,
    y: f32,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    radius: f32,
) -> bool {
    if !(left..=right).contains(&x) || !(top..=bottom).contains(&y) {
        return false;
    }

    let nearest_x = x.clamp(left + radius, right - radius);
    let nearest_y = y.clamp(top + radius, bottom - radius);
    let horizontal_distance = x - nearest_x;
    let vertical_distance = y - nearest_y;

    horizontal_distance.mul_add(horizontal_distance, vertical_distance * vertical_distance)
        <= radius * radius
}

#[derive(Default)]
struct MonitorCollector {
    screens: Vec<NativeScreenInfo>,
    error: Option<PlatformError>,
}

struct NativeScreenInfo {
    id: ScreenId,
    name: String,
    frame: Rect,
    work_area: Rect,
    is_primary: bool,
}

impl NativeScreenInfo {
    fn into_panes_space(self, space: CoordinateSpace) -> ScreenInfo {
        ScreenInfo {
            id: self.id,
            name: self.name,
            frame: space.native_rect_to_panes(self.frame),
            work_area: space.native_rect_to_panes(self.work_area),
        }
    }
}

fn native_screens() -> PlatformResult<Vec<NativeScreenInfo>> {
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

fn coordinate_space() -> PlatformResult<CoordinateSpace> {
    let screens = native_screens()?;
    coordinate_space_for(&screens)
}

fn coordinate_space_for(screens: &[NativeScreenInfo]) -> PlatformResult<CoordinateSpace> {
    let primary = screens
        .iter()
        .find(|screen| screen.is_primary)
        .ok_or(PlatformError::NotFound("Windows primary monitor not found"))?;

    Ok(CoordinateSpace::from_primary_frame(primary.frame))
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
    match native_screen_info(monitor) {
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

fn native_screen_info(monitor: HMONITOR) -> PlatformResult<NativeScreenInfo> {
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

    Ok(NativeScreenInfo {
        id: ScreenId(monitor.0 as usize as u64),
        name,
        frame: rect_from_win32(info.monitorInfo.rcMonitor),
        work_area: rect_from_win32(info.monitorInfo.rcWork),
        is_primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
    })
}

fn window_info(window: HWND, space: CoordinateSpace) -> PlatformResult<WindowInfo> {
    let style = window_style(window);
    let mut process_id = 0;
    // SAFETY: `window` is returned by GetForegroundWindow and `process_id` is writable storage.
    unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };

    Ok(WindowInfo {
        id: WindowId(window.0 as usize as u64),
        app_id: format!("pid:{process_id}"),
        title: window_title(window),
        rect: window_rect(window, space)?,
        is_resizable: style & WS_THICKFRAME == WS_THICKFRAME,
        // The runtime turns these states into clear unsupported-window errors rather than trying
        // to move a window the operating system or application has made unavailable.
        is_minimized: unsafe { IsIconic(window).as_bool() },
        is_hidden: !unsafe { IsWindowVisible(window).as_bool() },
        is_fullscreen: is_fullscreen_window(window),
    })
}

fn is_fullscreen_window(window: HWND) -> bool {
    // Maximized windows are intentionally supported: `set_window_rect` restores them before
    // applying a frame. Borderless fullscreen windows cover the monitor without being zoomed.
    if unsafe { IsZoomed(window).as_bool() } {
        return false;
    }

    let monitor = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) };
    if monitor.0.is_null() {
        return false;
    }

    let mut monitor_info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..MONITORINFO::default()
    };
    let mut window_rect = RECT::default();
    if !unsafe { GetMonitorInfoW(monitor, &raw mut monitor_info).as_bool() }
        || unsafe { GetWindowRect(window, &mut window_rect) }.is_err()
    {
        return false;
    }

    window_rect.left == monitor_info.rcMonitor.left
        && window_rect.top == monitor_info.rcMonitor.top
        && window_rect.right == monitor_info.rcMonitor.right
        && window_rect.bottom == monitor_info.rcMonitor.bottom
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

fn window_rect(window: HWND, space: CoordinateSpace) -> PlatformResult<Rect> {
    let mut rect = RECT::default();
    // SAFETY: `rect` is valid writable storage and the caller supplies a valid HWND.
    unsafe { GetWindowRect(window, &mut rect) }
        .map_err(|error| native_error("failed to read Windows window rectangle", error))?;
    Ok(space.native_rect_to_panes(rect_from_win32(rect)))
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

fn native_error(context: impl Display, error: impl Display) -> PlatformError {
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
