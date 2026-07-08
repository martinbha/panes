//! Persistent configuration loading for panes.
//!
//! Configuration is a TOML file with three optional sections:
//!
//! ```toml
//! [layout]
//! gap = 8.0
//! horizontal-split = 0.5
//!
//! [hotkeys]
//! left-half = "Control+Alt+ArrowLeft" # rebind by command id
//! center-half = ""                    # empty string unbinds
//!
//! [commands]
//! disabled = ["top-left"]             # hidden from menu and hotkeys
//! ```
//!
//! Parse failures fall back to built-in defaults with a hard error; invalid
//! individual values fall back per field and are reported as [`ConfigIssue`]s
//! so one bad value never takes the whole app down.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

use panes_core::{Command, LayoutConfig};
use panes_platform::{HotkeyBinding, MenuEntry, default_hotkey_bindings, default_menu_entries};
use serde::Deserialize;

/// Fully resolved application configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct AppConfig {
    pub layout: LayoutConfig,
    pub menu_entries: Vec<MenuEntry>,
    pub hotkey_bindings: Vec<HotkeyBinding>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            layout: LayoutConfig::default(),
            menu_entries: default_menu_entries(),
            hotkey_bindings: default_hotkey_bindings(),
        }
    }
}

/// Non-fatal problem found while resolving a config file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigIssue {
    InvalidValue {
        field: &'static str,
        message: String,
    },
    UnknownCommand {
        section: &'static str,
        id: String,
    },
    DisabledCommandBound {
        id: String,
    },
    DuplicateAccelerator {
        accelerator: String,
        kept: Command,
        dropped: Command,
    },
}

impl fmt::Display for ConfigIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidValue { field, message } => {
                write!(formatter, "{field}: {message}; using the default value")
            }
            Self::UnknownCommand { section, id } => {
                write!(formatter, "{section}: unknown command id {id:?}; ignored")
            }
            Self::DisabledCommandBound { id } => {
                write!(
                    formatter,
                    "hotkeys: {id:?} is bound but listed in commands.disabled; the hotkey is dropped"
                )
            }
            Self::DuplicateAccelerator {
                accelerator,
                kept,
                dropped,
            } => {
                write!(
                    formatter,
                    "hotkeys: {accelerator:?} is bound to both {} and {}; keeping {}",
                    kept.label(),
                    dropped.label(),
                    kept.label()
                )
            }
        }
    }
}

/// Fatal problem that prevents using a config file at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    Read { path: PathBuf, message: String },
    Parse { path: PathBuf, message: String },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, message } => {
                write!(formatter, "failed to read {}: {message}", path.display())
            }
            Self::Parse { path, message } => {
                write!(formatter, "invalid config {}: {message}", path.display())
            }
        }
    }
}

impl std::error::Error for ConfigError {}

/// Result of [`load`]: always usable, with any problems attached.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigLoad {
    pub config: AppConfig,
    pub issues: Vec<ConfigIssue>,
    pub error: Option<ConfigError>,
    pub path: Option<PathBuf>,
}

/// Platform config file location, e.g. `~/Library/Application
/// Support/panes/config.toml` on macOS or `%APPDATA%\panes\config.toml` on
/// Windows.
#[must_use]
pub fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|directory| directory.join("panes").join("config.toml"))
}

/// Load the config from the default platform path, falling back to built-in
/// defaults on any fatal error. Never panics and never fails.
#[must_use]
pub fn load() -> ConfigLoad {
    let Some(path) = default_config_path() else {
        return ConfigLoad {
            config: AppConfig::default(),
            issues: Vec::new(),
            error: None,
            path: None,
        };
    };

    match load_from_path(&path) {
        Ok((config, issues)) => ConfigLoad {
            config,
            issues,
            error: None,
            path: Some(path),
        },
        Err(error) => ConfigLoad {
            config: AppConfig::default(),
            issues: Vec::new(),
            error: Some(error),
            path: Some(path),
        },
    }
}

/// Load a config file from an explicit path. A missing file is not an error
/// and produces the built-in defaults.
pub fn load_from_path(path: &Path) -> Result<(AppConfig, Vec<ConfigIssue>), ConfigError> {
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((AppConfig::default(), Vec::new()));
        }
        Err(error) => {
            return Err(ConfigError::Read {
                path: path.to_path_buf(),
                message: error.to_string(),
            });
        }
    };

    parse(&source).map_err(|message| ConfigError::Parse {
        path: path.to_path_buf(),
        message,
    })
}

