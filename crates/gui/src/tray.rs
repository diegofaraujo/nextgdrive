//! System tray icon (StatusNotifierItem), shown while background sync runs.

use std::sync::Arc;

use gtk::{gdk_pixbuf, gio, glib};
use ksni::menu::StandardItem;
use ksni::{MenuItem, ToolTip, TrayMethods};
use nextgdrive_adapter_dbus::{HostStatus, STATE_PAUSED, STATE_SYNCING};
use nextgdrive_composition::host::{self, SyncHost};
use nextgdrive_i18n::Localizer;
use tokio::sync::mpsc;

use crate::APP_ID;
use crate::ctx::runtime;

/// The project's tray icon, drawn in its own color while idle or syncing.
const ICON: &str = include_str!("../../../data/icons/tray.svg");
const ICON_COLOR: &str = "#38BDF8";
const PAUSED_COLOR: &str = "#9CA3AF";
const PROBLEM_COLOR: &str = "#F59E0B";
/// Sizes offered to the panel, which picks the closest.
const ICON_SIZES: [i32; 6] = [16, 22, 24, 32, 48, 64];

/// The tray icon per state, as pixmaps: an icon name would need the icon
/// installed in the theme. Empty if SVG could not be loaded.
struct Icons {
    normal: Vec<ksni::Icon>,
    paused: Vec<ksni::Icon>,
    problem: Vec<ksni::Icon>,
}

impl Icons {
    /// Uses gdk-pixbuf, so it must run where GTK is initialized.
    fn render() -> Self {
        Self {
            normal: render(ICON_COLOR),
            paused: render(PAUSED_COLOR),
            problem: render(PROBLEM_COLOR),
        }
    }
}

fn render(color: &str) -> Vec<ksni::Icon> {
    let svg = glib::Bytes::from_owned(ICON.replace(ICON_COLOR, color).into_bytes());
    ICON_SIZES
        .iter()
        .filter_map(|&size| {
            let stream = gio::MemoryInputStream::from_bytes(&svg);
            let pixbuf = gdk_pixbuf::Pixbuf::from_stream_at_scale(
                &stream,
                size,
                size,
                true,
                gio::Cancellable::NONE,
            )
            .ok()?;
            Some(to_argb(&pixbuf))
        })
        .collect()
}

/// RGBA rows (with padding) to the ARGB32, network byte order, that
/// StatusNotifierItem expects.
fn to_argb(pixbuf: &gdk_pixbuf::Pixbuf) -> ksni::Icon {
    let (width, height) = (pixbuf.width(), pixbuf.height());
    let channels = usize::try_from(pixbuf.n_channels()).unwrap_or(4);
    let stride = usize::try_from(pixbuf.rowstride()).unwrap_or(0);
    let bytes = pixbuf.read_pixel_bytes();
    let mut data = Vec::with_capacity(usize::try_from(width * height * 4).unwrap_or(0));
    for row in 0..usize::try_from(height).unwrap_or(0) {
        for column in 0..usize::try_from(width).unwrap_or(0) {
            let at = row * stride + column * channels;
            let Some(pixel) = bytes.get(at..at + channels) else {
                break;
            };
            let alpha = if channels == 4 { pixel[3] } else { u8::MAX };
            data.extend_from_slice(&[alpha, pixel[0], pixel[1], pixel[2]]);
        }
    }
    ksni::Icon {
        width,
        height,
        data,
    }
}

/// Tray requests that must run on the GTK main thread.
pub enum TrayAction {
    Open,
    Quit,
}

struct Tray {
    i18n: Localizer,
    host: Arc<SyncHost>,
    status: HostStatus,
    actions: mpsc::UnboundedSender<TrayAction>,
    icons: Icons,
}

impl Tray {
    fn pixmap(&self) -> &[ksni::Icon] {
        match self.status.state.as_str() {
            STATE_PAUSED => &self.icons.paused,
            STATE_SYNCING => &self.icons.normal,
            _ if !self.status.errors.is_empty() => &self.icons.problem,
            _ => &self.icons.normal,
        }
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        APP_ID.into()
    }

