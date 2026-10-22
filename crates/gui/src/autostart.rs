//! Starting with the session, hidden in the system tray, through an XDG
//! autostart entry (`~/.config/autostart`).

use std::fs;
use std::io;
use std::path::PathBuf;

use gtk::glib;

use crate::APP_ID;

/// Command-line flag: start without opening the window.
pub const BACKGROUND_FLAG: &str = "--background";

fn entry_path() -> PathBuf {
    glib::user_config_dir()
        .join("autostart")
        .join(format!("{APP_ID}.desktop"))
}

pub fn is_enabled() -> bool {
    entry_path().is_file()
}

pub fn set_enabled(enabled: bool) -> io::Result<()> {
    let path = entry_path();
    if !enabled {
        return match fs::remove_file(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        };
    }
    let exe = std::env::current_exe()?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&path, entry(&exe.to_string_lossy()))
}

/// Quoting per the Desktop Entry spec, so paths with spaces work.
fn entry(exe: &str) -> String {
    let quoted = exe
        .replace('\\', "\\\\\\\\")
        .replace('"', "\\\\\"")
        .replace('`', "\\\\`")
        .replace('$', "\\\\$");
    format!(
        "[Desktop Entry]\nType=Application\nName=Next GDrive\nExec=\"{quoted}\" {BACKGROUND_FLAG}\n\
         Icon=folder-remote\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_line_quotes_the_path() {
        let entry = entry("/home/ana/my apps/next$gdrive");
        assert!(
            entry.contains("Exec=\"/home/ana/my apps/next\\\\$gdrive\" --background\n"),
            "{entry}"
        );
    }
}
