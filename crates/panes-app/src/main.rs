use panes_platform::{default_hotkey_bindings, default_menu_entries};

#[cfg(target_os = "windows")]
use panes_platform::NativePlatform;

#[cfg(target_os = "macos")]
fn main() {
    if wants_runtime_summary() {
        print_runtime_summary("macos");
        return;
    }

    let mut executor =
        panes_runtime::CommandExecutor::with_default_config(panes_macos::MacOsPlatform::new());
    panes_macos::run_keyboard_menu_app_with_handler(move |invocation| {
        if let Err(error) = executor.execute(invocation) {
            eprintln!(
                "failed to execute {} command from {:?}: {error}",
                invocation.command.label(),
                invocation.source
            );
        }
    });
}

#[cfg(target_os = "windows")]
fn main() {
    let platform = panes_windows::WindowsPlatform::new();
    print_runtime_summary(platform.platform_name());
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn main() {
    print_runtime_summary("unsupported");
    println!("panes currently targets macOS and Windows");
}

#[cfg(target_os = "macos")]
fn wants_runtime_summary() -> bool {
    std::env::args().any(|argument| argument == "--runtime-summary")
}

fn print_runtime_summary(platform_name: &str) {
    let menu_entries = default_menu_entries();
    let hotkey_bindings = default_hotkey_bindings();
    println!(
        "panes runtime target: {platform_name}\nmenu commands: {}\nhotkeys: {}",
        menu_entries.len(),
        hotkey_bindings.len()
    );
}
