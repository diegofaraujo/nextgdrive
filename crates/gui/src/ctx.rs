//! What every page needs, and helpers to run work off the GTK main thread.
//!
//! Ownership rule for pages: a page's state lives in an `Rc` kept alive by
//! its root widget ([`keep_alive`]). The state may hold the root's children
//! but never the root itself, and closures stored in widgets the state owns
//! must only hold a `Weak` to it; otherwise the page would never be freed.

use std::cell::RefCell;
use std::collections::HashSet;
use std::future::Future;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use adw::prelude::*;
use gtk::glib;
use nextgdrive_composition::Services;
use nextgdrive_domain::AccountId;
use nextgdrive_i18n::Localizer;

pub struct Ctx {
    pub i18n: Localizer,
    pub services: Arc<Services>,
    /// Weak: the window owns these widgets, and they own the pages holding `Ctx`.
    nav: glib::WeakRef<adw::NavigationView>,
    toasts: glib::WeakRef<adw::ToastOverlay>,
    /// Accounts being synced; two syncs of one account must not overlap.
    syncing: RefCell<HashSet<AccountId>>,
}

impl Ctx {
    pub fn new(
        i18n: Localizer,
        services: Arc<Services>,
        nav: &adw::NavigationView,
        toasts: &adw::ToastOverlay,
    ) -> Rc<Self> {
        Rc::new(Self {
            i18n,
            services,
            nav: nav.downgrade(),
            toasts: toasts.downgrade(),
            syncing: RefCell::default(),
        })
    }

    /// Marks the account as syncing. Returns `false` if it already was.
    pub fn begin_sync(&self, account: &AccountId) -> bool {
        self.syncing.borrow_mut().insert(account.clone())
    }

    pub fn end_sync(&self, account: &AccountId) {
        self.syncing.borrow_mut().remove(account);
    }

    pub fn push(&self, page: &adw::NavigationPage) {
        if let Some(nav) = self.nav.upgrade() {
            nav.push(page);
        }
    }

    pub fn toast(&self, message: &str) {
        if let Some(toasts) = self.toasts.upgrade() {
            toasts.add_toast(adw::Toast::new(message));
        }
    }

    pub fn window(&self) -> Option<gtk::Window> {
        self.nav.upgrade()?.root().and_downcast()
    }
}

/// Runtime for network and keyring work, kept off the GTK main thread.
pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("failed to start the async runtime")
    })
}

/// Runs `task` on the background runtime, then `then` with its output on the
/// GTK main thread.
pub fn spawn<T: Send + 'static>(
    task: impl Future<Output = T> + Send + 'static,
    then: impl FnOnce(T) + 'static,
) -> tokio::task::AbortHandle {
    let handle = runtime().spawn(task);
    let abort = handle.abort_handle();
    glib::spawn_future_local(async move {
        if let Ok(output) = handle.await {
            then(output);
        }
    });
    abort
}

/// Makes the background process start a round now, to pick up a change in
/// the settings (offline items, cache limit).
pub async fn wake_host() {
    if let Some(client) = nextgdrive_adapter_dbus::Client::connect().await {
        let _ = client.sync_now().await;
    }
}

/// Keeps `value` alive as long as `widget` exists.
pub fn keep_alive<T: 'static>(widget: &impl IsA<gtk::Widget>, value: Rc<T>) {
    widget.connect_destroy(move |_| {
        let _ = &value;
    });
}

/// A page with its own header bar, as libadwaita navigation expects.
pub fn navigation_page(
    title: &str,
    header: &adw::HeaderBar,
    content: &impl IsA<gtk::Widget>,
) -> adw::NavigationPage {
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(header);
    toolbar.set_content(Some(content));
    adw::NavigationPage::builder()
        .title(title)
        .child(&toolbar)
        .build()
}