    fn title(&self) -> String {
        self.i18n.tr("app-name")
    }

    /// Only if the project's icon could not be drawn.
    fn icon_name(&self) -> String {
        if !self.pixmap().is_empty() {
            return String::new();
        }
        match self.status.state.as_str() {
            STATE_SYNCING => "emblem-synchronizing-symbolic",
            STATE_PAUSED => "media-playback-pause-symbolic",
            _ if !self.status.errors.is_empty() => "dialog-warning-symbolic",
            _ => "folder-remote-symbolic",
        }
        .into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.pixmap().to_vec()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: self.i18n.tr("app-name"),
            description: host::describe(&self.i18n, &self.status),
            ..ToolTip::default()
        }
    }

    /// Left click opens the window.
    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.actions.send(TrayAction::Open);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let i18n = &self.i18n;
        let status = host::describe(i18n, &self.status);
        let mut items: Vec<MenuItem<Self>> = status
            .lines()
            .map(|line| {
                StandardItem {
                    label: line.replace('_', "__"),
                    enabled: false,
                    ..StandardItem::default()
                }
                .into()
            })
            .collect();
        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: i18n.tr("tray-open"),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.actions.send(TrayAction::Open);
                }),
                ..StandardItem::default()
            }
            .into(),
        );
        items.push(
            StandardItem {
                label: i18n.tr("tray-sync-now"),
                enabled: self.status.state != STATE_SYNCING,
                activate: Box::new(|t: &mut Self| t.host.sync_now()),
                ..StandardItem::default()
            }
            .into(),
        );
        let paused = self.status.state == STATE_PAUSED;
        items.push(
            StandardItem {
                label: i18n.tr(if paused { "tray-resume" } else { "tray-pause" }),
                activate: Box::new(move |t: &mut Self| {
                    if paused {
                        t.host.resume();
                    } else {
                        t.host.pause();
                    }
                }),
                ..StandardItem::default()
            }
            .into(),
        );
        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: i18n.tr("tray-quit"),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.actions.send(TrayAction::Quit);
                }),
                ..StandardItem::default()
            }
            .into(),
        );
        items
    }
}

/// Shows the tray icon and keeps it in step with the host's status. Returns
/// the requests the GTK side must handle, or `None` if no tray is available.
pub fn start(i18n: Localizer, host: Arc<SyncHost>) -> Option<mpsc::UnboundedReceiver<TrayAction>> {
    let (actions, receiver) = mpsc::unbounded_channel();
    let mut status = host.status();
    let tray = Tray {
        i18n,
        status: status.borrow().clone(),
        host,
        actions,
        icons: Icons::render(),
    };
    // A sandbox may not own the item's well-known name; watchers accept the
    // unique connection name instead.
    let handle = runtime()
        .block_on(tray.disable_dbus_name(ashpd::is_sandboxed()).spawn())
        .ok()?;
    runtime().spawn(async move {
        while status.changed().await.is_ok() {
            let current = status.borrow_and_update().clone();
            if handle.update(|tray| tray.status = current).await.is_none() {
                return; // Tray gone.
            }
        }
    });
    Some(receiver)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_is_drawn_in_each_color() {
        let normal = render(ICON_COLOR);
        assert_eq!(normal.len(), ICON_SIZES.len());
        let icon = normal.iter().find(|i| i.width == 24).unwrap();
        assert_eq!(icon.data.len(), 24 * 24 * 4);
        // Some pixel is opaque and sky blue (#38BDF8), in ARGB order.
        assert!(
            icon.data
                .chunks(4)
                .any(|p| p[0] == 255 && (p[1], p[2], p[3]) == (0x38, 0xBD, 0xF8))
        );
        assert_ne!(render(PROBLEM_COLOR)[0].data, normal[0].data);
    }
}
