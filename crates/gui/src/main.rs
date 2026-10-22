//! `nextgdrive-gui`: GTK4 + libadwaita interface. Composition root for the GUI.

mod accounts_page;

use adw::prelude::*;
use gtk::glib;
use nextgdrive_adapter_config::TomlAccountRepository;
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
    let title = i18n.tr("app-name");
    let content = match TomlAccountRepository::in_config_dir() {
        Some(accounts) => accounts_page::build(i18n, accounts),
        None => adw::StatusPage::builder()
            .icon_name("dialog-error-symbolic")
            .description(i18n.tr("error-no-config-dir"))
            .build()
            .upcast(),
    };

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&content));

    adw::ApplicationWindow::builder()
        .application(app)
        .title(title)
        .default_width(640)
        .default_height(560)
        .content(&toolbar)
        .build()
        .present();
}
