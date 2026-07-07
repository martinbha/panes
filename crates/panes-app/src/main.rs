use panes_platform::{NativePlatform, default_menu_entries};

#[cfg(target_os = "macos")]
use panes_macos::MacOsPlatform as CurrentPlatform;
#[cfg(target_os = "windows")]
use panes_windows::WindowsPlatform as CurrentPlatform;

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn main() {
    let mut platform = CurrentPlatform::new();
    let menu_entries = default_menu_entries();
    println!(
        "panes starting on {} with {} menu commands",
        platform.platform_name(),
        menu_entries.len()
    );

    if let Err(error) = platform.show_tray_menu(&menu_entries) {
        println!("native tray not ready: {error:?}");
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn main() {
    println!("panes currently targets macOS and Windows");
}
