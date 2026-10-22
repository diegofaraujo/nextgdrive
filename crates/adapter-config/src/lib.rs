//! Configuration storage adapters, backed by TOML files in the XDG config directory.

mod accounts;

pub use accounts::TomlAccountRepository;

use std::path::PathBuf;

const APP_DIR: &str = "nextgdrive";

/// `$XDG_CONFIG_HOME/nextgdrive`, usually `~/.config/nextgdrive`.
pub fn config_dir() -> Option<PathBuf> {
    xdg::BaseDirectories::with_prefix(APP_DIR).get_config_home()
}
