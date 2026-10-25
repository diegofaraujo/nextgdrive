use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::autostart;
use crate::ctx::{Ctx, spawn, wake_host};

/// Limits offered for the cache of streamed files.
const CACHE_LIMITS: [u64; 6] = [1 << 30, 2 << 30, 5 << 30, 10 << 30, 20 << 30, 50 << 30];

/// App-wide settings: where synced accounts live, background behavior, and
/// the cache of streamed files.
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
    // Set back to what took effect, without asking again.
    let applying = std::rc::Rc::new(std::cell::Cell::new(false));
    autostart_row.connect_active_notify(move |row| {
        if applying.get() {
            return;
        }
        let ctx = ctx_for_autostart.clone();
        let wanted = row.is_active();
        let reason = ctx.i18n.tr("gui-autostart-reason");
        let (row, applying) = (row.downgrade(), applying.clone());
        spawn(autostart::set_enabled(wanted, reason), move |result| {
            let enabled = match result {
                Ok(enabled) => enabled,
                Err(details) => {
                    let message = ctx
                        .i18n
                        .tr_args("gui-autostart-failed", [("details", details.into())]);
                    ctx.toast(&message);
                    !wanted
                }
            };
            if let Some(row) = row.upgrade()
                && row.is_active() != enabled
            {
                applying.set(true);
                row.set_active(enabled);
                applying.set(false);
            }
        });
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
    page.add(&cache_group(ctx, &dialog));
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

/// Space used by streamed files, its limit, and clearing it.
fn cache_group(ctx: &Rc<Ctx>, dialog: &adw::PreferencesDialog) -> adw::PreferencesGroup {
    let i18n = &ctx.i18n;
    let usage = adw::ActionRow::builder()
        .title(i18n.tr("gui-cache-usage"))
        .subtitle(i18n.tr("gui-cache-calculating"))
        .build();
    show_usage(ctx, &usage);

    let mut limits = CACHE_LIMITS.to_vec();
    let current = ctx
        .services
        .local
        .get_settings()
        .execute()
        .map_or(nextgdrive_domain::Settings::DEFAULT_CACHE_LIMIT, |s| {
            s.cache_limit
        });
    // Set from the CLI to something not offered here.
    if !limits.contains(&current) {
        limits.push(current);
        limits.sort_unstable();
    }
    let names: Vec<String> = limits.iter().map(|l| i18n.bytes(*l)).collect();
    let limit = adw::ComboRow::builder()
        .title(i18n.tr("gui-cache-limit"))
        .subtitle(i18n.tr("gui-cache-limit-subtitle"))
        .model(&gtk::StringList::new(
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        ))
        .selected(
            limits
                .iter()
                .position(|l| *l == current)
                .and_then(|p| u32::try_from(p).ok())
                .unwrap_or(0),
        )
        .build();
    let (ctx_for_limit, usage_ref) = (ctx.clone(), usage.downgrade());
    limit.connect_selected_notify(move |row| {
        let ctx = &ctx_for_limit;
        let Some(bytes) = limits.get(row.selected() as usize) else {
            return;
        };
        match ctx.services.local.set_cache_limit().execute(*bytes) {
            Ok(_) => {
                // The background process applies it on its next round.
                let usage_ref = usage_ref.clone();
                let ctx_for_usage = ctx.clone();
                spawn(wake_host(), move |()| {
                    if let Some(usage) = usage_ref.upgrade() {
                        show_usage(&ctx_for_usage, &usage);
                    }
                });
            }
            Err(e) => ctx.toast(&ctx.i18n.app_error(&e)),
        }
    });

    let clear_row = adw::ActionRow::builder()
        .title(i18n.tr("gui-cache-clear"))
        .subtitle(i18n.tr("gui-cache-clear-subtitle"))
        .build();
    let clear = gtk::Button::builder()
        .label(i18n.tr("gui-cache-clear-action"))
        .valign(gtk::Align::Center)
        .css_classes(["destructive-action"])
        .build();
    clear_row.add_suffix(&clear);
    let (ctx_for_clear, usage_ref, dialog_ref) =
        (ctx.clone(), usage.downgrade(), dialog.downgrade());
    clear.connect_clicked(move |_| {
        let ctx = ctx_for_clear.clone();
        let i18n = &ctx.i18n;
        let confirm = adw::AlertDialog::new(
            Some(&i18n.tr("gui-cache-clear-title")),
            Some(&i18n.tr("gui-cache-clear-body")),
        );
        confirm.add_responses(&[
            ("cancel", &i18n.tr("gui-cancel")),
            ("clear", &i18n.tr("gui-cache-clear-action")),
        ]);
        confirm.set_response_appearance("clear", adw::ResponseAppearance::Destructive);
        confirm.set_default_response(Some("cancel"));
        confirm.set_close_response("cancel");
        let (usage_ref, shown_in) = (usage_ref.clone(), dialog_ref.clone());
        confirm.connect_response(None, move |_, response| {
            if response == "clear" {
                clear_cache(&ctx, usage_ref.clone(), shown_in.clone());
            }
        });
        confirm.present(dialog_ref.upgrade().as_ref());
    });

    let group = adw::PreferencesGroup::builder()
        .title(i18n.tr("gui-cache-group"))
        .description(i18n.tr("gui-cache-description"))
        .build();
    group.add(&usage);
    group.add(&limit);
    group.add(&clear_row);
    group
}

/// Fills in the usage row (it walks the offline folders in Drive).
fn show_usage(ctx: &Rc<Ctx>, row: &adw::ActionRow) {
    let services = ctx.services.clone();
    let i18n = ctx.i18n.clone();
    let row = row.downgrade();
    spawn(
        async move {
            let error = |e: nextgdrive_application::AppError| i18n.app_error(&e);
            let settings = services.local.get_settings().execute().map_err(error)?;
            let accounts = services.local.list_accounts().execute().map_err(error)?;
            let cache = nextgdrive_composition::stream::open_cache(&i18n, settings.cache_limit)?;
            let usage = services
                .offline(&cache)
                .cache_usage(&accounts)
                .await
                .map_err(error)?;
            Ok::<_, String>(i18n.tr_args(
                "cache-usage",
                [
                    ("used", i18n.bytes(usage.used).into()),
                    ("limit", i18n.bytes(settings.cache_limit).into()),
                    ("offline", i18n.bytes(usage.offline).into()),
                ],
            ))
        },
        move |result| {
            if let Some(row) = row.upgrade() {
                row.set_subtitle(&result.unwrap_or_else(|message| message));
            }
        },
    );
}

fn clear_cache(
    ctx: &Rc<Ctx>,
    usage: glib::WeakRef<adw::ActionRow>,
    dialog: glib::WeakRef<adw::PreferencesDialog>,
) {
    let services = ctx.services.clone();
    let i18n = ctx.i18n.clone();
    let ctx = ctx.clone();
    spawn(
        async move {
            let error = |e: nextgdrive_application::AppError| i18n.app_error(&e);
            let settings = services.local.get_settings().execute().map_err(error)?;
            let accounts = services.local.list_accounts().execute().map_err(error)?;
            let cache = nextgdrive_composition::stream::open_cache(&i18n, settings.cache_limit)?;
            let freed = services
                .offline(&cache)
                .clear_cache(&accounts)
                .await
                .map_err(error)?;
            Ok::<_, String>(i18n.tr_args("cache-cleared", [("freed", i18n.bytes(freed).into())]))
        },
        move |result| {
            let message = result.unwrap_or_else(|message| message);
            match dialog.upgrade() {
                Some(dialog) => dialog.add_toast(adw::Toast::new(&message)),
                None => ctx.toast(&message),
            }
            if let Some(usage) = usage.upgrade() {
                show_usage(&ctx, &usage);
            }
        },
    );
}
