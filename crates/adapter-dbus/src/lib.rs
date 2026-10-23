//! D-Bus service of the process running background sync (the tray app or
//! `nextgdrive daemon`), and the client other processes use to control it.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use zbus::zvariant::Type;

/// Owning this name on the session bus means "I am running background sync".
pub const BUS_NAME: &str = "io.github.diegofaraujo.NextGDrive.Sync";
pub const OBJECT_PATH: &str = "/io/github/diegofaraujo/NextGDrive/Sync";

pub const STATE_IDLE: &str = "idle";
pub const STATE_SYNCING: &str = "syncing";
pub const STATE_PAUSED: &str = "paused";

/// What background sync is doing. Messages are already translated by the
/// process that runs it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct HostStatus {
    /// One of the `STATE_*` constants.
    pub state: String,
    /// Account being synced, when syncing.
    pub account: String,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    /// Unix time of the last completed check; 0 if none yet.
    pub last_check: i64,
    /// (account email, message) for each problem in the last check.
    pub errors: Vec<(String, String)>,
}

/// Implemented by the process running background sync.
pub trait HostControl: Send + Sync + 'static {
    fn status(&self) -> HostStatus;
    fn sync_now(&self);
    fn pause(&self);
    fn resume(&self);
    /// Many files of this account were deleted locally: trash them in Drive.
    fn confirm_deletions(&self, email: &str);
}

struct Service(Arc<dyn HostControl>);

#[zbus::interface(name = "io.github.diegofaraujo.NextGDrive.Sync1")]
impl Service {
    fn status(&self) -> HostStatus {
        self.0.status()
    }

    fn sync_now(&self) {
        self.0.sync_now();
    }

    fn pause(&self) {
        self.0.pause();
    }

    fn resume(&self) {
        self.0.resume();
    }

    fn confirm_deletions(&self, email: &str) {
        self.0.confirm_deletions(email);
    }
}

#[derive(Debug)]
pub enum ServeError {
    /// Another process already runs background sync.
    AlreadyRunning,
    Bus(zbus::Error),
}

/// Publishes `control` on the session bus. Keep the returned connection
/// alive for as long as the service should exist.
pub async fn serve(control: Arc<dyn HostControl>) -> Result<zbus::Connection, ServeError> {
    let connection = zbus::connection::Builder::session()
        .and_then(|b| b.serve_at(OBJECT_PATH, Service(control)))
        .map_err(ServeError::Bus)?
        .build()
        .await
        .map_err(ServeError::Bus)?;
    // Without `DoNotQueue`, a second process would silently wait in line.
    let flags = zbus::fdo::RequestNameFlags::DoNotQueue.into();
    match connection.request_name_with_flags(BUS_NAME, flags).await {
        Ok(
            zbus::fdo::RequestNameReply::PrimaryOwner | zbus::fdo::RequestNameReply::AlreadyOwner,
        ) => Ok(connection),
        Ok(_) | Err(zbus::Error::NameTaken) => Err(ServeError::AlreadyRunning),
        Err(e) => Err(ServeError::Bus(e)),
    }
}

#[zbus::proxy(
    interface = "io.github.diegofaraujo.NextGDrive.Sync1",
    default_service = "io.github.diegofaraujo.NextGDrive.Sync",
    default_path = "/io/github/diegofaraujo/NextGDrive/Sync"
)]
trait Sync1 {
    fn status(&self) -> zbus::Result<HostStatus>;
    fn sync_now(&self) -> zbus::Result<()>;
    fn pause(&self) -> zbus::Result<()>;
    fn resume(&self) -> zbus::Result<()>;
    fn confirm_deletions(&self, email: &str) -> zbus::Result<()>;
}

/// Controls the process running background sync.
pub struct Client(Sync1Proxy<'static>);

impl Client {
    /// `None` if no process runs background sync (or there is no session bus).
    pub async fn connect() -> Option<Self> {
        let connection = zbus::Connection::session().await.ok()?;
        let bus = zbus::fdo::DBusProxy::new(&connection).await.ok()?;
        let name = zbus::names::BusName::try_from(BUS_NAME).ok()?;
        if !bus.name_has_owner(name).await.ok()? {
            return None;
        }
        Some(Self(Sync1Proxy::new(&connection).await.ok()?))
    }

    pub async fn status(&self) -> zbus::Result<HostStatus> {
        self.0.status().await
    }

    pub async fn sync_now(&self) -> zbus::Result<()> {
        self.0.sync_now().await
    }

    pub async fn pause(&self) -> zbus::Result<()> {
        self.0.pause().await
    }

    pub async fn resume(&self) -> zbus::Result<()> {
        self.0.resume().await
    }

    pub async fn confirm_deletions(&self, email: &str) -> zbus::Result<()> {
        self.0.confirm_deletions(email).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<&'static str>>);

    impl HostControl for Recorder {
        fn status(&self) -> HostStatus {
            HostStatus {
                state: STATE_SYNCING.into(),
                account: "ana@x.com".into(),
                files_done: 1,
                files_total: 2,
                errors: vec![("bob@x.com".into(), "offline".into())],
                ..HostStatus::default()
            }
        }
        fn sync_now(&self) {
            self.0.lock().unwrap().push("sync_now");
        }
        fn pause(&self) {
            self.0.lock().unwrap().push("pause");
        }
        fn resume(&self) {
            self.0.lock().unwrap().push("resume");
        }
        fn confirm_deletions(&self, _: &str) {
            self.0.lock().unwrap().push("confirm");
        }
    }

    /// Uses the real session bus, and the real bus name: it fails (and must
    /// not be run) while Next GDrive itself is running.
    #[tokio::test]
    #[ignore = "needs a session bus and no running Next GDrive; run with --ignored"]
    async fn client_controls_the_served_host() {
        let recorder = Arc::new(Recorder::default());
        let _service = serve(recorder.clone()).await.unwrap();
        assert!(matches!(
            serve(recorder.clone()).await,
            Err(ServeError::AlreadyRunning)
        ));

        let client = Client::connect().await.expect("service is running");
        assert_eq!(client.status().await.unwrap(), recorder.status());
        client.sync_now().await.unwrap();
        client.pause().await.unwrap();
        client.resume().await.unwrap();
        assert_eq!(*recorder.0.lock().unwrap(), ["sync_now", "pause", "resume"]);
    }
}
