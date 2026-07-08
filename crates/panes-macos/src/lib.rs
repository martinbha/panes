use std::{collections::HashMap, fmt::Display};

mod coordinates;
mod screen;
mod window;

use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{HotKey, HotKeyParseError},
};
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
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu},
};

const QUIT_MENU_ID: &str = "panes.quit";

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
        |invocation| {
            println!(
                "received {:?} command from {:?}",
                invocation.command, invocation.source
            );
        },
    )
}

pub fn run_keyboard_menu_app_with_handler<F>(
    menu_entries: Vec<MenuEntry>,
    hotkey_bindings: Vec<HotkeyBinding>,
    mut handle_command: F,
) -> !
where
    F: FnMut(CommandInvocation) + 'static,
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
    GlobalHotKeyEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Hotkey(event));
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

                if let Some(invocation) = platform.invocation_for_menu_id(event.id()) {
                    handle_command(invocation);
                }
            }
            Event::UserEvent(UserEvent::Hotkey(event)) => {
                if event.state() != HotKeyState::Pressed {
                    return;
                }

                if let Some(invocation) = platform.invocation_for_hotkey_id(event.id()) {
                    handle_command(invocation);
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
    Hotkey(GlobalHotKeyEvent),
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
            let item = MenuItem::with_id(menu_id.clone(), &entry.label, true, None);
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
