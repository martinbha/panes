#![cfg(target_os = "macos")]

use std::{
    cell::RefCell,
    collections::HashMap,
    fmt::Display,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

mod accessibility_authorization;
mod coordinates;
mod screen;
mod window;

use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{HotKey, HotKeyParseError},
};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSAlert, NSApplication};
use objc2_foundation::NSString;
use panes_core::{Command, CommandCategory, Point, Rect, WindowId};
use panes_platform::{
    CommandInvocation, CommandSource, HotkeyBinding, MenuEntry, NativePlatform, PendingHotkeys,
    PlatformError, PlatformResult, ScreenInfo, WindowInfo, default_hotkey_bindings,
    default_menu_entries,
};
use tao::{
    event::{Event, StartCause},
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::macos::{ActivationPolicy, EventLoopExtMacOS},
};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{
        Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu,
        accelerator::{Accelerator, CMD_OR_CTRL, Code},
    },
};

const QUIT_MENU_ID: &str = "panes.quit";
const GUIDE_MENU_ID: &str = "panes.guide";
const ACCESSIBILITY_MENU_ID: &str = "panes.accessibility";
const ACCESSIBILITY_POLL_INTERVAL: Duration = Duration::from_secs(1);

pub struct MacOsPlatform {
    tray: Option<TrayState>,
    hotkeys: Option<RegisteredHotkeys>,
    windows: window::WindowCache,
    desktop: RefCell<Option<screen::DesktopSnapshot>>,
}

impl MacOsPlatform {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tray: None,
            hotkeys: None,
            windows: window::WindowCache::default(),
            desktop: RefCell::new(None),
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

    fn set_accessibility_trusted(&self, trusted: bool) {
        let Some(tray) = &self.tray else {
            return;
        };
        let (text, enabled) = accessibility_menu_item_state(trusted);
        tray.accessibility_item.set_text(text);
        tray.accessibility_item.set_enabled(enabled);
    }
}

