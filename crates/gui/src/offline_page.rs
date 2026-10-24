use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};
use nextgdrive_application::use_cases::OfflineItem;
use nextgdrive_domain::{Account, DriveItemId, ItemKind};

use crate::ctx::{Ctx, keep_alive, navigation_page, spawn};

/// How often the list refreshes while something downloads (each refresh
/// walks the offline folders in Drive, so not when nothing changes).
const REFRESH: Duration = Duration::from_secs(3);

/// Files and folders of a streamed account kept on this computer.
struct OfflinePage {
    ctx: Rc<Ctx>,
    account: Account,
    /// The account folder, where the file chooser starts.
    dir: PathBuf,
    page: adw::PreferencesPage,
    /// Rebuilt on every refresh.
    group: RefCell<adw::PreferencesGroup>,
    downloading: Cell<bool>,
}

pub fn build(ctx: &Rc<Ctx>, account: Account, dir: PathBuf) -> adw::NavigationPage {
    let i18n = &ctx.i18n;
    let header = adw::HeaderBar::new();
    let add_file = gtk::Button::builder()
        .icon_name("document-new-symbolic")
        .tooltip_text(i18n.tr("gui-offline-add-file"))
        .build();
    let add_folder = gtk::Button::builder()
        .icon_name("folder-new-symbolic")
        .tooltip_text(i18n.tr("gui-offline-add-folder"))
        .build();
    header.pack_start(&add_file);
    header.pack_start(&add_folder);

    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    page.add(&group);
    let root = navigation_page(&i18n.tr("gui-offline"), &header, &page);
    let state = Rc::new(OfflinePage {
        ctx: ctx.clone(),
        account,
        dir,
        page,
        group: RefCell::new(group),
        downloading: Cell::new(false),
    });

    let weak = Rc::downgrade(&state);
    add_file.connect_clicked(move |_| {
        if let Some(state) = weak.upgrade() {
            state.choose(false);
        }
    });
    let weak = Rc::downgrade(&state);
    add_folder.connect_clicked(move |_| {
        if let Some(state) = weak.upgrade() {
            state.choose(true);
        }
    });
    // Refreshes while the page exists; stops once it is gone.
    let weak = Rc::downgrade(&state);
    glib::timeout_add_local(REFRESH, move || match weak.upgrade() {
        Some(state) => {
            if state.downloading.get() {
                state.load();
            }
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });
    state.load();
    keep_alive(&root, state);
    root
}

impl OfflinePage {
    fn load(self: &Rc<Self>) {
        let services = self.ctx.services.clone();
        let (account, i18n) = (self.account.clone(), self.ctx.i18n.clone());
        let weak = Rc::downgrade(self);
        spawn(
            async move {
                let limit = services
                    .local
                    .get_settings()
                    .execute()
                    .map_err(|e| i18n.app_error(&e))?
                    .cache_limit;
                let cache = nextgdrive_composition::stream::open_cache(&i18n, limit)?;
                services
                    .offline(&cache)
                    .status(&account)
                    .await
                    .map_err(|e| i18n.app_error(&e))
            },
            move |result: Result<Vec<OfflineItem>, String>| {
                if let Some(state) = weak.upgrade() {
                    state.show(result);
                }
            },
        );
    }

    fn show(self: &Rc<Self>, result: Result<Vec<OfflineItem>, String>) {
        let i18n = &self.ctx.i18n;
        let group = adw::PreferencesGroup::builder()
            .description(i18n.tr("gui-offline-description"))
            .build();
        self.downloading.set(false);
        match result {
            Err(message) => group.add(
                &adw::ActionRow::builder()
                    .title(message)
                    .use_markup(false)
                    .build(),
            ),
            Ok(items) if items.is_empty() => {
                group.add(
                    &adw::ActionRow::builder()
                        .title(i18n.tr("gui-offline-empty"))
                        .build(),
                );
            }
            Ok(items) => {
                self.downloading.set(items.iter().any(|i| !i.complete()));
                for item in items {
                    group.add(&self.row(&item));
                }
            }
        }
        self.page.remove(&self.group.replace(group.clone()));
        self.page.add(&group);
    }

    fn row(self: &Rc<Self>, item: &OfflineItem) -> adw::ActionRow {
        let i18n = &self.ctx.i18n;
        let progress = i18n.tr_args(
            if item.complete() {
                "offline-complete"
            } else {
                "offline-progress"
            },
            [
                ("cached", i18n.bytes(item.bytes_cached).into()),
                ("total", i18n.bytes(item.bytes_total).into()),
            ],
        );
        let row = adw::ActionRow::builder()
            .title(&item.item.name)
            .subtitle(progress)
            .use_markup(false)
            .build();
        let icon = match item.item.kind {
            ItemKind::Folder => "folder-symbolic",
            _ => "text-x-generic-symbolic",
        };
        row.add_prefix(&gtk::Image::from_icon_name(icon));
        if !item.complete() {
            let spinner = gtk::Spinner::builder()
                .spinning(true)
                .valign(gtk::Align::Center)
                .build();
            row.add_suffix(&spinner);
        }
        let remove = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text(i18n.tr("gui-offline-remove"))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let (weak, id) = (Rc::downgrade(self), item.item.id.clone());
        remove.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.remove(&id);
            }
        });
        row.add_suffix(&remove);
        row
    }

    /// Lets the user pick a file or folder inside the account folder.
    fn choose(self: &Rc<Self>, folder: bool) {
        let chooser = gtk::FileDialog::builder()
            .title(self.ctx.i18n.tr(if folder {
                "gui-offline-add-folder"
            } else {
                "gui-offline-add-file"
            }))
            .initial_folder(&gio::File::for_path(&self.dir))
            .modal(true)
            .build();
        let weak = Rc::downgrade(self);
        let window = self.ctx.window();
        glib::spawn_future_local(async move {
            let picked = if folder {
                chooser.select_folder_future(window.as_ref()).await
            } else {
                chooser.open_future(window.as_ref()).await
            };
            // Cancelling the chooser is not an error worth reporting.
            let (Ok(file), Some(state)) = (picked, weak.upgrade()) else {
                return;
            };
            let Some(path) = file.path() else { return };
            state.add(path);
        });
    }

    fn add(self: &Rc<Self>, path: PathBuf) {
        let Ok(relative) = path.strip_prefix(&self.dir).map(PathBuf::from) else {
            let message = self.ctx.i18n.tr_args(
                "cli-offline-outside",
                [("path", path.display().to_string().into())],
            );
            return self.ctx.toast(&message);
        };
        let services = self.ctx.services.clone();
        let (account, i18n) = (self.account.clone(), self.ctx.i18n.clone());
        let weak = Rc::downgrade(self);
        spawn(
            async move {
                let limit = services
                    .local
                    .get_settings()
                    .execute()
                    .map_err(|e| i18n.app_error(&e))?
                    .cache_limit;
                let cache = nextgdrive_composition::stream::open_cache(&i18n, limit)?;
                let item = services
                    .offline(&cache)
                    .add(&account, &relative)
                    .await
                    .map_err(|e| i18n.app_error(&e))?;
                wake().await;
                Ok::<_, String>(item.name)
            },
            move |result| {
                let Some(state) = weak.upgrade() else { return };
                match result {
                    Ok(name) => {
                        state.downloading.set(true);
                        state.ctx.toast(
                            &state
                                .ctx
                                .i18n
                                .tr_args("cli-offline-added", [("name", name.into())]),
                        );
                    }
                    Err(message) => state.ctx.toast(&message),
                }
                state.load();
            },
        );
    }

    fn remove(self: &Rc<Self>, id: &DriveItemId) {
        let services = self.ctx.services.clone();
        let (account, id, i18n) = (self.account.clone(), id.clone(), self.ctx.i18n.clone());
        let weak = Rc::downgrade(self);
        spawn(
            async move {
                let limit = services
                    .local
                    .get_settings()
                    .execute()
                    .map_err(|e| i18n.app_error(&e))?
                    .cache_limit;
                let cache = nextgdrive_composition::stream::open_cache(&i18n, limit)?;
                services
                    .offline(&cache)
                    .remove(&account, &id)
                    .map_err(|e| i18n.app_error(&e))?;
                wake().await;
                Ok::<_, String>(())
            },
            move |result| {
                let Some(state) = weak.upgrade() else { return };
                if let Err(message) = result {
                    state.ctx.toast(&message);
                }
                state.load();
            },
        );
    }
}

/// The background process downloads (or releases) on its next round.
async fn wake() {
    if let Some(client) = nextgdrive_adapter_dbus::Client::connect().await {
        let _ = client.sync_now().await;
    }
}
