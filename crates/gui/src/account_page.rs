use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use nextgdrive_application::AppError;
use nextgdrive_application::use_cases::{
    Deletions, SkipReason, SyncOptions, SyncProgress, SyncReport,
};
use nextgdrive_domain::Account;
use nextgdrive_i18n::FluentValue;

use crate::ctx::{Ctx, keep_alive, navigation_page, spawn};
use crate::{folders_page, trash_page};

/// One account: storage, what is synced, and syncing on demand.
struct AccountPage {
    ctx: Rc<Ctx>,
    account: Account,
    quota_row: adw::ActionRow,
    quota_bar: gtk::LevelBar,
    folders_row: adw::ActionRow,
    sync_row: adw::ActionRow,
    sync_button: gtk::Button,
    progress_row: adw::ActionRow,
    progress_bar: gtk::ProgressBar,
    open_row: adw::ActionRow,
    /// Details of the last sync; rebuilt after each one.
    report_group: RefCell<adw::PreferencesGroup>,
    page: adw::PreferencesPage,
}

pub fn build(ctx: &Rc<Ctx>, account: Account) -> adw::NavigationPage {
    let i18n = &ctx.i18n;

    let quota_bar = gtk::LevelBar::builder()
        .width_request(140)
        .valign(gtk::Align::Center)
        .build();
    let quota_row = adw::ActionRow::builder()
        .title(i18n.tr("gui-storage"))
        .subtitle(i18n.tr("gui-quota-loading"))
        .build();
    quota_row.add_suffix(&quota_bar);
    let storage = adw::PreferencesGroup::new();
    storage.add(&quota_row);

    let folders_row = adw::ActionRow::builder()
        .title(i18n.tr("gui-folders"))
        .activatable(true)
        .build();
    folders_row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    let sync_button = gtk::Button::builder()
        .label(i18n.tr("gui-sync-now"))
        .valign(gtk::Align::Center)
        .css_classes(["suggested-action"])
        .build();
    let sync_row = adw::ActionRow::builder()
        .title(i18n.tr("gui-sync-title"))
        .subtitle(i18n.tr("gui-sync-never"))
        .build();
    sync_row.add_suffix(&sync_button);
    let progress_bar = gtk::ProgressBar::builder()
        .width_request(140)
        .valign(gtk::Align::Center)
        .build();
    let progress_row = adw::ActionRow::builder()
        .title(i18n.tr("gui-syncing"))
        .visible(false)
        .build();
    progress_row.add_suffix(&progress_bar);
    let open_row = adw::ActionRow::builder()
        .title(i18n.tr("gui-open-folder"))
        .activatable(true)
        .build();
    open_row.add_suffix(&gtk::Image::from_icon_name("folder-open-symbolic"));
    let sync = adw::PreferencesGroup::builder()
        .title(i18n.tr("gui-sync-group"))
        .build();
    sync.add(&folders_row);
    sync.add(&sync_row);
    sync.add(&progress_row);
    sync.add(&open_row);

    let trash_row = adw::ActionRow::builder()
        .title(i18n.tr("gui-trash"))
        .subtitle(i18n.tr("gui-trash-subtitle"))
        .activatable(true)
        .build();
    trash_row.add_prefix(&gtk::Image::from_icon_name("user-trash-symbolic"));
    trash_row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    let trash = adw::PreferencesGroup::new();
    trash.add(&trash_row);

    let page = adw::PreferencesPage::new();
    page.add(&storage);
    page.add(&sync);
    page.add(&trash);
    let report_group = adw::PreferencesGroup::new();
    page.add(&report_group);

    let root = navigation_page(account.email.as_str(), &adw::HeaderBar::new(), &page);
    let state = Rc::new(AccountPage {
        ctx: ctx.clone(),
        account,
        quota_row,
        quota_bar,
        folders_row,
        sync_row,
        sync_button,
        progress_row,
        progress_bar,
        open_row,
        report_group: RefCell::new(report_group),
        page,
    });

    let weak = Rc::downgrade(&state);
    state.folders_row.connect_activated(move |_| {
        if let Some(state) = weak.upgrade() {
            let folders = folders_page::build(&state.ctx, state.account.clone(), None, false);
            state.ctx.push(&folders);
        }
    });
    let weak = Rc::downgrade(&state);
    state.sync_button.connect_clicked(move |_| {
        if let Some(state) = weak.upgrade() {
            state.sync();
        }
    });
    let weak = Rc::downgrade(&state);
    state.open_row.connect_activated(move |_| {
        if let Some(state) = weak.upgrade() {
            state.open_folder();
        }
    });
    // The row belongs to the page, so the closure may only hold a `Weak`.
    let weak = Rc::downgrade(&state);
    trash_row.connect_activated(move |_| {
        if let Some(state) = weak.upgrade() {
            state
                .ctx
                .push(&trash_page::build(&state.ctx, state.account.clone()));
        }
    });
    // Back from choosing folders: the selection may have changed.
    let weak = Rc::downgrade(&state);
    root.connect_shown(move |_| {
        if let Some(state) = weak.upgrade() {
            state.refresh_selection();
        }
    });

    state.load_quota();
    state.refresh_selection();
    keep_alive(&root, state);
    root
}