impl Default for MacOsPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl NativePlatform for MacOsPlatform {
    fn platform_name(&self) -> &'static str {
        "macos"
    }

    fn cursor_position(&self) -> PlatformResult<Point> {
        let snapshot = self.desktop_snapshot()?;
        screen::cursor_position_in(snapshot.coordinate_space)
    }

    fn screens(&self) -> PlatformResult<Vec<ScreenInfo>> {
        let snapshot = screen::desktop_snapshot()?;
        let screens = snapshot.screens.clone();
        self.desktop.replace(Some(snapshot));
        Ok(screens)
    }

    fn front_window(&self) -> PlatformResult<Option<WindowInfo>> {
        let snapshot = self.desktop_snapshot()?;
        window::front_window_in(&self.windows, snapshot.coordinate_space)
    }

    fn set_window_rect(&self, window_id: WindowId, rect: Rect) -> PlatformResult<Rect> {
        let snapshot = self.desktop_snapshot()?;
        let result = window::set_window_rect_in(
            &self.windows,
            window_id,
            rect,
            snapshot.coordinate_space,
            &snapshot.screens,
        );
        self.desktop.take();
        result
    }

    fn forget_window(&self, window_id: WindowId) {
        self.windows.forget(window_id);
    }

    fn register_hotkeys(&mut self, bindings: &[HotkeyBinding]) -> PlatformResult<()> {
        let manager = GlobalHotKeyManager::new()
            .map_err(|error| native_error("failed to create macOS hotkey manager", error))?;
        let mut keys = Vec::with_capacity(bindings.len());
        let mut command_by_id = HashMap::with_capacity(bindings.len());

        for binding in bindings {
            let hotkey = match parse_hotkey(binding) {
                Ok(hotkey) => hotkey,
                Err(error) => {
                    eprintln!("panes skipped a hotkey: {error:?}");
                    continue;
                }
            };
            if let Err(error) = manager.register(hotkey) {
                eprintln!(
                    "panes skipped hotkey {} for {}: {error}",
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
        let trusted = accessibility_authorization::is_trusted();
        let (menu, command_by_menu_id, accessibility_item) = build_tray_menu(entries, trusted)?;
        let icon = panes_icon()?;
        let tray_icon = TrayIconBuilder::new()
            .with_tooltip("panes")
            .with_icon(icon)
            .with_icon_as_template(true)
            .with_menu(Box::new(menu.clone()))
            .build()
            .map_err(|error| native_error("failed to build macOS tray icon", error))?;

        self.tray = Some(TrayState {
            _tray_icon: tray_icon,
            _menu: menu,
            command_by_menu_id,
            accessibility_item,
        });
        Ok(())
    }
}

impl MacOsPlatform {
    fn desktop_snapshot(&self) -> PlatformResult<screen::DesktopSnapshot> {
        if let Some(snapshot) = self.desktop.borrow().clone() {
            return Ok(snapshot);
        }

        let snapshot = screen::desktop_snapshot()?;
        self.desktop.replace(Some(snapshot.clone()));
        Ok(snapshot)
    }
}

pub fn run_keyboard_menu_app() -> ! {
    run_keyboard_menu_app_with_handler(
        default_menu_entries(),
        default_hotkey_bindings(),
        |invocation, repeats| {
            println!(
                "received {:?} command from {:?} (x{repeats})",
                invocation.command, invocation.source
            );
        },
    )
}

/// Runs the tray-menu and hotkey event loop, calling `handle_command` with
/// each invocation and the number of identical presses it stands for. Hotkey
/// presses that arrive while an earlier command still blocks the main thread
/// are queued and drained in one batch; consecutive identical invocations
/// collapse into a single call with `repeats > 1` so the handler can apply
/// one combined frame change instead of replaying the burst.
pub fn run_keyboard_menu_app_with_handler<F>(
    menu_entries: Vec<MenuEntry>,
    hotkey_bindings: Vec<HotkeyBinding>,
    mut handle_command: F,
) -> !
where
    F: FnMut(CommandInvocation, usize) + 'static,
{
    // Run as an accessory app (like LSUIElement): panes never takes focus,
    // so the frontmost application keeps its focused window while the user
    // clicks the tray menu, and no Dock icon appears.
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);
    event_loop.set_activate_ignoring_other_apps(false);
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

    let mut platform = MacOsPlatform::new();
    let mut next_accessibility_check = None;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = next_accessibility_check.map_or(ControlFlow::Wait, ControlFlow::WaitUntil);

        match event {
            Event::NewEvents(StartCause::Init) => {
                if let Err(error) = platform
                    .show_tray_menu(&menu_entries)
                    .and_then(|()| platform.register_hotkeys(&hotkey_bindings))
                {
                    eprintln!("panes failed to start native macOS input runtime: {error:?}");
                    *control_flow = ControlFlow::Exit;
                    return;
                }

                let trusted = accessibility_authorization::is_trusted();
                platform.set_accessibility_trusted(trusted);
                if !trusted {
                    let trusted = accessibility_authorization::prompt();
                    platform.set_accessibility_trusted(trusted);
                    if !trusted {
                        let next_check = Instant::now() + ACCESSIBILITY_POLL_INTERVAL;
                        next_accessibility_check = Some(next_check);
                        *control_flow = ControlFlow::WaitUntil(next_check);
                    }
                }
            }
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                if accessibility_authorization::is_trusted() {
                    platform.set_accessibility_trusted(true);
                    next_accessibility_check = None;
                    *control_flow = ControlFlow::Wait;
                } else {
                    let next_check = Instant::now() + ACCESSIBILITY_POLL_INTERVAL;
                    next_accessibility_check = Some(next_check);
                    *control_flow = ControlFlow::WaitUntil(next_check);
                }
            }
            Event::UserEvent(UserEvent::Menu(event)) => {
                if event.id() == QUIT_MENU_ID {
                    *control_flow = ControlFlow::Exit;
                    return;
                }

                if event.id() == ACCESSIBILITY_MENU_ID {
                    let trusted = accessibility_authorization::prompt();
                    platform.set_accessibility_trusted(trusted);
                    if trusted {
                        next_accessibility_check = None;
                        *control_flow = ControlFlow::Wait;
                    } else {
                        if let Err(error) = accessibility_authorization::open_settings() {
                            eprintln!("panes could not open Accessibility settings: {error:?}");
                        }
                        let next_check = Instant::now() + ACCESSIBILITY_POLL_INTERVAL;
                        next_accessibility_check = Some(next_check);
                        *control_flow = ControlFlow::WaitUntil(next_check);
                    }
                    return;
                }

                if event.id() == GUIDE_MENU_ID {
                    show_guide(&menu_entries);
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
    accessibility_item: MenuItem,
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

fn build_tray_menu(
    entries: &[MenuEntry],
    accessibility_trusted: bool,
) -> PlatformResult<(Menu, HashMap<String, Command>, MenuItem)> {
    let menu = Menu::new();
    let mut command_by_menu_id = HashMap::with_capacity(entries.len());

    let (accessibility_text, accessibility_enabled) =
        accessibility_menu_item_state(accessibility_trusted);
    let accessibility_item = MenuItem::with_id(
        ACCESSIBILITY_MENU_ID,
        accessibility_text,
        accessibility_enabled,
        None,
    );
    menu.append(&accessibility_item).map_err(|error| {
        native_error("failed to append Accessibility permission menu item", error)
    })?;
    menu.append(&PredefinedMenuItem::separator())
        .map_err(|error| native_error("failed to append menu separator", error))?;

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

    let separator = PredefinedMenuItem::separator();
    menu.append(&separator)
        .map_err(|error| native_error("failed to append menu separator", error))?;

    let guide = MenuItem::with_id(GUIDE_MENU_ID, "Guide", true, None);
    menu.append(&guide)
        .map_err(|error| native_error("failed to append guide menu item", error))?;

    let separator = PredefinedMenuItem::separator();
    menu.append(&separator)
        .map_err(|error| native_error("failed to append menu separator", error))?;

    let quit = MenuItem::with_id(QUIT_MENU_ID, "Quit Panes", true, Some(quit_accelerator()));
    menu.append(&quit)
        .map_err(|error| native_error("failed to append quit menu item", error))?;

    Ok((menu, command_by_menu_id, accessibility_item))
}

fn quit_accelerator() -> Accelerator {
    Accelerator::new(Some(CMD_OR_CTRL), Code::KeyQ)
}

fn accessibility_menu_item_state(trusted: bool) -> (&'static str, bool) {
    if trusted {
        ("Accessibility Permission Granted", false)
    } else {
        ("Grant Accessibility Permission…", true)
    }
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

/// Shows a native alert listing every enabled action and its shortcut.
fn show_guide(entries: &[MenuEntry]) {
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("panes can only show the guide from the main thread");
        return;
    };

    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str("panes commands"));
    alert.setInformativeText(&NSString::from_str(&guide_text(entries)));

    // Bring the modal panel to the front even though panes is an accessory
    // app that never activates on its own.
    #[allow(deprecated)]
    NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    let _ = alert.runModal();
}

fn guide_text(entries: &[MenuEntry]) -> String {
    let mut text = String::new();

    for category in CommandCategory::ALL {
        let rows = entries
            .iter()
            .filter(|entry| entry.command.category() == *category)
            .collect::<Vec<_>>();
        if rows.is_empty() {
            continue;
        }

        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(category.label());
        text.push('\n');
        for entry in rows {
            let shortcut = entry
                .accelerator
                .as_deref()
                .map_or_else(|| "\u{2014}".to_owned(), pretty_accelerator);
            text.push_str(&format!("{shortcut}\u{2002}\u{2002}{}\n", entry.label));
        }
    }

    text
}

/// Formats an accelerator string like `Control+Alt+ArrowLeft` using macOS
/// keyboard symbols.
fn pretty_accelerator(accelerator: &str) -> String {
    accelerator
        .split('+')
        .map(|part| match part {
            "Control" | "Ctrl" => "\u{2303}",
            "Alt" | "Option" => "\u{2325}",
            "Shift" => "\u{21e7}",
            "Super" | "Meta" | "Command" | "Cmd" => "\u{2318}",
            "ArrowLeft" => "\u{2190}",
            "ArrowRight" => "\u{2192}",
            "ArrowUp" => "\u{2191}",
            "ArrowDown" => "\u{2193}",
            "Enter" | "Return" => "\u{23ce}",
            "Backspace" => "\u{232b}",
            "Escape" => "\u{238b}",
            "Space" => "Space",
            "Equal" => "=",
            "Minus" => "-",
            other => other
                .strip_prefix("Digit")
                .or_else(|| other.strip_prefix("Key"))
                .unwrap_or(other),
        })
        .collect()
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

fn native_error(context: impl Display, error: impl Display) -> PlatformError {
    PlatformError::Native(format!("{context}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_hotkey_bindings_all_parse() {
        for binding in default_hotkey_bindings() {
            parse_hotkey(&binding).unwrap_or_else(|error| {
                panic!(
                    "default accelerator {} for {} does not parse: {error:?}",
                    binding.accelerator,
                    binding.command.label()
                )
            });
        }
    }

    #[test]
    fn pretty_accelerator_uses_macos_symbols() {
        assert_eq!(
            pretty_accelerator("Control+Alt+ArrowLeft"),
            "\u{2303}\u{2325}\u{2190}"
        );
        assert_eq!(
            pretty_accelerator("Control+Alt+Shift+ArrowUp"),
            "\u{2303}\u{2325}\u{21e7}\u{2191}"
        );
        assert_eq!(
            pretty_accelerator("Control+Alt+Digit1"),
            "\u{2303}\u{2325}1"
        );
        assert_eq!(pretty_accelerator("Control+Alt+U"), "\u{2303}\u{2325}U");
        assert_eq!(pretty_accelerator("Control+Alt+Equal"), "\u{2303}\u{2325}=");
    }

    #[test]
    fn quit_uses_the_native_command_q_accelerator() {
        let accelerator = quit_accelerator();

        assert!(accelerator.modifiers().contains(CMD_OR_CTRL));
        assert_eq!(accelerator.key(), Code::KeyQ);
    }

    #[test]
    fn guide_text_groups_actions_by_category_with_shortcuts() {
        let entries = vec![
            MenuEntry {
                command: Command::LeftHalf,
                label: "Left Half".to_owned(),
                accelerator: Some("Control+Alt+ArrowLeft".to_owned()),
            },
            MenuEntry {
                command: Command::CenterHalf,
                label: "Center Half".to_owned(),
                accelerator: None,
            },
            MenuEntry {
                command: Command::Grow,
                label: "Grow".to_owned(),
                accelerator: Some("Control+Alt+Equal".to_owned()),
            },
        ];

        let text = guide_text(&entries);

        assert_eq!(
            text,
            "Halves\n\u{2303}\u{2325}\u{2190}\u{2002}\u{2002}Left Half\n\u{2014}\u{2002}\u{2002}Center Half\n\nResize\n\u{2303}\u{2325}=\u{2002}\u{2002}Grow\n"
        );
    }

    #[test]
    fn command_menu_ids_are_namespaced() {
        assert_eq!(
            command_menu_id(Command::LeftHalf),
            "panes.command.left-half"
        );
    }

    #[test]
    fn accessibility_menu_item_reflects_trust() {
        assert_eq!(
            accessibility_menu_item_state(false),
            ("Grant Accessibility Permission…", true)
        );
        assert_eq!(
            accessibility_menu_item_state(true),
            ("Accessibility Permission Granted", false)
        );
    }

    #[test]
    fn tray_icon_uses_antialiased_rounded_geometry() {
        assert_eq!(icon_alpha(0, 0), 0);
        assert_eq!(icon_alpha(10, 5), u8::MAX);
        assert_eq!(icon_alpha(5, 10), u8::MAX);
        assert_eq!(icon_alpha(10, 10), 0);
        assert_eq!(icon_alpha(16, 10), u8::MAX);
        assert_eq!(icon_alpha(10, 16), u8::MAX);
        assert!((1..u8::MAX).contains(&icon_alpha(5, 5)));
    }
}
