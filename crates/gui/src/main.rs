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
mod preferences;
mod tray;

use std::cell::Cell;
use std::sync::Arc;

use adw::prelude::*;
use gtk::glib;
use nextgdrive_composition::Services;
use nextgdrive_composition::host::{StartError, SyncHost};
use nextgdrive_i18n::Localizer;

const APP_ID: &str = "io.github.diegofaraujo.NextGDrive";

fn main() -> glib::ExitCode {
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
    let host = ctx::runtime().block_on(SyncHost::start(
        services.clone(),
        i18n.clone(),
        Box::new(|_| {}),
    ));

    let nav = adw::NavigationView::new();
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&nav));
    window.set_content(Some(&toasts));
    let ctx = ctx::Ctx::new(i18n.clone(), services, &nav, &toasts);
    nav.add(&accounts_page::build(&ctx));

    let tray = match host {
        Ok(host) => tray::start(i18n, Arc::new(host)),
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
