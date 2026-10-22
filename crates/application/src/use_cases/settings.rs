use std::path::PathBuf;

use nextgdrive_domain::Settings;

use crate::AppError;
use crate::ports::SettingsRepository;

pub struct GetSettings<'a, S> {
    settings: &'a S,
}

impl<'a, S: SettingsRepository> GetSettings<'a, S> {
    pub fn new(settings: &'a S) -> Self {
        Self { settings }
    }

    pub fn execute(&self) -> Result<Settings, AppError> {
        self.settings.load().map_err(AppError::Storage)
    }
}

/// Changes the folder that holds the synced accounts. Files already synced
/// stay where they are; the next sync downloads into the new folder.
pub struct SetSyncRoot<'a, S> {
    settings: &'a S,
}

impl<'a, S: SettingsRepository> SetSyncRoot<'a, S> {
    pub fn new(settings: &'a S) -> Self {
        Self { settings }
    }

    pub fn execute(&self, root: PathBuf) -> Result<(), AppError> {
        if !root.is_absolute() {
            return Err(AppError::InvalidSyncRoot(root));
        }
        let mut settings = self.settings.load().map_err(AppError::Storage)?;
        settings.sync_root = root;
        self.settings.save(&settings).map_err(AppError::Storage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::fakes::InMemorySettings;

    #[test]
    fn sync_root_must_be_absolute() {
        let settings = InMemorySettings::default();
        let set = SetSyncRoot::new(&settings);

        assert!(matches!(
            set.execute("relative/dir".into()),
            Err(AppError::InvalidSyncRoot(_))
        ));
        set.execute("/data/drive".into()).unwrap();
        assert_eq!(
            settings.load().unwrap().sync_root,
            PathBuf::from("/data/drive")
        );
    }
}
