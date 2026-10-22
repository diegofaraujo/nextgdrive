//! Configuration storage adapters, backed by TOML files in the XDG config directory.

mod accounts;
mod settings;

pub use accounts::TomlAccountRepository;
pub use settings::TomlSettingsRepository;

use std::path::{Path, PathBuf};
use std::{fs, io};

const APP_DIR: &str = "nextgdrive";

/// `$XDG_CONFIG_HOME/nextgdrive`, usually `~/.config/nextgdrive`.
pub fn config_dir() -> Option<PathBuf> {
    xdg::BaseDirectories::with_prefix(APP_DIR).get_config_home()
}

/// Writes to a temporary file first, so a crash never leaves a truncated file behind.
fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}