/// Parse and resolve config file contents. Returns the resolved config plus
/// non-fatal issues, or an error message when the TOML itself is invalid.
pub fn parse(source: &str) -> Result<(AppConfig, Vec<ConfigIssue>), String> {
    let file: ConfigFile = toml::from_str(source).map_err(|error| error.to_string())?;
    Ok(resolve(file))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct ConfigFile {
    #[serde(default)]
    layout: LayoutSection,
    #[serde(default)]
    hotkeys: BTreeMap<String, String>,
    #[serde(default)]
    commands: CommandsSection,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct LayoutSection {
    gap: Option<f64>,
    horizontal_split: Option<f64>,
    vertical_split: Option<f64>,
    almost_maximize_width: Option<f64>,
    almost_maximize_height: Option<f64>,
    resize_step: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct CommandsSection {
    #[serde(default)]
    disabled: Vec<String>,
}

fn resolve(file: ConfigFile) -> (AppConfig, Vec<ConfigIssue>) {
    let mut issues = Vec::new();

    let layout = resolve_layout(&file.layout, &mut issues);
    let disabled = resolve_disabled(&file.commands, &mut issues);
    let hotkey_bindings = resolve_hotkeys(&file.hotkeys, &disabled, &mut issues);

    let accelerators: HashMap<Command, String> = hotkey_bindings
        .iter()
        .map(|binding| (binding.command, binding.accelerator.clone()))
        .collect();
    let menu_entries = Command::ALL
        .iter()
        .copied()
        .filter(|command| !disabled.contains(command))
        .map(|command| MenuEntry {
            command,
            label: command.label().to_owned(),
            accelerator: accelerators.get(&command).cloned(),
        })
        .collect();

    (
        AppConfig {
            layout,
            menu_entries,
            hotkey_bindings,
        },
        issues,
    )
}

fn resolve_layout(section: &LayoutSection, issues: &mut Vec<ConfigIssue>) -> LayoutConfig {
    let defaults = LayoutConfig::default();

    LayoutConfig {
        gap: validated(
            section.gap,
            defaults.gap,
            "layout.gap",
            |value| value.is_finite() && value >= 0.0,
            "must be a finite number of at least 0",
            issues,
        ),
        horizontal_split: validated_fraction(
            section.horizontal_split,
            defaults.horizontal_split,
            "layout.horizontal-split",
            issues,
        ),
        vertical_split: validated_fraction(
            section.vertical_split,
            defaults.vertical_split,
            "layout.vertical-split",
            issues,
        ),
        almost_maximize_width: validated_fraction(
            section.almost_maximize_width,
            defaults.almost_maximize_width,
            "layout.almost-maximize-width",
            issues,
        ),
        almost_maximize_height: validated_fraction(
            section.almost_maximize_height,
            defaults.almost_maximize_height,
            "layout.almost-maximize-height",
            issues,
        ),
        resize_step: validated(
            section.resize_step,
            defaults.resize_step,
            "layout.resize-step",
            |value| value.is_finite() && value >= 1.0,
            "must be a finite number of at least 1",
            issues,
        ),
    }
}

fn validated(
    value: Option<f64>,
    default: f64,
    field: &'static str,
    is_valid: impl Fn(f64) -> bool,
    requirement: &str,
    issues: &mut Vec<ConfigIssue>,
) -> f64 {
    match value {
        None => default,
        Some(value) if is_valid(value) => value,
        Some(value) => {
            issues.push(ConfigIssue::InvalidValue {
                field,
                message: format!("{value} {requirement}"),
            });
            default
        }
    }
}

fn validated_fraction(
    value: Option<f64>,
    default: f64,
    field: &'static str,
    issues: &mut Vec<ConfigIssue>,
) -> f64 {
    validated(
        value,
        default,
        field,
        |value| value.is_finite() && value > 0.0 && value < 1.0,
        "must be a number strictly between 0 and 1",
        issues,
    )
}

fn resolve_disabled(section: &CommandsSection, issues: &mut Vec<ConfigIssue>) -> HashSet<Command> {
    let mut disabled = HashSet::new();

    for id in &section.disabled {
        match Command::from_id(id) {
            Some(command) => {
                disabled.insert(command);
            }
            None => issues.push(ConfigIssue::UnknownCommand {
                section: "commands.disabled",
                id: id.clone(),
            }),
        }
    }

    disabled
}

fn resolve_hotkeys(
    overrides: &BTreeMap<String, String>,
    disabled: &HashSet<Command>,
    issues: &mut Vec<ConfigIssue>,
) -> Vec<HotkeyBinding> {
    let mut accelerators: HashMap<Command, String> = default_hotkey_bindings()
        .into_iter()
        .map(|binding| (binding.command, binding.accelerator))
        .collect();

    for (id, accelerator) in overrides {
        let Some(command) = Command::from_id(id) else {
            issues.push(ConfigIssue::UnknownCommand {
                section: "hotkeys",
                id: id.clone(),
            });
            continue;
        };

        if accelerator.trim().is_empty() {
            accelerators.remove(&command);
            continue;
        }

        if disabled.contains(&command) {
            issues.push(ConfigIssue::DisabledCommandBound { id: id.clone() });
            continue;
        }

        accelerators.insert(command, accelerator.clone());
    }

    let mut bindings = Vec::new();
    let mut seen: HashMap<String, Command> = HashMap::new();

    for &command in Command::ALL {
        if disabled.contains(&command) {
            continue;
        }
        let Some(accelerator) = accelerators.get(&command) else {
            continue;
        };

        if let Some(&kept) = seen.get(accelerator) {
            issues.push(ConfigIssue::DuplicateAccelerator {
                accelerator: accelerator.clone(),
                kept,
                dropped: command,
            });
            continue;
        }

        seen.insert(accelerator.clone(), command);
        bindings.push(HotkeyBinding {
            command,
            accelerator: accelerator.clone(),
        });
    }

    bindings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(source: &str) -> (AppConfig, Vec<ConfigIssue>) {
        parse(source).expect("config should parse")
    }

    fn menu_accelerator_for(config: &AppConfig, command: Command) -> Option<&str> {
        config
            .menu_entries
            .iter()
            .find(|entry| entry.command == command)
            .and_then(|entry| entry.accelerator.as_deref())
    }

    fn accelerator_for(config: &AppConfig, command: Command) -> Option<&str> {
        config
            .hotkey_bindings
            .iter()
            .find(|binding| binding.command == command)
            .map(|binding| binding.accelerator.as_str())
    }

    #[test]
    fn empty_source_produces_defaults() {
        let (config, issues) = parsed("");

        assert_eq!(config, AppConfig::default());
        assert_eq!(issues, Vec::new());
    }

    #[test]
    fn layout_values_override_defaults_individually() {
        let (config, issues) = parsed(
            "[layout]\n\
             gap = 8.0\n\
             horizontal-split = 0.6\n",
        );

        assert_eq!(issues, Vec::new());
        assert_eq!(config.layout.gap, 8.0);
        assert_eq!(config.layout.horizontal_split, 0.6);
        assert_eq!(config.layout.vertical_split, 0.5);
        assert_eq!(config.layout.resize_step, 30.0);
    }

    #[test]
    fn invalid_layout_values_fall_back_per_field() {
        let (config, issues) = parsed(
            "[layout]\n\
             gap = -5.0\n\
             horizontal-split = 1.5\n\
             vertical-split = 0.4\n\
             resize-step = 0.0\n",
        );

        assert_eq!(config.layout.gap, 0.0);
        assert_eq!(config.layout.horizontal_split, 0.5);
        assert_eq!(config.layout.vertical_split, 0.4);
        assert_eq!(config.layout.resize_step, 30.0);

        let fields: Vec<&str> = issues
            .iter()
            .map(|issue| match issue {
                ConfigIssue::InvalidValue { field, .. } => *field,
                other => panic!("unexpected issue {other:?}"),
            })
            .collect();
        assert_eq!(
            fields,
            [
                "layout.gap",
                "layout.horizontal-split",
                "layout.resize-step"
            ]
        );
    }

    #[test]
    fn non_finite_layout_values_are_rejected() {
        let (config, issues) = parsed("[layout]\ngap = inf\n");

        assert_eq!(config.layout.gap, 0.0);
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn hotkeys_can_be_rebound_by_command_id() {
        let (config, issues) = parsed("[hotkeys]\nmaximize = \"Control+Shift+M\"\n");

        assert_eq!(issues, Vec::new());
        assert_eq!(
            accelerator_for(&config, Command::Maximize),
            Some("Control+Shift+M")
        );
        assert_eq!(
            accelerator_for(&config, Command::LeftHalf),
            Some("Control+Alt+ArrowLeft")
        );
        assert_eq!(
            menu_accelerator_for(&config, Command::Maximize),
            Some("Control+Shift+M")
        );
    }

    #[test]
    fn empty_accelerator_unbinds_a_command() {
        let (config, issues) = parsed("[hotkeys]\nmaximize = \"\"\n");

        assert_eq!(issues, Vec::new());
        assert_eq!(accelerator_for(&config, Command::Maximize), None);
        assert_eq!(menu_accelerator_for(&config, Command::Maximize), None);
        assert!(
            config
                .menu_entries
                .iter()
                .any(|entry| entry.command == Command::Maximize)
        );
    }

    #[test]
    fn unknown_hotkey_command_ids_are_reported_and_skipped() {
        let (config, issues) = parsed(
            "[hotkeys]\n\
             maximise = \"Control+Shift+M\"\n\
             center = \"Control+Shift+C\"\n",
        );

        assert_eq!(
            issues,
            [ConfigIssue::UnknownCommand {
                section: "hotkeys",
                id: "maximise".to_owned(),
            }]
        );
        assert_eq!(
            accelerator_for(&config, Command::Center),
            Some("Control+Shift+C")
        );
    }

    #[test]
    fn disabled_commands_are_removed_from_menu_and_hotkeys() {
        let (config, issues) = parsed("[commands]\ndisabled = [\"top-left\", \"top-right\"]\n");

        assert_eq!(issues, Vec::new());
        assert!(
            !config
                .menu_entries
                .iter()
                .any(|entry| entry.command == Command::TopLeft)
        );
        assert_eq!(accelerator_for(&config, Command::TopLeft), None);
        assert_eq!(accelerator_for(&config, Command::TopRight), None);
        assert_eq!(config.menu_entries.len(), Command::ALL.len() - 2);
    }

    #[test]
    fn binding_a_disabled_command_is_reported() {
        let (config, issues) = parsed(
            "[hotkeys]\n\
             top-left = \"Control+Shift+U\"\n\
             \n\
             [commands]\n\
             disabled = [\"top-left\"]\n",
        );

        assert_eq!(
            issues,
            [ConfigIssue::DisabledCommandBound {
                id: "top-left".to_owned(),
            }]
        );
        assert_eq!(accelerator_for(&config, Command::TopLeft), None);
    }

    #[test]
    fn unbinding_a_disabled_command_is_silent() {
        let (config, issues) = parsed(
            "[hotkeys]\n\
             top-left = \"\"\n\
             \n\
             [commands]\n\
             disabled = [\"top-left\"]\n",
        );

        assert_eq!(issues, Vec::new());
        assert_eq!(accelerator_for(&config, Command::TopLeft), None);
    }

    #[test]
    fn unknown_disabled_ids_are_reported() {
        let (_, issues) = parsed("[commands]\ndisabled = [\"not-a-command\"]\n");

        assert_eq!(
            issues,
            [ConfigIssue::UnknownCommand {
                section: "commands.disabled",
                id: "not-a-command".to_owned(),
            }]
        );
    }

    #[test]
    fn duplicate_accelerators_keep_the_first_command() {
        let (config, issues) = parsed("[hotkeys]\nmaximize = \"Control+Alt+C\"\n");

        // Control+Alt+C is also the default for Center; Maximize precedes it
        // in Command::ALL order, so Maximize keeps the accelerator.
        assert_eq!(
            issues,
            [ConfigIssue::DuplicateAccelerator {
                accelerator: "Control+Alt+C".to_owned(),
                kept: Command::Maximize,
                dropped: Command::Center,
            }]
        );
        assert_eq!(
            accelerator_for(&config, Command::Maximize),
            Some("Control+Alt+C")
        );
        assert_eq!(accelerator_for(&config, Command::Center), None);
    }

    #[test]
    fn malformed_toml_is_a_hard_error() {
        assert!(parse("[layout\ngap = 1").is_err());
    }

    #[test]
    fn unknown_keys_are_a_hard_error() {
        let error = parse("[layout]\ngapp = 1.0\n").unwrap_err();

        assert!(error.contains("gapp"), "error should name the key: {error}");
    }

    #[test]
    fn missing_file_loads_defaults() {
        let path = std::env::temp_dir().join("panes-config-test-missing/config.toml");

        let (config, issues) = load_from_path(&path).expect("missing file is not an error");

        assert_eq!(config, AppConfig::default());
        assert_eq!(issues, Vec::new());
    }

    #[test]
    fn config_file_is_loaded_from_disk() {
        let directory = std::env::temp_dir().join(format!(
            "panes-config-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&directory).expect("create temp dir");
        let path = directory.join("config.toml");
        std::fs::write(&path, "[layout]\ngap = 12.0\n").expect("write config");

        let (config, issues) = load_from_path(&path).expect("config should load");

        assert_eq!(config.layout.gap, 12.0);
        assert_eq!(issues, Vec::new());

        std::fs::remove_dir_all(&directory).expect("clean up temp dir");
    }

    #[test]
    fn parse_errors_carry_the_file_path() {
        let directory = std::env::temp_dir().join(format!(
            "panes-config-test-parse-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&directory).expect("create temp dir");
        let path = directory.join("config.toml");
        std::fs::write(&path, "not valid toml [").expect("write config");

        let error = load_from_path(&path).expect_err("parse should fail");

        match &error {
            ConfigError::Parse {
                path: error_path, ..
            } => assert_eq!(error_path, &path),
            other => panic!("unexpected error {other:?}"),
        }

        std::fs::remove_dir_all(&directory).expect("clean up temp dir");
    }

    #[test]
    fn default_config_path_ends_with_panes_config_toml() {
        if let Some(path) = default_config_path() {
            assert!(path.ends_with("panes/config.toml"));
        }
    }
}
