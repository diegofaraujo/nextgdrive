use std::cell::RefCell;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::gio;
use nextgdrive_application::ports::PendingAuthorization;
use nextgdrive_composition::PendingSignIn;
use nextgdrive_domain::Account;

use crate::ctx::{Ctx, keep_alive, navigation_page, spawn};
use crate::{account_page, preferences};

/// Lists linked accounts with their storage usage, and lets the user link and
/// unlink accounts.
struct AccountsPage {
    ctx: Rc<Ctx>,
    /// Shows either `list` or `status`.
    stack: gtk::Stack,
    list: adw::PreferencesPage,
    /// Rebuilt on every refresh.
    group: RefCell<adw::PreferencesGroup>,
    status: adw::StatusPage,
}

pub fn build(ctx: &Rc<Ctx>) -> adw::NavigationPage {
    let i18n = &ctx.i18n;
    let list = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    list.add(&group);
    let status = adw::StatusPage::new();
    let stack = gtk::Stack::new();
    stack.add_child(&list);
    stack.add_child(&status);

    let header = adw::HeaderBar::new();
    let add = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text(i18n.tr("gui-add-account"))
        .build();
    header.pack_start(&add);
    let menu = gio::Menu::new();
    menu.append(
        Some(&i18n.tr("gui-preferences")),
        Some("accounts.preferences"),
    );
    menu.append(Some(&i18n.tr("gui-about")), Some("accounts.about"));
    let quit = gio::Menu::new();
    quit.append(Some(&i18n.tr("gui-quit")), Some("app.quit"));
    menu.append_section(None, &quit);
    header.pack_end(
        &gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .tooltip_text(i18n.tr("gui-main-menu"))
            .menu_model(&menu)
            .build(),
    );

    let root = navigation_page(&i18n.tr("app-name"), &header, &stack);
    let page = Rc::new(AccountsPage {
        ctx: ctx.clone(),
        stack,
        list,
        group: RefCell::new(group),
        status,
    });

    let actions = gio::SimpleActionGroup::new();
    let preferences = gio::SimpleAction::new("preferences", None);
    let weak = Rc::downgrade(&page);
    preferences.connect_activate(move |_, _| {
        if let Some(page) = weak.upgrade() {
            preferences::present(&page.ctx);
        }
    });
    actions.add_action(&preferences);
    let about = gio::SimpleAction::new("about", None);
    let weak = Rc::downgrade(&page);
    about.connect_activate(move |_, _| {
        if let Some(page) = weak.upgrade() {
            present_about(&page.ctx);
        }
    });
    actions.add_action(&about);
    root.insert_action_group("accounts", Some(&actions));

    let weak = Rc::downgrade(&page);
    add.connect_clicked(move |button| {
        if let Some(page) = weak.upgrade() {
            page.add_account(button);
        }
    });
    // Refresh when coming back from an account's page.
    let weak = Rc::downgrade(&page);
    root.connect_shown(move |_| {
        if let Some(page) = weak.upgrade() {
            page.refresh();
        }
    });
    page.refresh();
    keep_alive(&root, page);
    root
}

impl AccountsPage {
    fn refresh(self: &Rc<Self>) {
        let i18n = &self.ctx.i18n;
        match self.ctx.services.local.list_accounts().execute() {
            Err(e) => self.show_status(
                "dialog-error-symbolic",
                &i18n.tr("gui-load-error-title"),
                &i18n.app_error(&e),
            ),
            Ok(list) if list.is_empty() => self.show_status(
                crate::APP_ID,
                &i18n.tr("gui-no-accounts-title"),
                &i18n.tr("gui-no-accounts-description"),
            ),
            Ok(list) => {
                let group = adw::PreferencesGroup::builder()
                    .title(i18n.tr("gui-accounts-title"))
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
        let i18n = &self.ctx.i18n;
        let row = adw::ActionRow::builder()
            .title(account.email.as_str())
            .subtitle(i18n.tr("gui-quota-loading"))
            // Emails and names are user data, not Pango markup.
            .use_markup(false)
            .activatable(true)
            .build();
        let remove = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text(i18n.tr("gui-remove-account"))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let weak = Rc::downgrade(self);
        let removed = account.clone();
        remove.connect_clicked(move |button| {
            if let Some(page) = weak.upgrade() {
                page.confirm_remove(button, &removed);
            }
        });
        row.add_suffix(&remove);
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));

        let weak = Rc::downgrade(self);
        let opened = account.clone();
        row.connect_activated(move |_| {
            if let Some(page) = weak.upgrade() {
                page.ctx
                    .push(&account_page::build(&page.ctx, opened.clone()));
            }
        });
        self.load_quota(&row, account);
        row
    }

    fn load_quota(self: &Rc<Self>, row: &adw::ActionRow, account: &Account) {
        let services = self.ctx.services.clone();
        let account = account.clone();
        let (row, weak) = (row.downgrade(), Rc::downgrade(self));
        spawn(
            async move { services.account_status().execute(&account).await },
            move |result| {
                let (Some(row), Some(page)) = (row.upgrade(), weak.upgrade()) else {
                    return;
                };
                let i18n = &page.ctx.i18n;
                match result {
                    Ok(info) => {
                        let quota = i18n.quota(&info.quota);
                        row.set_subtitle(&match info.account.display_name {
                            Some(name) => format!("{name} · {quota}"),
                            None => quota,
                        });
                    }
                    Err(e) => {
                        row.set_subtitle(&i18n.tr("gui-quota-error"));
                        row.set_tooltip_text(Some(&i18n.app_error(&e)));
                    }
                }
            },
        );
    }

