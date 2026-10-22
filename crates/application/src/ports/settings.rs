use nextgdrive_domain::Settings;

use super::PortError;

/// Persists user settings. Loading before anything was saved yields defaults.
pub trait SettingsRepository: Sync {
    fn load(&self) -> Result<Settings, PortError>;

    fn save(&self, settings: &Settings) -> Result<(), PortError>;
}
