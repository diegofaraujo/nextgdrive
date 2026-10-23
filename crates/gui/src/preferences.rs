use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::autostart;
use crate::ctx::Ctx;

/// App-wide settings: for now, where synced accounts live.
pub fn present(ctx: &Rc<Ctx>) {
    let i18n = &ctx.i18n;
    let root_row = adw::ActionRow::builder()
        .title(i18n.tr("gui-sync-root"))
        .use_markup(false)
        .build();
    let change = gtk::Button::builder()
        .label(i18n.tr("gui-sync-root-change"))
        .valign(gtk::Align::Center)
        .build();
    root_row.add_suffix(&change);
    match ctx.services.local.get_settings().execute() {
        Ok(settings) => root_row.set_subtitle(&settings.sync_root.display().to_string()),
        Err(e) => root_row.set_subtitle(&i18n.app_error(&e)),
    }

    let group = adw::PreferencesGroup::builder()
        .title(i18n.tr("gui-sync-group"))
        .description(i18n.tr("gui-sync-root-note"))
        .build();
    group.add(&root_row);
    let autostart_row = adw::SwitchRow::builder()
        .title(i18n.tr("gui-autostart"))
        .subtitle(i18n.tr("gui-autostart-subtitle"))
        .active(autostart::is_enabled())
        .build();
    let ctx_for_autostart = ctx.clone();
    autostart_row.connect_active_notify(move |row| {
        if let Err(e) = autostart::set_enabled(row.is_active()) {
            ctx_for_autostart.toast(&e.to_string());
        }
    });
    let notifications_row = adw::SwitchRow::builder()
        .title(i18n.tr("gui-notifications"))
        .subtitle(i18n.tr("gui-notifications-subtitle"))
        .active(
            ctx.services
                .local
                .get_settings()
                .execute()
                .is_ok_and(|s| s.notifications),
        )
        .build();
    let ctx_for_notifications = ctx.clone();
    notifications_row.connect_active_notify(move |row| {
        let ctx = &ctx_for_notifications;
        if let Err(e) = ctx
            .services
            .local
            .set_notifications()
            .execute(row.is_active())
        {
            ctx.toast(&ctx.i18n.app_error(&e));
        }
    });
    let background = adw::PreferencesGroup::builder()
        .title(i18n.tr("gui-background-group"))
        .build();
    background.add(&autostart_row);
    background.add(&notifications_row);

    let page = adw::PreferencesPage::new();
    page.add(&group);
    page.add(&background);
    let dialog = adw::PreferencesDialog::new();
    dialog.add(&page);

    let ctx_for_change = ctx.clone();
    let (row, dialog_ref) = (root_row.downgrade(), dialog.downgrade());
    change.connect_clicked(move |_| {
        let ctx = ctx_for_change.clone();
        let (row, dialog_ref) = (row.clone(), dialog_ref.clone());
        glib::spawn_future_local(async move {
            let chooser = gtk::FileDialog::builder()
                .title(ctx.i18n.tr("gui-sync-root"))
                .modal(true)
                .build();
            let window = dialog_ref
                .upgrade()
                .and_then(|d| d.root())
                .and_downcast::<gtk::Window>();
            // Cancelling the chooser is not an error worth reporting.
            let Ok(folder) = chooser.select_folder_future(window.as_ref()).await else {
                return;
            };
            let Some(path) = folder.path() else { return };
            match ctx.services.local.set_sync_root().execute(path.clone()) {
                Ok(()) => {
                    if let Some(row) = row.upgrade() {
                        row.set_subtitle(&path.display().to_string());
                    }
                }
                Err(e) => ctx.toast(&ctx.i18n.app_error(&e)),
            }
        });
    });
    dialog.present(ctx.window().as_ref());
}
