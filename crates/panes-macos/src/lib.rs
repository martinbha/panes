use std::{
    collections::{HashMap, VecDeque},
    fmt::Display,
    sync::{Arc, Mutex},
};

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
    CommandInvocation, CommandSource, HotkeyBinding, MenuEntry, NativePlatform, PlatformError,
    PlatformResult, ScreenInfo, WindowInfo, default_hotkey_bindings, default_menu_entries,
};
use tao::{
    event::{Event, StartCause},
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::macos::{ActivationPolicy, EventLoopExtMacOS},
};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{
        Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu, accelerator::Accelerator,
    },
};

const QUIT_MENU_ID: &str = "panes.quit";
const GUIDE_MENU_ID: &str = "panes.guide";

pub struct MacOsPlatform {
    tray: Option<TrayState>,
    hotkeys: Option<RegisteredHotkeys>,
    windows: window::WindowCache,
}

impl MacOsPlatform {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tray: None,
            hotkeys: None,
            windows: window::WindowCache::default(),
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
        screen::cursor_position()
    }

    fn screens(&self) -> PlatformResult<Vec<ScreenInfo>> {
        screen::screens()
    }

    fn front_window(&self) -> PlatformResult<Option<WindowInfo>> {
        window::front_window(&self.windows)
    }

    fn set_window_rect(&self, window_id: WindowId, rect: Rect) -> PlatformResult<Rect> {
        window::set_window_rect(&self.windows, window_id, rect)
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
        let (menu, command_by_menu_id) = build_tray_menu(entries)?;
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
        });
        Ok(())
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
    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Menu(event));
    }));

    let proxy = event_loop.create_proxy();
    let pending_hotkeys: Arc<Mutex<VecDeque<u32>>> = Arc::new(Mutex::new(VecDeque::new()));
    let hotkey_queue = Arc::clone(&pending_hotkeys);
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        if event.state() != HotKeyState::Pressed {
            return;
        }
        if let Ok(mut queue) = hotkey_queue.lock() {
            queue.push_back(event.id());
        }
        let _ = proxy.send_event(UserEvent::HotkeysReady);
    }));

    let mut platform = MacOsPlatform::new();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::NewEvents(StartCause::Init) => {
                if let Err(error) = platform
                    .show_tray_menu(&menu_entries)
                    .and_then(|()| platform.register_hotkeys(&hotkey_bindings))
                {
                    eprintln!("panes failed to start native macOS input runtime: {error:?}");
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::UserEvent(UserEvent::Menu(event)) => {
                if event.id() == QUIT_MENU_ID {
                    *control_flow = ControlFlow::Exit;
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
                // One wake-up per press is sent, but the first one drains the
                // whole queue, so later wake-ups usually find it empty.
                let ids: Vec<u32> = pending_hotkeys
                    .lock()
                    .map(|mut queue| queue.drain(..).collect())
                    .unwrap_or_default();
                let invocations = ids
                    .into_iter()
                    .filter_map(|id| platform.invocation_for_hotkey_id(id));

                for (invocation, repeats) in coalesce_invocations(invocations) {
                    handle_command(invocation, repeats);
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

/// Collapses consecutive identical invocations into `(invocation, repeats)`
/// runs so a burst of queued presses of the same hotkey becomes one handler
/// call — and therefore one native frame change — instead of replaying every
/// press individually.
fn coalesce_invocations(
    invocations: impl IntoIterator<Item = CommandInvocation>,
) -> Vec<(CommandInvocation, usize)> {
    let mut runs: Vec<(CommandInvocation, usize)> = Vec::new();

    for invocation in invocations {
        match runs.last_mut() {
            Some((last, repeats)) if *last == invocation => *repeats += 1,
            _ => runs.push((invocation, 1)),
        }
    }

    runs
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

    let separator = PredefinedMenuItem::separator();
    menu.append(&separator)
        .map_err(|error| native_error("failed to append menu separator", error))?;

    let guide = MenuItem::with_id(GUIDE_MENU_ID, "Guide", true, None);
    menu.append(&guide)
        .map_err(|error| native_error("failed to append guide menu item", error))?;

    let separator = PredefinedMenuItem::separator();
    menu.append(&separator)
        .map_err(|error| native_error("failed to append menu separator", error))?;

    let quit = MenuItem::with_id(QUIT_MENU_ID, "Quit Panes", true, None);
    menu.append(&quit)
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
            if is_icon_stroke(x, y) {
                let offset = (y * SIZE + x) * 4;
                rgba[offset + 3] = 255;
            }
        }
    }

    Icon::from_rgba(rgba, SIZE as u32, SIZE as u32)
        .map_err(|error| native_error("failed to create tray icon", error))
}

fn is_icon_stroke(x: usize, y: usize) -> bool {
    let in_bounds = (5..=26).contains(&x) && (5..=26).contains(&y);
    let outer = x == 5 || x == 26 || y == 5 || y == 26;
    let vertical_divider = (15..=16).contains(&x) && (6..=25).contains(&y);
    let horizontal_divider = (15..=16).contains(&y) && (6..=25).contains(&x);

    in_bounds && (outer || vertical_divider || horizontal_divider)
}

fn native_error(context: impl Display, error: impl Display) -> PlatformError {
    PlatformError::Native(format!("{context}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesce_collapses_only_consecutive_identical_invocations() {
        let grow = CommandInvocation {
            command: Command::Grow,
            source: CommandSource::Keyboard,
        };
        let shrink = CommandInvocation {
            command: Command::Shrink,
            source: CommandSource::Keyboard,
        };

        assert_eq!(coalesce_invocations([]), Vec::new());
        assert_eq!(coalesce_invocations([grow]), vec![(grow, 1)]);
        assert_eq!(coalesce_invocations([grow, grow, grow]), vec![(grow, 3)]);
        assert_eq!(
            coalesce_invocations([grow, grow, shrink, grow]),
            vec![(grow, 2), (shrink, 1), (grow, 1)]
        );
    }

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
    fn tray_icon_has_visible_pixels() {
        assert!(is_icon_stroke(5, 5));
        assert!(is_icon_stroke(15, 20));
        assert!(!is_icon_stroke(0, 0));
        assert!(!is_icon_stroke(10, 10));
    }
}
