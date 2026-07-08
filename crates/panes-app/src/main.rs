use panes_platform::{default_hotkey_bindings, default_menu_entries};

#[cfg(target_os = "windows")]
use panes_platform::NativePlatform;

#[cfg(target_os = "macos")]
fn main() {
    if wants_runtime_summary() {
        print_runtime_summary("macos");
        return;
    }

    let loaded = panes_runtime::config::load();
    report_config_problems(&loaded);

    let mut executor = panes_runtime::CommandExecutor::new(
        panes_macos::MacOsPlatform::new(),
        loaded.config.layout.clone(),
    );
    panes_macos::run_keyboard_menu_app_with_handler(
        loaded.config.menu_entries,
        loaded.config.hotkey_bindings,
        move |invocation, repeats| {
            if let Err(error) = executor.execute_repeated(invocation, repeats) {
                eprintln!(
                    "failed to execute {} command from {:?}: {error}",
                    invocation.command.label(),
                    invocation.source
                );
            }
        },
    );
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

#[cfg(target_os = "macos")]
fn report_config_problems(loaded: &panes_runtime::config::ConfigLoad) {
    if let Some(error) = &loaded.error {
        eprintln!("panes config error: {error}; using built-in defaults");
    }

    for issue in &loaded.issues {
        eprintln!("panes config warning: {issue}");
    }
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