impl AccountPage {
    fn account_dir(&self) -> Option<PathBuf> {
        let settings = self.ctx.services.local.get_settings().execute().ok()?;
        Some(settings.account_dir(&self.account))
    }

    fn refresh_selection(&self) {
        let i18n = &self.ctx.i18n;
        let count = match self.ctx.services.local.get_settings().execute() {
            Ok(settings) => settings.selection(&self.account.id).count(),
            Err(e) => {
                self.folders_row.set_subtitle(&i18n.app_error(&e));
                return;
            }
        };
        self.folders_row.set_subtitle(
            &i18n.tr_args("gui-folders-count", [("count", FluentValue::from(count))]),
        );
        self.sync_button.set_sensitive(count > 0);
        self.open_row
            .set_sensitive(self.account_dir().is_some_and(|dir| dir.is_dir()));
    }

    fn load_quota(self: &Rc<Self>) {
        let services = self.ctx.services.clone();
        let account = self.account.clone();
        let weak = Rc::downgrade(self);
        spawn(
            async move { services.account_status().execute(&account).await },
            move |result| {
                let Some(state) = weak.upgrade() else { return };
                let i18n = &state.ctx.i18n;
                match result {
                    Ok(info) => {
                        state.quota_row.set_subtitle(&i18n.quota(&info.quota));
                        if let Some(limit) = info.quota.limit.filter(|l| *l > 0) {
                            #[allow(clippy::cast_precision_loss)] // Display only.
                            state
                                .quota_bar
                                .set_value(info.quota.usage as f64 / limit as f64);
                        } else {
                            state.quota_bar.set_visible(false);
                        }
                    }
                    Err(e) => {
                        state.quota_row.set_subtitle(&i18n.tr("gui-quota-error"));
                        state.quota_row.set_tooltip_text(Some(&i18n.app_error(&e)));
                        state.quota_bar.set_visible(false);
                    }
                }
            },
        );
    }

    fn sync(self: &Rc<Self>) {
        self.sync_with(Deletions::Ask);
    }

