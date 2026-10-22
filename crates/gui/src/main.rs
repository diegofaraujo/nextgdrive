//! `nextgdrive-gui`: GTK4 + libadwaita interface.
//!
//! GTK runs on the main thread; use cases run on a background Tokio runtime
//! (see [`runtime`]) and their results are awaited back on the main thread.

mod accounts_page;

use std::sync::{Arc, OnceLock};

use adw::prelude::*;
use gtk::glib;
use nextgdrive_composition::Services;
use nextgdrive_i18n::Localizer;

const APP_ID: &str = "io.github.diegofaraujo.NextGDrive";

/// Runtime for network and keyring work, kept off the GTK main thread.
pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("failed to start the async runtime")
    })
}

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
    let header = adw::HeaderBar::new();
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(i18n.tr("app-name"))
        .default_width(640)
        .default_height(560)
        .build();

    // May show the system prompt to unlock the keyring.
    let content = match runtime().block_on(Services::new(&i18n)) {
        Ok(services) => accounts_page::build(i18n, Arc::new(services), &header),
        Err(message) => adw::StatusPage::builder()
            .icon_name("dialog-error-symbolic")
            .description(message)
            .build()
            .upcast(),
    };

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));
    window.set_content(Some(&toolbar));
    window.present();
}
