#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use panes_platform::{default_hotkey_bindings, default_menu_entries};

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
                report_command_failure(invocation, &error);
            }
        },
    );
}

#[cfg(target_os = "windows")]
fn main() {
    if wants_runtime_summary() {
        print_runtime_summary("windows");
        return;
    }

    let loaded = panes_runtime::config::load();
    report_config_problems(&loaded);

    let mut executor = panes_runtime::CommandExecutor::new(
        panes_windows::WindowsPlatform::new(),
        loaded.config.layout.clone(),
    );
    panes_windows::run_keyboard_menu_app_with_handler(
        loaded.config.menu_entries,
        loaded.config.hotkey_bindings,
        move |invocation, repeats| {
            if let Err(error) = executor.execute_repeated(invocation, repeats) {
                report_command_failure(invocation, &error);
            }
        },
    );
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn main() {
    print_runtime_summary("unsupported");
    println!("panes currently targets macOS and Windows");
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn wants_runtime_summary() -> bool {
    std::env::args().any(|argument| argument == "--runtime-summary")
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn report_config_problems(loaded: &panes_runtime::config::ConfigLoad) {
    if let Some(error) = &loaded.error {
        eprintln!("panes config error: {error}; using built-in defaults");
    }

    for issue in &loaded.issues {
        eprintln!("panes config warning: {issue}");
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn report_command_failure(
    invocation: panes_platform::CommandInvocation,
    error: &panes_runtime::CommandExecutionError,
) {
    use panes_runtime::CommandFailureLevel;

    let level = error.failure_level();
    if cfg!(debug_assertions) || level == CommandFailureLevel::Error {
        eprintln!("{}", format_command_failure(invocation, error, level));
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn format_command_failure(
    invocation: panes_platform::CommandInvocation,
    error: &panes_runtime::CommandExecutionError,
    level: panes_runtime::CommandFailureLevel,
) -> String {
    use panes_runtime::CommandFailureLevel;

    format!(
        "event=command_failure level={} command={} source={:?} error={error:?}",
        match level {
            CommandFailureLevel::Debug => "debug",
            CommandFailureLevel::Error => "error",
        },
        invocation.command.id(),
        invocation.source,
        error = error.to_string(),
    )
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

#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
mod tests {
    use panes_core::Command;
    use panes_platform::{CommandInvocation, CommandSource};
    use panes_runtime::{CommandExecutionError, CommandFailureLevel};

    use super::format_command_failure;

    #[test]
    fn command_failure_is_one_parseable_record() {
        let invocation = CommandInvocation {
            command: Command::Maximize,
            source: CommandSource::Keyboard,
        };

        let record = format_command_failure(
            invocation,
            &CommandExecutionError::NoFocusedWindow,
            CommandFailureLevel::Debug,
        );

        assert_eq!(
            record,
            "event=command_failure level=debug command=maximize source=Keyboard error=\"no focused window\""
        );
    }
}
