#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use std::process::ExitCode;

use panes_core::Command;
use panes_platform::{default_hotkey_bindings, default_menu_entries};

const USAGE: &str = "\
Usage:
  panes
  panes --runtime-summary
  panes exec --list
  panes exec [--delay <milliseconds>] <command-id>";

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum AppMode {
    Resident,
    RuntimeSummary,
    ListCommands,
    Exec { command: Command, delay_ms: u64 },
    Help,
}

#[cfg(target_os = "macos")]
fn main() -> ExitCode {
    let mode = match process_mode() {
        Ok(mode) => mode,
        Err(error) => return report_argument_error(&error),
    };

    match mode {
        AppMode::Resident => {
            let loaded = panes_runtime::config::load();
            report_config_problems(&loaded);
            let config_path = loaded.path.clone();

            let mut executor = panes_runtime::CommandExecutor::new(
                panes_macos::MacOsPlatform::new(),
                loaded.config.layout.clone(),
            );
            panes_macos::run_keyboard_menu_app_with_handler(
                loaded.config.menu_entries,
                loaded.config.hotkey_bindings,
                loaded.config.launch_at_login,
                move |invocation, repeats| {
                    if let Err(error) = executor.execute_repeated(invocation, repeats) {
                        report_command_failure(invocation, &error);
                    }
                },
                move |enabled| persist_launch_at_login(&config_path, enabled),
            );
        }
        AppMode::RuntimeSummary => {
            print_runtime_summary("macos");
            ExitCode::SUCCESS
        }
        AppMode::ListCommands => {
            print!("{}", command_id_listing());
            ExitCode::SUCCESS
        }
        AppMode::Exec { command, delay_ms } => {
            execute_one_shot(panes_macos::MacOsPlatform::new(), command, delay_ms)
        }
        AppMode::Help => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
    }
}

