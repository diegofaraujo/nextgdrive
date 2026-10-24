//! `nextgdrive-gui`: GTK4 + libadwaita interface, living in the system tray
//! while it keeps accounts in sync.
//!
//! GTK runs on the main thread; use cases and background sync run on a Tokio
//! runtime (see [`ctx::runtime`]) and their results come back to the main thread.

mod account_page;
mod accounts_page;
mod autostart;
mod ctx;
mod folders_page;
mod notifications;
mod offline_page;
mod preferences;
mod trash_page;
mod tray;

use std::cell::Cell;
use std::sync::Arc;

use adw::prelude::*;
use gtk::glib;
use nextgdrive_composition::Services;
use nextgdrive_composition::host::{Notice, StartError, SyncHost};
use nextgdrive_i18n::Localizer;

const APP_ID: &str = "io.github.diegofaraujo.NextGDrive";

fn main() -> glib::ExitCode {
    // GtkApplication adds the resources' `icons/` folder to the icon theme,
    // so the app icon is found without being installed.
    gtk::gio::resources_register_include!("nextgdrive.gresource")
        .expect("embedded resources are valid");
    let hidden = std::env::args().any(|arg| arg == autostart::BACKGROUND_FLAG);
    let app = adw::Application::builder().application_id(APP_ID).build();
    // Only the first activation may start hidden; later ones (launching the
    // app again, clicking the tray icon) show the window.
    let first = Cell::new(true);
    app.connect_activate(move |app| activate(app, first.replace(false) && hidden));
    // GApplication would reject our own flag.
    app.run_with_args::<&str>(&[])
}

fn activate(app: &adw::Application, hidden: bool) {
    if let Some(window) = app.windows().into_iter().next() {
        window.present();
        return;
    }

    let i18n = Localizer::from_system();
    gtk::Window::set_default_icon_name(APP_ID);
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(i18n.tr("app-name"))
        .default_width(640)
        .default_height(620)
        .build();

    // May show the system prompt to unlock the keyring.
    let services = match ctx::runtime().block_on(Services::new(&i18n)) {
        Ok(services) => Arc::new(services),
        Err(message) => return show_error(&window, &message),
    };
    // Notices come from the sync thread; notifications are sent from GTK's.
    let (notices_tx, mut notices) = tokio::sync::mpsc::unbounded_channel::<Notice>();
    let notify = Box::new(move |notice: Notice| {
        let _ = notices_tx.send(notice);
    });
    let host = ctx::runtime().block_on(SyncHost::start(
        services.clone(),
        i18n.clone(),
        Box::new(|_| {}),
        notify,
    ));
    let (app_ref, notifying) = (app.downgrade(), services.clone());
    glib::spawn_future_local(async move {
        while let Some(notice) = notices.recv().await {
            let enabled = notifying
                .local
                .get_settings()
                .execute()
                .is_ok_and(|s| s.notifications);
            if let (true, Some(app)) = (enabled, app_ref.upgrade()) {
                notifications::send(&app, &notice);
            }
        }
    });
    let show = gtk::gio::SimpleAction::new("show-window", None);
    let shown = window.downgrade();
    show.connect_activate(move |_, _| {
        if let Some(window) = shown.upgrade() {
            window.present();
        }
    });
    app.add_action(&show);

    let nav = adw::NavigationView::new();
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&nav));
    window.set_content(Some(&toasts));
    let ctx = ctx::Ctx::new(i18n.clone(), services, &nav, &toasts);
    nav.add(&accounts_page::build(&ctx));

    let host = host.map(Arc::new);
    if let Ok(host) = &host {
        // However the app ends, streamed folders are unmounted first; a
        // process that exits without that leaves them dead ("not connected").
        let stopping = host.clone();
        app.connect_shutdown(move |_| ctx::runtime().block_on(stopping.shutdown()));
        // Ending the session sends SIGTERM: quit the normal way.
        let (terminated, mut on_terminate) = tokio::sync::mpsc::unbounded_channel::<()>();
        ctx::runtime().spawn(async move {
            use tokio::signal::unix::{SignalKind, signal};
            // SIGTERM ends the session; SIGINT is Ctrl+C in the terminal
            // that started the app.
            let (Ok(mut term), Ok(mut int)) = (
                signal(SignalKind::terminate()),
                signal(SignalKind::interrupt()),
            ) else {
                return;
            };
            tokio::select! {
                _ = term.recv() => {}
                _ = int.recv() => {}
            }
            let _ = terminated.send(());
        });
        let quitting = app.downgrade();
        glib::spawn_future_local(async move {
            if on_terminate.recv().await.is_some()
                && let Some(app) = quitting.upgrade()
            {
                app.quit();
            }
        });
    }
    let tray = match host {
        Ok(host) => tray::start(i18n, host),
        Err(StartError::AlreadyRunning) => {
            ctx.toast(&ctx.i18n.tr("gui-host-elsewhere"));
            None
        }
        Err(StartError::Bus(details)) => {
            ctx.toast(
                &ctx.i18n
                    .tr_args("error-dbus", [("details", details.into())]),
            );
            None
        }
    };
    if let Some(mut actions) = tray {
        // Closing the window leaves the app running in the tray.
        std::mem::forget(app.hold());
        window.set_hide_on_close(true);
        let (app, window_ref) = (app.clone(), window.downgrade());
        glib::spawn_future_local(async move {
            while let Some(action) = actions.recv().await {
                match action {
                    tray::TrayAction::Open => {
                        if let Some(window) = window_ref.upgrade() {
                            window.present();
                        }
                    }
                    tray::TrayAction::Quit => app.quit(),
                }
            }
        });
        if hidden {
            return;
        }
    }
    window.present();
}

fn show_error(window: &adw::ApplicationWindow, message: &str) {
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(
        &adw::StatusPage::builder()
            .icon_name("dialog-error-symbolic")
            .description(message)
            .build(),
    ));
    window.set_content(Some(&toolbar));
    window.present();
}

#[cfg(test)]
mod tests {
    use gtk::gio;

    #[test]
    fn the_app_icon_is_embedded_where_gtk_looks_for_it() {
        gio::resources_register_include!("nextgdrive.gresource").unwrap();
        let path = format!(
            "/io/github/diegofaraujo/NextGDrive/icons/hicolor/256x256/apps/{}.png",
            super::APP_ID
        );
        assert!(gio::resources_lookup_data(&path, gio::ResourceLookupFlags::NONE).is_ok());
    }
}
