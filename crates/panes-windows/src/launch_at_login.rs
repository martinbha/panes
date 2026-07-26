use std::path::Path;

use panes_platform::{LaunchAtLoginStatus, PlatformError, PlatformResult};
use windows_registry::{CURRENT_USER, Key, Value};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "Panes";
const MAX_RUN_COMMAND_UNITS: usize = 260;

pub(crate) fn status() -> PlatformResult<LaunchAtLoginStatus> {
    let key = run_key()?;
    let Some(value) = value(&key) else {
        return Ok(LaunchAtLoginStatus::Disabled);
    };
    let Ok(command) = <Value as TryInto<String>>::try_into(value) else {
        return Ok(LaunchAtLoginStatus::Stale);
    };
    let expected = launch_command()?;

    Ok(if commands_match(&command, &expected) {
        LaunchAtLoginStatus::Enabled
    } else {
        LaunchAtLoginStatus::Stale
    })
}

pub(crate) fn set_enabled(enabled: bool) -> PlatformResult<()> {
    let key = run_key()?;
    if enabled {
        let command = launch_command()?;
        key.set_string(VALUE_NAME, command).map_err(|error| {
            PlatformError::Native(format!("failed to enable Windows launch at login: {error}"))
        })
    } else if value(&key).is_some() {
        key.remove_value(VALUE_NAME).map_err(|error| {
            PlatformError::Native(format!(
                "failed to disable Windows launch at login: {error}"
            ))
        })
    } else {
        Ok(())
    }
}

fn run_key() -> PlatformResult<Key> {
    CURRENT_USER.create(RUN_KEY).map_err(|error| {
        PlatformError::Native(format!(
            "failed to open Windows login registry key: {error}"
        ))
    })
}

fn value(key: &Key) -> Option<Value> {
    key.values()
        .ok()?
        .find_map(|(name, value)| name.eq_ignore_ascii_case(VALUE_NAME).then_some(value))
}

fn launch_command() -> PlatformResult<String> {
    let executable = std::env::current_exe().map_err(|error| {
        PlatformError::Native(format!(
            "failed to resolve the Windows executable path: {error}"
        ))
    })?;
    let command = quoted_path(&executable);
    if command.encode_utf16().count() > MAX_RUN_COMMAND_UNITS {
        return Err(PlatformError::Unsupported(
            "Windows launch-at-login command exceeds the registry Run limit",
        ));
    }
    Ok(command)
}

fn quoted_path(path: &Path) -> String {
    format!("\"{}\"", path.to_string_lossy())
}

fn commands_match(actual: &str, expected: &str) -> bool {
    actual.trim().eq_ignore_ascii_case(expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_executable_paths_with_spaces() {
        assert_eq!(
            quoted_path(Path::new(r"C:\Program Files\Panes\panes.exe")),
            r#""C:\Program Files\Panes\panes.exe""#
        );
    }

    #[test]
    fn command_matching_ignores_windows_path_case_and_outer_whitespace() {
        assert!(commands_match(
            r#"  "C:\APPS\Panes.exe" "#,
            r#""c:\apps\panes.exe""#
        ));
        assert!(!commands_match(
            r#""C:\old\panes.exe""#,
            r#""C:\new\panes.exe""#
        ));
    }
}
