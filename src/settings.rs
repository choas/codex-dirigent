//! Minimal, atomic application settings.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::codex::{CodexConfig, DEFAULT_MODEL};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub codex_cli_path: String,
    pub codex_model: String,
    pub codex_extra_arguments: String,
    /// Newline-separated environment variable names. Values are never saved.
    pub codex_environment_names: String,
    pub codex_pre_run_command: String,
    pub codex_post_run_command: String,
    pub last_repository: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            codex_cli_path: "codex".to_owned(),
            codex_model: DEFAULT_MODEL.to_owned(),
            codex_extra_arguments: String::new(),
            codex_environment_names: String::new(),
            codex_pre_run_command: String::new(),
            codex_post_run_command: String::new(),
            last_repository: None,
        }
    }
}

impl Settings {
    #[must_use]
    pub fn codex_config(&self) -> CodexConfig {
        CodexConfig {
            cli_path: PathBuf::from(self.codex_cli_path.trim()),
            model: self.codex_model.trim().to_owned(),
            extra_arguments: self.codex_extra_arguments.clone(),
            environment_names: self
                .codex_environment_names
                .lines()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect(),
            pre_run_command: self.codex_pre_run_command.clone(),
            post_run_command: self.codex_post_run_command.clone(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("settings location is unavailable")]
    LocationUnavailable,
    #[error("settings file is invalid: {0}")]
    Invalid(#[from] serde_json::Error),
    #[error("settings could not be read or saved: {0}")]
    Io(#[from] std::io::Error),
    #[error("atomic settings replacement failed: {0}")]
    Persist(#[from] tempfile::PersistError),
}

/// Return the macOS Application Support location for settings.
///
/// # Errors
///
/// Returns an error when the home directory is unavailable.
pub fn default_path() -> Result<PathBuf, SettingsError> {
    let home = std::env::var_os("HOME").ok_or(SettingsError::LocationUnavailable)?;
    Ok(PathBuf::from(home).join("Library/Application Support/Codex Dirigent/settings.json"))
}

/// Load settings, returning defaults when the file does not yet exist.
/// Unknown fields from older products or versions are ignored by Serde.
///
/// # Errors
///
/// Returns an error when the file cannot be read or its JSON/types are invalid.
pub fn load(path: &Path) -> Result<Settings, SettingsError> {
    match fs::read(path) {
        Ok(bytes) => {
            let mut settings: Settings = serde_json::from_slice(&bytes)?;
            if settings.codex_model.trim().is_empty() {
                DEFAULT_MODEL.clone_into(&mut settings.codex_model);
            }
            Ok(settings)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(error) => Err(SettingsError::Io(error)),
    }
}

/// Atomically save settings beside the final file.
///
/// # Errors
///
/// Returns an error when directory creation, serialization, syncing, or the
/// final atomic replacement fails.
pub fn save(path: &Path, settings: &Settings) -> Result<(), SettingsError> {
    let parent = path.parent().ok_or(SettingsError::LocationUnavailable)?;
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temporary, settings)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_obsolete_fields_do_not_block_loading() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        fs::write(
            &path,
            r#"{
                "codex_model": "gpt-5.4",
                "codex_environment_names": "TOKEN_NAME\nBUILD_MODE",
                "obsolete_backend_selector": "unused",
                "old_visual_effect": true
            }"#,
        )
        .unwrap();
        let settings = load(&path).unwrap();
        assert_eq!(settings.codex_model, "gpt-5.4");
        assert_eq!(
            settings.codex_config().environment_names,
            ["TOKEN_NAME", "BUILD_MODE"]
        );
    }

    #[test]
    fn save_is_round_trip_and_does_not_persist_environment_values() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("nested/settings.json");
        let settings = Settings {
            codex_environment_names: "SECRET_NAME".to_owned(),
            last_repository: Some(PathBuf::from("/tmp/project")),
            ..Settings::default()
        };
        save(&path, &settings).unwrap();
        assert_eq!(load(&path).unwrap(), settings);
        let serialized = fs::read_to_string(path).unwrap();
        assert!(serialized.contains("SECRET_NAME"));
        assert!(!serialized.contains("secret-value"));
    }

    #[test]
    fn corrupt_file_returns_error_for_nonfatal_startup_handling() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        fs::write(&path, b"{not json").unwrap();
        assert!(matches!(load(&path), Err(SettingsError::Invalid(_))));
    }

    #[test]
    fn blank_persisted_model_migrates_to_supported_gpt_5_6_variant() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        fs::write(&path, r#"{"codex_model":"  "}"#).unwrap();

        let settings = load(&path).unwrap();

        assert_eq!(settings.codex_model, DEFAULT_MODEL);
        assert_eq!(settings.codex_config().model, DEFAULT_MODEL);
    }
}
