use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use nextgdrive_application::AppError;
use nextgdrive_domain::{Account, DriveItemId, DriveKind};

use crate::ctx::{Ctx, keep_alive, navigation_page, spawn};

/// A folder the user can select and open.
struct Entry {
    id: DriveItemId,
    name: String,
    selected: bool,
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
                    self.group.add(&self.row(entry));
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
        let id = entry.id.clone();
        check.connect_toggled(move |check| {
            if reverting.get() {
                return;
            }
            if let Some(page) = weak.upgrade() {
                page.change(check, &id, reverting.clone());
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

    fn change(
        self: &Rc<Self>,
        check: &gtk::CheckButton,
        id: &DriveItemId,
        reverting: Rc<Cell<bool>>,
    ) {
        let selected = check.is_active();
        let services = self.ctx.services.clone();
        let (account, id) = (self.account.clone(), id.clone());
        let (weak, check) = (Rc::downgrade(self), check.downgrade());
        check.upgrade().inspect(|c| c.set_sensitive(false));
        spawn(
            async move {
                let change = services.change_selection();
                if selected {
                    change.select(&account, &id).await
                } else {
                    change.deselect(&account, &id)
                }
            },
            move |result| {
                let Some(check) = check.upgrade() else { return };
                check.set_sensitive(true);
                if let Err(e) = result {
                    reverting.set(true);
                    check.set_active(!selected);
                    reverting.set(false);
                    if let Some(page) = weak.upgrade() {
                        page.ctx.toast(&page.ctx.i18n.app_error(&e));
                    }
                }
            },
        );
    }
}
