use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use nextgdrive_application::AppError;
use nextgdrive_application::use_cases::LocalCopies;
use nextgdrive_domain::{Account, DriveItemId, DriveKind};
use nextgdrive_i18n::FluentValue;

use crate::ctx::{Ctx, keep_alive, navigation_page, spawn};

/// A folder the user can select and open.
struct Entry {
    id: DriveItemId,
    name: String,
    selected: bool,
    /// A shared drive, listed in its own group.
    shared: bool,
}

/// Chooses which folders of an account to sync, one level at a time: the
/// account's drives first, then the subfolders of whatever the user opens.
struct FoldersPage {
    ctx: Rc<Ctx>,
    account: Account,
    /// Inside a selected folder, everything is synced already.
    inherited: bool,
    stack: gtk::Stack,
    group: adw::PreferencesGroup,
    /// Shared drives, on the first level only; hidden while empty.
    shared_group: adw::PreferencesGroup,
    status: adw::StatusPage,
}

/// `parent`: `None` lists the drives; otherwise the folder (id, name) to list.
pub fn build(
    ctx: &Rc<Ctx>,
    account: Account,
    parent: Option<(DriveItemId, String)>,
    inherited: bool,
) -> adw::NavigationPage {
    let i18n = &ctx.i18n;
    let group = adw::PreferencesGroup::new();
    if parent.is_none() {
        group.set_description(Some(&i18n.tr("gui-folders-description")));
    } else if inherited {
        group.set_description(Some(&i18n.tr("gui-folders-inherited")));
    }
    let shared_group = adw::PreferencesGroup::builder()
        .title(i18n.tr("gui-shared-drives"))
        .description(i18n.tr("gui-shared-drives-description"))
        .visible(false)
        .build();
    let list = adw::PreferencesPage::new();
    list.add(&group);
    list.add(&shared_group);
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

    let title = match &parent {
        Some((_, name)) => name.clone(),
        None => i18n.tr("gui-folders"),
    };
    let root = navigation_page(&title, &adw::HeaderBar::new(), &stack);
    let page = Rc::new(FoldersPage {
        ctx: ctx.clone(),
        account,
        inherited,
        stack,
        group,
        shared_group,
        status,
    });
    page.load(parent.map(|(id, _)| id));
    keep_alive(&root, page);
    root
}

impl FoldersPage {
    fn load(self: &Rc<Self>, parent: Option<DriveItemId>) {
        let services = self.ctx.services.clone();
        let account = self.account.clone();
        let weak = Rc::downgrade(self);
        spawn(
            async move {
                let browse = services.browse_drive();
                match parent {
                    None => {
                        let settings = services.local.get_settings().execute()?;
                        let drives = browse.drives(&account).await?;
                        Ok(drives
                            .into_iter()
                            .map(|e| Entry {
                                shared: matches!(e.item.kind, DriveKind::Shared { .. }),
                                name: match e.item.kind {
                                    DriveKind::MyDrive => settings.my_drive_folder.clone(),
                                    DriveKind::Shared { name } => name,
                                },
                                id: e.item.root,
                                selected: e.selected,
                            })
                            .collect())
                    }
                    Some(parent) => Ok(browse
                        .folders(&account, &parent)
                        .await?
                        .into_iter()
                        .map(|e| Entry {
                            id: e.item.id,
                            name: e.item.name,
                            selected: e.selected,
                            shared: false,
                        })
                        .collect()),
                }
            },
            move |result: Result<Vec<Entry>, AppError>| {
                let Some(page) = weak.upgrade() else { return };
                page.show(result);
            },
        );
    }

    fn show(self: &Rc<Self>, result: Result<Vec<Entry>, AppError>) {
        let i18n = &self.ctx.i18n;
        match result {
            Err(e) => {
                self.status.set_icon_name(Some("dialog-error-symbolic"));
                self.status.set_title(&i18n.tr("gui-load-error-title"));
                self.status.set_description(Some(&i18n.app_error(&e)));
                self.stack.set_visible_child_name("status");
            }
            Ok(entries) if entries.is_empty() => {
                self.status.set_icon_name(Some("folder-symbolic"));
                self.status.set_title(&i18n.tr("gui-no-subfolders"));
                self.stack.set_visible_child_name("status");
            }
            Ok(entries) => {
                for entry in entries {
                    let group = if entry.shared {
                        self.shared_group.set_visible(true);
                        &self.shared_group
                    } else {
                        &self.group
                    };
                    group.add(&self.row(entry));
                }
                self.stack.set_visible_child_name("list");
            }
        }
    }

