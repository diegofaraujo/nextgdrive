use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use nextgdrive_application::AppError;
use nextgdrive_application::use_cases::TrashEntry;
use nextgdrive_domain::{Account, ItemKind};

use crate::ctx::{Ctx, keep_alive, navigation_page, spawn};

/// An account's Google Drive trash, with a Restore button per item.
struct TrashPage {
    ctx: Rc<Ctx>,
    account: Account,
    stack: gtk::Stack,
    group: adw::PreferencesGroup,
    status: adw::StatusPage,
    /// Items still listed; the empty state shows when it reaches zero.
    remaining: Cell<usize>,
}

pub fn build(ctx: &Rc<Ctx>, account: Account) -> adw::NavigationPage {
    let i18n = &ctx.i18n;
    let group = adw::PreferencesGroup::builder()
        .description(i18n.tr("gui-trash-subtitle"))
        .build();
    let list = adw::PreferencesPage::new();
    list.add(&group);
    let loading = adw::StatusPage::builder()
        .child(
            &gtk::Spinner::builder()
                .spinning(true)
                .width_request(32)
                .height_request(32)
                .build(),
        )
        .build();
    let status = adw::StatusPage::new();
    let stack = gtk::Stack::new();
    stack.add_named(&loading, Some("loading"));
    stack.add_named(&list, Some("list"));
    stack.add_named(&status, Some("status"));
    stack.set_visible_child_name("loading");

    let root = navigation_page(&i18n.tr("gui-trash"), &adw::HeaderBar::new(), &stack);
    let page = Rc::new(TrashPage {
        ctx: ctx.clone(),
        account,
        stack,
        group,
        status,
        remaining: Cell::new(0),
    });
    page.load();
    keep_alive(&root, page);
    root
}

impl TrashPage {
    fn load(self: &Rc<Self>) {
        let services = self.ctx.services.clone();
        let account = self.account.clone();
        let weak = Rc::downgrade(self);
        spawn(
            async move { services.drive_trash().list(&account).await },
            move |result: Result<Vec<TrashEntry>, AppError>| {
                let Some(page) = weak.upgrade() else { return };
                let i18n = &page.ctx.i18n;
                match result {
                    Err(e) => page.show_status(
                        "dialog-error-symbolic",
                        &i18n.tr("gui-load-error-title"),
                        &i18n.app_error(&e),
                    ),
                    Ok(entries) if entries.is_empty() => page.show_empty(),
                    Ok(entries) => {
                        page.remaining.set(entries.len());
                        for entry in entries {
                            page.group.add(&page.row(entry));
                        }
                        page.stack.set_visible_child_name("list");
                    }
                }
            },
        );
    }

    fn show_status(&self, icon: &str, title: &str, description: &str) {
        self.status.set_icon_name(Some(icon));
        self.status.set_title(title);
        self.status.set_description(Some(description));
        self.stack.set_visible_child_name("status");
    }

    fn show_empty(&self) {
        let i18n = &self.ctx.i18n;
        self.show_status(
            "user-trash-symbolic",
            &i18n.tr("gui-trash-empty-title"),
            &i18n.tr("gui-trash-empty-description"),
        );
    }

    fn row(self: &Rc<Self>, entry: TrashEntry) -> adw::ActionRow {
        let i18n = &self.ctx.i18n;
        let location = entry
            .location
            .clone()
            .unwrap_or_else(|| i18n.tr("trash-location-unknown"));
        let subtitle = match entry.trashed_at {
            Some(at) => i18n.tr_args(
                "gui-trash-entry-dated",
                [
                    ("location", location.into()),
                    ("date", i18n.date_time(at).into()),
                ],
            ),
            None => i18n.tr_args("gui-trash-entry", [("location", location.into())]),
        };
        let row = adw::ActionRow::builder()
            .title(&entry.item.name)
            .subtitle(subtitle)
            // Names are user data, not Pango markup.
            .use_markup(false)
            .build();
        let icon = match entry.item.kind {
            ItemKind::Folder => "folder-symbolic",
            ItemKind::GoogleDoc { .. } => "x-office-document-symbolic",
            _ => "text-x-generic-symbolic",
        };
        row.add_prefix(&gtk::Image::from_icon_name(icon));
        let restore = gtk::Button::builder()
            .label(i18n.tr("gui-trash-restore"))
            .valign(gtk::Align::Center)
            .build();
        row.add_suffix(&restore);

        let (weak, row_ref) = (Rc::downgrade(self), row.downgrade());
        restore.connect_clicked(move |button| {
            let Some(page) = weak.upgrade() else { return };
            button.set_sensitive(false);
            let services = page.ctx.services.clone();
            let (account, id) = (page.account.clone(), entry.item.id.clone());
            let (weak, row_ref, button) = (Rc::downgrade(&page), row_ref.clone(), button.clone());
            let location = entry.location.clone();
            spawn(
                async move { services.drive_trash().restore(&account, &id).await },
                move |result| {
                    let Some(page) = weak.upgrade() else { return };
                    let i18n = &page.ctx.i18n;
                    match result {
                        Ok(item) => {
                            let message = match location {
                                Some(location) => i18n.tr_args(
                                    "trash-restored",
                                    [("name", item.name.into()), ("location", location.into())],
                                ),
                                None => i18n.tr_args(
                                    "trash-restored-unknown",
                                    [("name", item.name.into())],
                                ),
                            };
                            page.ctx.toast(&message);
                            if let Some(row) = row_ref.upgrade() {
                                page.group.remove(&row);
                            }
                            page.remaining.set(page.remaining.get().saturating_sub(1));
                            if page.remaining.get() == 0 {
                                page.show_empty();
                            }
                        }
                        Err(e) => {
                            button.set_sensitive(true);
                            page.ctx.toast(&i18n.app_error(&e));
                        }
                    }
                },
            );
        });
        row
    }
}
