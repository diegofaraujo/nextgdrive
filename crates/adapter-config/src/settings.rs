use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nextgdrive_application::ports::{PortError, SettingsRepository};
use nextgdrive_domain::{AccountId, DriveItemId, Settings};
use serde::{Deserialize, Serialize};

/// Stores settings in a TOML file. Missing values take the given defaults, so
/// a missing file means default settings.
pub struct TomlSettingsRepository {
    path: PathBuf,
    defaults: Settings,
}

#[derive(Default, Serialize, Deserialize)]
struct SettingsFile {
    sync_root: Option<PathBuf>,
    my_drive_folder: Option<String>,
    shared_drives_folder: Option<String>,
    poll_interval_secs: Option<u64>,
    notifications: Option<bool>,
    /// Keyed by account id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    accounts: BTreeMap<String, AccountSettings>,
}

#[derive(Default, Serialize, Deserialize)]
struct AccountSettings {
    #[serde(default)]
    selected_folders: BTreeSet<String>,
}

impl TomlSettingsRepository {
    pub fn new(path: impl Into<PathBuf>, defaults: Settings) -> Self {
        Self {
            path: path.into(),
            defaults,
        }
    }

    /// Uses `settings.toml` in the default config directory.
    pub fn in_config_dir(defaults: Settings) -> Option<Self> {
        crate::config_dir().map(|dir| Self::new(dir.join("settings.toml"), defaults))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn error(&self, source: impl std::fmt::Display) -> PortError {
        PortError::new(format!("{}: {source}", self.path.display()))
    }
}

impl SettingsRepository for TomlSettingsRepository {
    fn load(&self) -> Result<Settings, PortError> {
        let file: SettingsFile = match fs::read_to_string(&self.path) {
            Ok(text) => toml::from_str(&text).map_err(|e| self.error(e))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => SettingsFile::default(),
            Err(e) => return Err(self.error(e)),
        };
        let defaults = self.defaults.clone();
        Ok(Settings {
            sync_root: file.sync_root.unwrap_or(defaults.sync_root),
            my_drive_folder: file.my_drive_folder.unwrap_or(defaults.my_drive_folder),
            shared_drives_folder: file
                .shared_drives_folder
                .unwrap_or(defaults.shared_drives_folder),
            // A zero interval would make background sync spin.
            poll_interval_secs: file
                .poll_interval_secs
                .filter(|secs| *secs > 0)
                .unwrap_or(defaults.poll_interval_secs),
            notifications: file.notifications.unwrap_or(defaults.notifications),
            selections: file
                .accounts
                .into_iter()
                .filter(|(_, a)| !a.selected_folders.is_empty())
                .map(|(id, a)| {
                    let folders = a
                        .selected_folders
                        .into_iter()
                        .map(DriveItemId::new)
                        .collect();
                    (AccountId::new(id), folders)
                })
                .collect(),
        })
    }

    /// Writes every value, defaults included, so the folder names chosen
    /// (translated) on first save do not change with the language later.
    fn save(&self, settings: &Settings) -> Result<(), PortError> {
        let file = SettingsFile {
            sync_root: Some(settings.sync_root.clone()),
            my_drive_folder: Some(settings.my_drive_folder.clone()),
            shared_drives_folder: Some(settings.shared_drives_folder.clone()),
            poll_interval_secs: Some(settings.poll_interval_secs),
            notifications: Some(settings.notifications),
            accounts: settings
                .selections
                .iter()
                .map(|(id, folders)| {
                    let selected_folders = folders.iter().map(ToString::to_string).collect();
                    (id.to_string(), AccountSettings { selected_folders })
                })
                .collect(),
        };
        let text = toml::to_string(&file).map_err(|e| self.error(e))?;
        crate::write_atomically(&self.path, &text).map_err(|e| self.error(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> Settings {
        Settings {
            sync_root: "/home/ana/GoogleDrive".into(),
            my_drive_folder: "Meu Drive".into(),
            shared_drives_folder: "Drives compartilhados".into(),
            poll_interval_secs: 60,
            notifications: true,
            selections: BTreeMap::new(),
        }
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let repo = TomlSettingsRepository::new(dir.path().join("settings.toml"), defaults());
        assert_eq!(repo.load().unwrap(), defaults());
    }

    #[test]
    fn round_trips_and_pins_folder_names() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let mut settings = defaults();
        settings.sync_root = "/data/drive".into();
        settings.select(&AccountId::new("1"), DriveItemId::new("folder-a"));
        TomlSettingsRepository::new(&path, defaults())
            .save(&settings)
            .unwrap();

        // Later run in another language: saved names win over new defaults.
        let english = Settings {
            my_drive_folder: "My Drive".into(),
            ..defaults()
        };
        assert_eq!(
            TomlSettingsRepository::new(&path, english).load().unwrap(),
            settings
        );
    }
}