    fn row(self: &Rc<Self>, entry: Entry) -> adw::ActionRow {
        let check = gtk::CheckButton::builder()
            .active(entry.selected || self.inherited)
            .sensitive(!self.inherited)
            .valign(gtk::Align::Center)
            .build();
        let row = adw::ActionRow::builder()
            .title(&entry.name)
            // Folder names are user data, not Pango markup.
            .use_markup(false)
            .activatable(true)
            .build();
        row.add_prefix(&check);
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));

        // Set while undoing a failed change, so the undo is not sent again.
        let reverting = Rc::new(Cell::new(false));
        let weak = Rc::downgrade(self);
        let (id, name) = (entry.id.clone(), entry.name.clone());
        check.connect_toggled(move |check| {
            if reverting.get() {
                return;
            }
            if let Some(page) = weak.upgrade() {
                page.toggled(check, &id, &name, reverting.clone());
            }
        });

        let weak = Rc::downgrade(self);
        let check_ref = check.downgrade();
        row.connect_activated(move |_| {
            let Some(page) = weak.upgrade() else { return };
            let selected = check_ref.upgrade().is_some_and(|c| c.is_active());
            let child = build(
                &page.ctx,
                page.account.clone(),
                Some((entry.id.clone(), entry.name.clone())),
                page.inherited || selected,
            );
            page.ctx.push(&child);
        });
        row
    }

    fn toggled(
        self: &Rc<Self>,
        check: &gtk::CheckButton,
        id: &DriveItemId,
        name: &str,
        reverting: Rc<Cell<bool>>,
    ) {
        if check.is_active() {
            self.select(check, id, reverting);
        } else {
            self.ask_deselect(check, id, name, reverting);
        }
    }

    fn select(
        self: &Rc<Self>,
        check: &gtk::CheckButton,
        id: &DriveItemId,
        reverting: Rc<Cell<bool>>,
    ) {
        let services = self.ctx.services.clone();
        let (account, id) = (self.account.clone(), id.clone());
        let (weak, check) = (Rc::downgrade(self), check.downgrade());
        check.upgrade().inspect(|c| c.set_sensitive(false));
        spawn(
            async move { services.change_selection().select(&account, &id).await },
            move |result| {
                let Some(check) = check.upgrade() else { return };
                check.set_sensitive(true);
                if let Err(e) = result {
                    revert(&check, &reverting);
                    if let Some(page) = weak.upgrade() {
                        page.ctx.toast(&page.ctx.i18n.app_error(&e));
                    }
                }
            },
        );
    }

    /// Asks what to do with the downloaded files before deselecting.
    fn ask_deselect(
        self: &Rc<Self>,
        check: &gtk::CheckButton,
        id: &DriveItemId,
        name: &str,
        reverting: Rc<Cell<bool>>,
    ) {
        let i18n = &self.ctx.i18n;
        let dialog = adw::AlertDialog::new(
            Some(&i18n.tr_args("gui-deselect-title", [("name", name.to_owned().into())])),
            Some(&i18n.tr("gui-deselect-body")),
        );
        dialog.add_responses(&[
            ("cancel", &i18n.tr("gui-cancel")),
            ("keep", &i18n.tr("gui-deselect-keep")),
            ("remove", &i18n.tr("gui-deselect-remove")),
        ]);
        dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        let (weak, check_ref, id) = (Rc::downgrade(self), check.downgrade(), id.clone());
        dialog.connect_response(None, move |_, response| {
            let Some(check) = check_ref.upgrade() else {
                return;
            };
            let copies = match response {
                "keep" => LocalCopies::Keep,
                "remove" => LocalCopies::Remove,
                _ => return revert(&check, &reverting),
            };
            if let Some(page) = weak.upgrade() {
                page.deselect(&check, &id, copies, reverting.clone());
            }
        });
        dialog.present(Some(check));
    }

    fn deselect(
        self: &Rc<Self>,
        check: &gtk::CheckButton,
        id: &DriveItemId,
        copies: LocalCopies,
        reverting: Rc<Cell<bool>>,
    ) {
        let services = self.ctx.services.clone();
        let (account, id) = (self.account.clone(), id.clone());
        let (weak, check) = (Rc::downgrade(self), check.downgrade());
        check.upgrade().inspect(|c| c.set_sensitive(false));
        spawn(
            async move { services.change_selection().deselect(&account, &id, copies) },
            move |result| {
                let Some(check) = check.upgrade() else { return };
                check.set_sensitive(true);
                let Some(page) = weak.upgrade() else { return };
                let i18n = &page.ctx.i18n;
                match result {
                    Ok(_) if copies == LocalCopies::Keep => {
                        page.ctx.toast(&i18n.tr("gui-deselect-kept-files"))
                    }
                    Ok(report) => {
                        let mut message = i18n.tr_args(
                            "gui-deselect-removed",
                            [("removed", FluentValue::from(report.removed))],
                        );
                        if !report.kept.is_empty() {
                            let kept = i18n.tr_args(
                                "gui-deselect-kept",
                                [("count", FluentValue::from(report.kept.len()))],
                            );
                            message = format!("{message}. {kept}");
                        }
                        page.ctx.toast(&message);
                    }
                    Err(e) => {
                        revert(&check, &reverting);
                        page.ctx.toast(&i18n.app_error(&e));
                    }
                }
            },
        );
    }
}

/// Puts a checkbox back without treating it as a new change.
fn revert(check: &gtk::CheckButton, reverting: &Cell<bool>) {
    reverting.set(true);
    check.set_active(!check.is_active());
    reverting.set(false);
}