    fn sync_with(self: &Rc<Self>, deletions: Deletions) {
        if !self.ctx.begin_sync(&self.account.id) {
            return;
        }
        self.sync_button.set_sensitive(false);
        self.progress_row.set_visible(true);
        self.progress_bar.set_fraction(0.0);
        self.progress_row.set_subtitle("");

        // Progress travels from the sync task to the main thread over a channel.
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<SyncProgress>();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Some(progress) = receiver.recv().await {
                let Some(state) = weak.upgrade() else { return };
                state.show_progress(&progress);
            }
        });

        let services = self.ctx.services.clone();
        let account = self.account.clone();
        let weak = Rc::downgrade(self);
        let ctx = self.ctx.clone();
        let id = self.account.id.clone();
        let options = SyncOptions {
            conflict_label: nextgdrive_composition::conflict_label(&self.ctx.i18n),
            deletions,
        };
        spawn(
            async move {
                let on_progress = move |p: &SyncProgress| {
                    let _ = sender.send(p.clone());
                };
                services
                    .sync_account()
                    .execute(&account, &options, &on_progress)
                    .await
            },
            move |result| {
                ctx.end_sync(&id);
                let Some(state) = weak.upgrade() else { return };
                state.progress_row.set_visible(false);
                state.refresh_selection();
                match result {
                    Ok(report) => state.show_report(&report),
                    Err(AppError::MassDeletion { count }) => state.ask_about_deletions(count),
                    Err(e) => {
                        let message = state.ctx.i18n.app_error(&e);
                        state.sync_row.set_subtitle(&message);
                        state.ctx.toast(&message);
                    }
                }
            },
        );
    }

    /// Many files were deleted here at once: trash them in Drive, or bring them back?
    fn ask_about_deletions(self: &Rc<Self>, count: usize) {
        let i18n = &self.ctx.i18n;
        let dialog = adw::AlertDialog::new(
            Some(&i18n.tr_args("gui-deletions-title", [("count", FluentValue::from(count))])),
            Some(&i18n.tr("gui-deletions-body")),
        );
        dialog.add_responses(&[
            ("cancel", &i18n.tr("gui-cancel")),
            ("restore", &i18n.tr("gui-deletions-restore")),
            ("trash", &i18n.tr("gui-deletions-trash")),
        ]);
        dialog.set_response_appearance("trash", adw::ResponseAppearance::Destructive);
        dialog.set_response_appearance("restore", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("restore"));
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            let Some(state) = weak.upgrade() else { return };
            match response {
                "trash" => state.sync_with(Deletions::Trash),
                "restore" => state.sync_with(Deletions::Restore),
                _ => state.sync_row.set_subtitle(
                    &state
                        .ctx
                        .i18n
                        .tr_args("error-mass-deletion", [("count", FluentValue::from(count))]),
                ),
            }
        });
        dialog.present(self.ctx.window().as_ref());
    }

    fn show_progress(&self, progress: &SyncProgress) {
        let i18n = &self.ctx.i18n;
        if progress.bytes_total > 0 {
            #[allow(clippy::cast_precision_loss)] // Display only.
            self.progress_bar
                .set_fraction(progress.bytes_done as f64 / progress.bytes_total as f64);
        } else if progress.files_total > 0 {
            #[allow(clippy::cast_precision_loss)]
            self.progress_bar
                .set_fraction(progress.files_done as f64 / progress.files_total as f64);
        }
        self.progress_row.set_subtitle(&i18n.tr_args(
            "sync-progress",
            [
                ("done", FluentValue::from(progress.files_done)),
                ("total", FluentValue::from(progress.files_total)),
                ("bytes_done", i18n.bytes(progress.bytes_done).into()),
                ("bytes_total", i18n.bytes(progress.bytes_total).into()),
            ],
        ));
    }

    fn show_report(&self, report: &SyncReport) {
        let i18n = &self.ctx.i18n;
        self.sync_row.set_subtitle(&i18n.sync_summary(report));

        let group = adw::PreferencesGroup::new();
        let base = self.account_dir();
        let relative = |path: &Path| match &base {
            Some(base) => path
                .strip_prefix(base)
                .unwrap_or(path)
                .display()
                .to_string(),
            None => path.display().to_string(),
        };
        let list = |id: &str, icon: &str, rows: Vec<(String, Option<String>)>| {
            let expander = adw::ExpanderRow::builder()
                .title(i18n.tr_args(id, [("count", FluentValue::from(rows.len()))]))
                .build();
            expander.add_prefix(&gtk::Image::from_icon_name(icon));
            for (title, subtitle) in rows {
                let row = adw::ActionRow::builder()
                    .title(title)
                    .use_markup(false)
                    .build();
                if let Some(subtitle) = subtitle {
                    row.set_subtitle(&subtitle);
                }
                expander.add_row(&row);
            }
            expander
        };
        if !report.conflict_copies.is_empty() {
            let rows = report
                .conflict_copies
                .iter()
                .map(|p| (relative(p), None))
                .collect();
            group.add(&list("gui-report-copies", "edit-copy-symbolic", rows));
        }
        if !report.conflicts.is_empty() {
            let rows = report
                .conflicts
                .iter()
                .map(|c| (relative(&c.path), Some(i18n.conflict_reason(c.reason))))
                .collect();
            group.add(&list(
                "gui-report-conflicts",
                "dialog-warning-symbolic",
                rows,
            ));
        }
        if !report.failures.is_empty() {
            let rows = report
                .failures
                .iter()
                .map(|(p, e)| (relative(p), Some(i18n.app_error(e))))
                .collect();
            group.add(&list("gui-report-failures", "dialog-error-symbolic", rows));
        }
        if !report.skipped.is_empty() {
            let rows = report
                .skipped
                .iter()
                .map(|(p, reason)| {
                    let why = match reason {
                        SkipReason::Shortcut => i18n.tr("gui-skipped-shortcut"),
                        SkipReason::NotDownloadable => i18n.tr("gui-skipped-locked"),
                    };
                    (p.display().to_string(), Some(why))
                })
                .collect();
            group.add(&list(
                "gui-report-skipped",
                "action-unavailable-symbolic",
                rows,
            ));
        }
        self.page.remove(&self.report_group.replace(group.clone()));
        self.page.add(&group);
    }

    fn open_folder(&self) {
        let Some(dir) = self.account_dir() else {
            return;
        };
        gtk::FileLauncher::new(Some(&gio::File::for_path(dir))).launch(
            self.ctx.window().as_ref(),
            gio::Cancellable::NONE,
            |_| {},
        );
    }
}
