//! Starting with the session, hidden in the system tray, through an XDG
//! autostart entry (`~/.config/autostart`). In a sandbox (Flatpak) the app
//! cannot write it: the Background portal does, after asking the user.

use std::fs;
use std::io;
use std::path::PathBuf;

use gtk::glib;

use crate::APP_ID;

/// Command-line flag: start without opening the window.
pub const BACKGROUND_FLAG: &str = "--background";

/// The entry, in the real `~/.config` also from a sandbox (whose
/// `XDG_CONFIG_HOME` is its own), where the portal writes it under the same name.
fn entry_path() -> PathBuf {
    let config = if ashpd::is_sandboxed() {
        glib::home_dir().join(".config")
    } else {
        glib::user_config_dir()
    };
    config.join("autostart").join(format!("{APP_ID}.desktop"))
}

pub fn is_enabled() -> bool {
    entry_path().is_file()
}

/// Returns whether the app now starts with the session: the portal may be
/// refused by the user. `reason` is shown in the portal's dialog.
pub async fn set_enabled(enabled: bool, reason: String) -> Result<bool, String> {
    if ashpd::is_sandboxed() {
        let response = ashpd::desktop::background::Background::request()
            .reason(reason.as_str())
            .auto_start(enabled)
            .command(["nextgdrive-gui", BACKGROUND_FLAG])
            .dbus_activatable(false)
            .send()
            .await
            .and_then(|request| request.response())
            .map_err(|e| e.to_string())?;
        return Ok(response.auto_start());
    }
    write_entry(enabled).map_err(|e| e.to_string())?;
    Ok(enabled)
}

fn write_entry(enabled: bool) -> io::Result<()> {
    let path = entry_path();
    if !enabled {
        return match fs::remove_file(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        };
    }
    // An AppImage runs from a mount that is gone after it exits: start the
    // AppImage file itself.
    let exe = match std::env::var_os("APPIMAGE") {
        Some(appimage) => PathBuf::from(appimage),
        None => std::env::current_exe()?,
    };
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
         Icon={APP_ID}\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n"
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
