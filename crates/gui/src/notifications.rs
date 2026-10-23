//! Desktop notifications (freedesktop.org, through GNotification).

use adw::prelude::*;
use gtk::gio;
use nextgdrive_composition::host::{Notice, NoticeKind};
use nextgdrive_i18n::Localizer;

pub fn send(app: &adw::Application, notice: &Notice) {
    let notification = gio::Notification::new(&notice.title);
    notification.set_body(Some(&notice.body));
    notification.set_icon(&gio::ThemedIcon::new(match notice.kind {
        NoticeKind::ConflictCopies => "edit-copy-symbolic",
        NoticeKind::NeedsAttention | NoticeKind::ConfirmDeletions => "dialog-warning-symbolic",
        NoticeKind::Problem => "dialog-error-symbolic",
    }));
    notification.set_default_action("app.show-window");
    if notice.kind == NoticeKind::ConfirmDeletions {
        notification.add_button(
            &Localizer::from_system().tr("notify-open"),
            "app.show-window",
        );
        notification.set_priority(gio::NotificationPriority::High);
    }
    // One notification per kind and account: a newer one replaces it.
    let id = format!("{:?}-{}", notice.kind, notice.account);
    app.send_notification(Some(&id), &notification);
}