#[cfg(target_os = "windows")]
fn main() -> ExitCode {
    let mode = match process_mode() {
        Ok(mode) => mode,
        Err(error) => return report_argument_error(&error),
    };

    match mode {
        AppMode::Resident => {
            let loaded = panes_runtime::config::load();
            report_config_problems(&loaded);
            let config_path = loaded.path.clone();

            let mut executor = panes_runtime::CommandExecutor::new(
                panes_windows::WindowsPlatform::new(),
                loaded.config.layout.clone(),
            );
            panes_windows::run_keyboard_menu_app_with_handler(
                loaded.config.menu_entries,
                loaded.config.hotkey_bindings,
                loaded.config.launch_at_login,
                move |invocation, repeats| {
                    if let Err(error) = executor.execute_repeated(invocation, repeats) {
                        report_command_failure(invocation, &error);
                    }
                },
                move |enabled| persist_launch_at_login(&config_path, enabled),
            );
        }
        AppMode::RuntimeSummary => {
            print_runtime_summary("windows");
            ExitCode::SUCCESS
        }
        AppMode::ListCommands => {
            print!("{}", command_id_listing());
            ExitCode::SUCCESS
        }
        AppMode::Exec { command, delay_ms } => {
            execute_one_shot(panes_windows::WindowsPlatform::new(), command, delay_ms)
        }
        AppMode::Help => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn main() -> ExitCode {
    let mode = match process_mode() {
        Ok(mode) => mode,
        Err(error) => return report_argument_error(&error),
    };

    match mode {
        AppMode::RuntimeSummary | AppMode::Resident => {
            print_runtime_summary("unsupported");
            println!("panes currently targets macOS and Windows");
            ExitCode::SUCCESS
        }
        AppMode::ListCommands => {
            print!("{}", command_id_listing());
            ExitCode::SUCCESS
        }
        AppMode::Exec { .. } => {
            eprintln!("panes exec is only available on macOS and Windows");
            ExitCode::FAILURE
        }
        AppMode::Help => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
    }
}

fn process_mode() -> Result<AppMode, String> {
    parse_arguments(&std::env::args().skip(1).collect::<Vec<_>>())
}

fn parse_arguments(arguments: &[String]) -> Result<AppMode, String> {
    match arguments {
        [] => Ok(AppMode::Resident),
        [flag] if flag == "--runtime-summary" => Ok(AppMode::RuntimeSummary),
        [flag] if flag == "--help" || flag == "-h" => Ok(AppMode::Help),
        [subcommand, rest @ ..] if subcommand == "exec" => parse_exec_arguments(rest),
        _ => Err(format!("unrecognized arguments: {}", arguments.join(" "))),
    }
}

fn parse_exec_arguments(arguments: &[String]) -> Result<AppMode, String> {
    match arguments {
        [flag] if flag == "--list" => Ok(AppMode::ListCommands),
        [flag] if flag == "--help" || flag == "-h" => Ok(AppMode::Help),
        [command_id] => command_mode(command_id, 0),
        [delay_flag, delay, command_id] if delay_flag == "--delay" => {
            let delay_ms = delay.parse::<u64>().map_err(|_| {
                format!("invalid delay '{delay}'; expected non-negative milliseconds")
            })?;
            command_mode(command_id, delay_ms)
        }
        [] => Err("missing command id; run `panes exec --list` to list commands".to_owned()),
        _ => Err(format!(
            "invalid exec arguments: {}; run `panes exec --help` for usage",
            arguments.join(" ")
        )),
    }
}

fn command_mode(command_id: &str, delay_ms: u64) -> Result<AppMode, String> {
    let command = Command::from_id(command_id).ok_or_else(|| {
        format!("unknown command id '{command_id}'; run `panes exec --list` to list commands")
    })?;
    Ok(AppMode::Exec { command, delay_ms })
}

fn command_id_listing() -> String {
    let mut listing = Command::ALL
        .iter()
        .copied()
        .map(Command::id)
        .collect::<Vec<_>>()
        .join("\n");
    listing.push('\n');
    listing
}

fn report_argument_error(error: &str) -> ExitCode {
    eprintln!("panes: {error}\n\n{USAGE}");
    ExitCode::from(2)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn execute_one_shot<P: panes_platform::NativePlatform>(
    platform: P,
    command: Command,
    delay_ms: u64,
) -> ExitCode {
    use panes_platform::{CommandInvocation, CommandSource};

    let loaded = panes_runtime::config::load();
    report_config_problems(&loaded);

    if delay_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
    }

    let invocation = CommandInvocation {
        command,
        source: CommandSource::Cli,
    };
    let mut executor = panes_runtime::CommandExecutor::new(platform, loaded.config.layout);
    match executor.execute(invocation) {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("panes exec {} failed: {error}", command.id());
            ExitCode::FAILURE
        }
    }
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
fn persist_launch_at_login(
    config_path: &Option<std::path::PathBuf>,
    enabled: bool,
) -> Result<(), String> {
    let path = config_path
        .as_deref()
        .ok_or_else(|| "the platform config directory is unavailable".to_owned())?;
    panes_runtime::config::save_launch_at_login(path, enabled).map_err(|error| error.to_string())
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

#[cfg(test)]
mod tests {
    use panes_core::Command;

    use super::{AppMode, command_id_listing, parse_arguments};

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn parses_resident_summary_and_help_modes() {
        assert_eq!(parse_arguments(&[]), Ok(AppMode::Resident));
        assert_eq!(
            parse_arguments(&arguments(&["--runtime-summary"])),
            Ok(AppMode::RuntimeSummary)
        );
        assert_eq!(parse_arguments(&arguments(&["--help"])), Ok(AppMode::Help));
    }

    #[test]
    fn parses_command_and_delay_modes() {
        assert_eq!(
            parse_arguments(&arguments(&["exec", "left-half"])),
            Ok(AppMode::Exec {
                command: Command::LeftHalf,
                delay_ms: 0,
            })
        );
        assert_eq!(
            parse_arguments(&arguments(&["exec", "--delay", "500", "top-right"])),
            Ok(AppMode::Exec {
                command: Command::TopRight,
                delay_ms: 500,
            })
        );
    }

    #[test]
    fn lists_every_stable_command_id() {
        let listing = command_id_listing();

        assert_eq!(listing.lines().count(), Command::ALL.len());
        for command in Command::ALL {
            assert!(listing.lines().any(|id| id == command.id()));
        }
    }

    #[test]
    fn rejects_unknown_commands_and_invalid_delays() {
        let unknown = parse_arguments(&arguments(&["exec", "not-a-command"]))
            .expect_err("unknown command should fail");
        assert!(unknown.contains("unknown command id 'not-a-command'"));
        assert!(unknown.contains("panes exec --list"));

        let delay = parse_arguments(&arguments(&["exec", "--delay", "-1", "left-half"]))
            .expect_err("negative delay should fail");
        assert!(delay.contains("non-negative milliseconds"));
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
mod native_tests {
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

    #[test]
    fn cli_failure_record_keeps_its_source() {
        let invocation = CommandInvocation {
            command: Command::LeftHalf,
            source: CommandSource::Cli,
        };

        let record = format_command_failure(
            invocation,
            &CommandExecutionError::NoFocusedWindow,
            CommandFailureLevel::Debug,
        );

        assert!(record.contains("command=left-half source=Cli"));
    }
}
