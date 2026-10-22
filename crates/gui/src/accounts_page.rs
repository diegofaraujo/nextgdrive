use std::cell::RefCell;
use std::future::Future;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use adw::prelude::*;
use gtk::glib;
use nextgdrive_application::AppError;
use nextgdrive_application::ports::PendingAuthorization;
use nextgdrive_composition::{PendingSignIn, Services};
use nextgdrive_domain::Account;
use nextgdrive_i18n::Localizer;

use crate::runtime;

/// Lists linked accounts with their storage usage, and lets the user link and
/// unlink accounts.
struct AccountsPage {
    i18n: Localizer,
    services: Arc<Services>,
    /// Shows either `list` or `status`.
    stack: gtk::Stack,
    list: adw::PreferencesPage,
    /// Rebuilt on every refresh.
    group: RefCell<adw::PreferencesGroup>,
    status: adw::StatusPage,
    toasts: adw::ToastOverlay,
}

pub fn build(i18n: Localizer, services: Arc<Services>, header: &adw::HeaderBar) -> gtk::Widget {
    let list = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    list.add(&group);
    let status = adw::StatusPage::new();
    let stack = gtk::Stack::new();
    stack.add_child(&list);
    stack.add_child(&status);
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&stack));

    let add = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text(i18n.tr("gui-add-account"))
        .build();
    header.pack_start(&add);

    let page = Rc::new(AccountsPage {
        i18n,
        services,
        stack,
        list,
        group: RefCell::new(group),
        status,
        toasts,
    });
    page.refresh();
    let widget = page.toasts.clone().upcast();
    // This closure owns the page: the header bar lives as long as the window,
    // and the page holds no reference back to it, so there is no cycle.
    // Everything the page itself owns must only hold a `Weak` to it.
    add.connect_clicked(move |button| page.add_account(button));
    widget
}