    /// Starts sign-in, opens the browser, and waits in a dialog that lets the user cancel.
    fn add_account(self: &Rc<Self>, button: &gtk::Button) {
        button.set_sensitive(false);
        let services = self.ctx.services.clone();
        let (button, weak) = (button.clone(), Rc::downgrade(self));
        spawn(
            async move { services.add_account().start().await },
            move |result| {
                let Some(page) = weak.upgrade() else { return };
                match result {
                    Ok(pending) => page.wait_for_sign_in(&button, pending),
                    Err(e) => {
                        button.set_sensitive(true);
                        page.ctx.toast(&page.ctx.i18n.app_error(&e));
                    }
                }
            },
        );
    }

    fn wait_for_sign_in(self: &Rc<Self>, button: &gtk::Button, pending: PendingSignIn) {
        let i18n = &self.ctx.i18n;
        let url = pending.url().to_owned();
        gtk::UriLauncher::new(&url).launch(
            self.ctx.window().as_ref(),
            gio::Cancellable::NONE,
            |_| {},
        );

        let dialog = adw::AlertDialog::new(
            Some(&i18n.tr("gui-add-waiting-title")),
            Some(&i18n.tr("gui-add-waiting-body")),
        );
        dialog.set_extra_child(Some(&gtk::LinkButton::with_label(
            &url,
            &i18n.tr("gui-open-browser"),
        )));
        dialog.add_response("cancel", &i18n.tr("gui-cancel"));
        dialog.set_close_response("cancel");

        let services = self.ctx.services.clone();
        let (finished, weak, add) = (dialog.downgrade(), Rc::downgrade(self), button.clone());
        // Aborting drops the pending authorization, which closes the loopback server.
        let abort = spawn(
            async move { services.add_account().finish(pending).await },
            move |result| {
                if let Some(dialog) = finished.upgrade() {
                    dialog.force_close();
                }
                add.set_sensitive(true);
                let Some(page) = weak.upgrade() else { return };
                let i18n = &page.ctx.i18n;
                match result {
                    Ok(info) => {
                        let email = info.account.email.to_string();
                        page.ctx
                            .toast(&i18n.tr_args("account-added", [("email", email.into())]));
                        page.refresh();
                    }
                    Err(e) => page.ctx.toast(&i18n.app_error(&e)),
                }
            },
        );
        let add = button.downgrade();
        dialog.connect_response(Some("cancel"), move |_, _| {
            abort.abort();
            // An aborted task's continuation never runs, so re-enable here.
            if let Some(add) = add.upgrade() {
                add.set_sensitive(true);
            }
        });
        dialog.present(self.ctx.window().as_ref());
    }

    fn confirm_remove(self: &Rc<Self>, parent: &gtk::Button, account: &Account) {
        let i18n = &self.ctx.i18n;
        let email = account.email.to_string();
        let dialog = adw::AlertDialog::new(
            Some(&i18n.tr_args("gui-remove-confirm-title", [("email", email.into())])),
            Some(&i18n.tr("gui-remove-confirm-body")),
        );
        dialog.add_responses(&[
            ("cancel", &i18n.tr("gui-cancel")),
            ("remove", &i18n.tr("gui-remove")),
        ]);
        dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        let weak: Weak<Self> = Rc::downgrade(self);
        let email = account.email.clone();
        dialog.connect_response(Some("remove"), move |_, _| {
            let Some(page) = weak.upgrade() else { return };
            let services = page.ctx.services.clone();
            let email = email.clone();
            let weak = Rc::downgrade(&page);
            spawn(
                async move { services.remove_account().execute(&email).await },
                move |result| {
                    let Some(page) = weak.upgrade() else { return };
                    let i18n = &page.ctx.i18n;
                    let message = match result {
                        Ok(removed) => i18n.tr_args(
                            "account-removed",
                            [("email", removed.email.to_string().into())],
                        ),
                        Err(e) => i18n.app_error(&e),
                    };
                    page.ctx.toast(&message);
                    page.refresh();
                },
            );
        });
        dialog.present(Some(parent));
    }
}

fn present_about(ctx: &Rc<Ctx>) {
    let i18n = &ctx.i18n;
    let repository = env!("CARGO_PKG_REPOSITORY");
    let about = adw::AboutDialog::builder()
        .application_name(i18n.tr("app-name"))
        .application_icon(crate::APP_ID)
        .version(env!("CARGO_PKG_VERSION"))
        .comments(i18n.tr("gui-about-comments"))
        .website(repository)
        .issue_url(format!("{repository}/issues"))
        .support_url(format!("{repository}/discussions"))
        .license_type(gtk::License::Gpl30)
        // "Name URL" shows the name as a link.
        .developers([format!(
            "{} {repository}/graphs/contributors",
            i18n.tr("gui-about-contributors")
        )])
        .build();
    about.present(ctx.window().as_ref());
}
