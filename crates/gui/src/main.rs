//! `nextgdrive-gui`: GTK4 + libadwaita interface.
//!
//! GTK runs on the main thread; use cases run on a background Tokio runtime
//! (see [`ctx::runtime`]) and their results come back to the main thread.

mod account_page;
mod accounts_page;
mod ctx;
mod folders_page;
mod preferences;

use std::sync::Arc;

use adw::prelude::*;
use gtk::glib;
use nextgdrive_composition::Services;
use nextgdrive_i18n::Localizer;

const APP_ID: &str = "io.github.diegofaraujo.NextGDrive";

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(activate);
    app.run()
}

fn activate(app: &adw::Application) {
    // Single instance: a second launch focuses the existing window.
    if let Some(window) = app.active_window() {
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
    match ctx::runtime().block_on(Services::new(&i18n)) {
        Ok(services) => {
            let nav = adw::NavigationView::new();
            let toasts = adw::ToastOverlay::new();
            toasts.set_child(Some(&nav));
            let ctx = ctx::Ctx::new(i18n, Arc::new(services), &nav, &toasts);
            nav.add(&accounts_page::build(&ctx));
            window.set_content(Some(&toasts));
        }
        Err(message) => {
            let toolbar = adw::ToolbarView::new();
            toolbar.add_top_bar(&adw::HeaderBar::new());
            toolbar.set_content(Some(
                &adw::StatusPage::builder()
                    .icon_name("dialog-error-symbolic")
                    .description(message)
                    .build(),
            ));
            window.set_content(Some(&toolbar));
        }
    }
    window.present();
}