/// Runs `task` on the background runtime and hands its output to `then` on
/// the GTK main thread. `then` is skipped if the page is gone by then.
fn spawn<T, F>(page: &Rc<AccountsPage>, task: F, then: impl FnOnce(&Rc<AccountsPage>, T) + 'static)
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
{
    let handle = runtime().spawn(task);
    let page = Rc::downgrade(page);
    glib::spawn_future_local(async move {
        if let (Ok(output), Some(page)) = (handle.await, page.upgrade()) {
            then(&page, output);
        }
    });
}

impl AccountsPage {
    fn refresh(self: &Rc<Self>) {
        match self.services.list_accounts().execute() {
            Err(e) => self.show_status(
                "dialog-error-symbolic",
                &self.i18n.tr("gui-load-error-title"),
                &self.i18n.app_error(&e),
            ),
            Ok(list) if list.is_empty() => self.show_status(
                "folder-remote-symbolic",
                &self.i18n.tr("gui-no-accounts-title"),
                &self.i18n.tr("gui-no-accounts-description"),
            ),
            Ok(list) => {
                let group = adw::PreferencesGroup::builder()
                    .title(self.i18n.tr("gui-accounts-title"))
                    .build();
                for account in &list {
                    group.add(&self.account_row(account));
                }
                self.list.remove(&self.group.replace(group.clone()));
                self.list.add(&group);
                self.stack.set_visible_child(&self.list);
            }
        }
    }

    fn show_status(&self, icon: &str, title: &str, description: &str) {
        self.status.set_icon_name(Some(icon));
        self.status.set_title(title);
        self.status.set_description(Some(description));
        self.stack.set_visible_child(&self.status);
    }

    fn account_row(self: &Rc<Self>, account: &Account) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(account.email.as_str())
            .subtitle(self.i18n.tr("gui-quota-loading"))
            // Emails and names are user data, not Pango markup.
            .use_markup(false)
            .build();
        let remove = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text(self.i18n.tr("gui-remove-account"))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let page = Rc::downgrade(self);
        let removed = account.clone();
        remove.connect_clicked(move |button| {
            if let Some(page) = page.upgrade() {
                page.confirm_remove(button, &removed);
            }
        });
        row.add_suffix(&remove);
        self.load_quota(&row, account);
        row
    }

    fn load_quota(self: &Rc<Self>, row: &adw::ActionRow, account: &Account) {
        let services = self.services.clone();
        let account = account.clone();
        let row = row.downgrade();
        spawn(
            self,
            async move { services.account_status().execute(&account).await },
            move |page, result| {
                let Some(row) = row.upgrade() else { return };
                match result {
                    Ok(info) => {
                        let quota = page.i18n.quota(&info.quota);
                        row.set_subtitle(&match info.account.display_name {
                            Some(name) => format!("{name} · {quota}"),
                            None => quota,
                        });
                    }
                    Err(e) => {
                        row.set_subtitle(&page.i18n.tr("gui-quota-error"));
                        row.set_tooltip_text(Some(&page.i18n.app_error(&e)));
                    }
                }
            },
        );
    }

    /// Starts sign-in, opens the browser, and waits in a dialog that lets the user cancel.
    fn add_account(self: &Rc<Self>, button: &gtk::Button) {
        button.set_sensitive(false);
        let services = self.services.clone();
        let button = button.clone();
        spawn(
            self,
            async move { services.add_account().start().await },
            move |page, result| match result {
                Ok(pending) => page.wait_for_sign_in(&button, pending),
                Err(e) => {
                    button.set_sensitive(true);
                    page.toast(&page.i18n.app_error(&e));
                }
            },
        );
    }

    fn wait_for_sign_in(self: &Rc<Self>, button: &gtk::Button, pending: PendingSignIn) {
        let url = pending.url().to_owned();
        let window = button.root().and_downcast::<gtk::Window>();
        gtk::UriLauncher::new(&url).launch(window.as_ref(), gtk::gio::Cancellable::NONE, |_| {});

        let dialog = adw::AlertDialog::new(
            Some(&self.i18n.tr("gui-add-waiting-title")),
            Some(&self.i18n.tr("gui-add-waiting-body")),
        );
        dialog.set_extra_child(Some(&gtk::LinkButton::with_label(
            &url,
            &self.i18n.tr("gui-open-browser"),
        )));
        dialog.add_response("cancel", &self.i18n.tr("gui-cancel"));
        dialog.set_close_response("cancel");

        let services = self.services.clone();
        let handle = runtime().spawn(async move { services.add_account().finish(pending).await });
        // Dropping the pending authorization closes the loopback server.
        let abort = handle.abort_handle();
        dialog.connect_response(Some("cancel"), move |_, _| abort.abort());
        dialog.present(Some(button));

        let page: Weak<Self> = Rc::downgrade(self);
        let button = button.clone();
        glib::spawn_future_local(async move {
            let result = handle.await;
            dialog.force_close();
            button.set_sensitive(true);
            let Some(page) = page.upgrade() else { return };
            match result {
                Ok(Ok(info)) => {
                    let email = info.account.email.to_string();
                    page.toast(
                        &page
                            .i18n
                            .tr_args("account-added", [("email", email.into())]),
                    );
                    page.refresh();
                }
                Ok(Err(e)) => page.toast(&page.i18n.app_error(&e)),
                // Cancelled by the user.
                Err(_) => {}
            }
        });
    }

    fn confirm_remove(self: &Rc<Self>, parent: &gtk::Button, account: &Account) {
        let email = account.email.to_string();
        let dialog = adw::AlertDialog::new(
            Some(
                &self
                    .i18n
                    .tr_args("gui-remove-confirm-title", [("email", email.into())]),
            ),
            Some(&self.i18n.tr("gui-remove-confirm-body")),
        );
        dialog.add_responses(&[
            ("cancel", &self.i18n.tr("gui-cancel")),
            ("remove", &self.i18n.tr("gui-remove")),
        ]);
        dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        let page: Weak<Self> = Rc::downgrade(self);
        let email = account.email.clone();
        dialog.connect_response(Some("remove"), move |_, _| {
            let Some(page) = page.upgrade() else { return };
            let services = page.services.clone();
            let email = email.clone();
            spawn(
                &page,
                async move { services.remove_account().execute(&email).await },
                |page, result: Result<Account, AppError>| {
                    let message = match result {
                        Ok(removed) => page.i18n.tr_args(
                            "account-removed",
                            [("email", removed.email.to_string().into())],
                        ),
                        Err(e) => page.i18n.app_error(&e),
                    };
                    page.toast(&message);
                    page.refresh();
                },
            );
        });
        dialog.present(Some(parent));
    }

    fn toast(&self, message: &str) {
        self.toasts.add_toast(adw::Toast::new(message));
    }
}
