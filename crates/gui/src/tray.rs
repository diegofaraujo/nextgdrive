//! System tray icon (StatusNotifierItem), shown while background sync runs.

use std::sync::Arc;

use ksni::menu::StandardItem;
use ksni::{MenuItem, ToolTip, TrayMethods};
use nextgdrive_adapter_dbus::{HostStatus, STATE_PAUSED, STATE_SYNCING};
use nextgdrive_composition::host::{self, SyncHost};
use nextgdrive_i18n::Localizer;
use tokio::sync::mpsc;

use crate::APP_ID;
use crate::ctx::runtime;

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
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        APP_ID.into()
    }

    fn title(&self) -> String {
        self.i18n.tr("app-name")
    }

    fn icon_name(&self) -> String {
        match self.status.state.as_str() {
            STATE_SYNCING => "emblem-synchronizing-symbolic",
            STATE_PAUSED => "media-playback-pause-symbolic",
            _ if !self.status.errors.is_empty() => "dialog-warning-symbolic",
            _ => "folder-remote-symbolic",
        }
        .into()
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
    };
    let handle = runtime().block_on(tray.spawn()).ok()?;
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
