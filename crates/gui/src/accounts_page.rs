use std::cell::RefCell;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use nextgdrive_adapter_config::TomlAccountRepository;
use nextgdrive_application::use_cases::{ListAccounts, RemoveAccount};
use nextgdrive_domain::Account;
use nextgdrive_i18n::Localizer;

/// Lists linked accounts and lets the user unlink them.
struct AccountsPage {
    i18n: Localizer,
    accounts: TomlAccountRepository,
    /// Shows either `list` or `status`.
    stack: gtk::Stack,
    list: adw::PreferencesPage,
    /// Rebuilt on every refresh.
    group: RefCell<adw::PreferencesGroup>,
    status: adw::StatusPage,
    toasts: adw::ToastOverlay,
}

pub fn build(i18n: Localizer, accounts: TomlAccountRepository) -> gtk::Widget {
    let list = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    list.add(&group);
    let status = adw::StatusPage::new();
    let stack = gtk::Stack::new();
    stack.add_child(&list);
    stack.add_child(&status);
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&stack));

    let page = Rc::new(AccountsPage {
        i18n,
        accounts,
        stack,
        list,
        group: RefCell::new(group),
        status,
        toasts,
    });
    page.refresh();
    page.toasts.clone().upcast()
}

impl AccountsPage {
    fn refresh(self: &Rc<Self>) {
        match ListAccounts::new(&self.accounts).execute() {
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
            .subtitle(account.display_name.as_deref().unwrap_or_default())
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
        let account = account.clone();
        remove.connect_clicked(move |button| {
            if let Some(page) = page.upgrade() {
                page.confirm_remove(button, &account);
            }
        });
        row.add_suffix(&remove);
        row
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
            let message = match RemoveAccount::new(&page.accounts).execute(&email) {
                Ok(removed) => page.i18n.tr_args(
                    "account-removed",
                    [("email", removed.email.to_string().into())],
                ),
                Err(e) => page.i18n.app_error(&e),
            };
            page.toasts.add_toast(adw::Toast::new(&message));
            page.refresh();
        });
        dialog.present(Some(parent));
    }
}
