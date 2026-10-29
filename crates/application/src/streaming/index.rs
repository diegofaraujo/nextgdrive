//! The index: the metadata of every item in an account's Drive kept on this
//! computer (`ports::DriveIndex`), so streamed folders open without asking
//! Drive, also offline and right after a restart.

use std::time::{Duration, SystemTime};

use nextgdrive_domain::{Account, AccountId, Drive, DriveItem, DriveItemId};

use crate::ports::{
    AccessToken, Authenticator, ChangedItems, Changes, CredentialStore, DriveAccountInfo,
    DriveIndex, DriveService, FileReader, FileWriter, ItemChange, PortError, UploadTarget,
};
use crate::{AppError, TokenProvider};

/// The whole Drive is listed again this often, a safety net for anything
/// the change feed missed.
pub const REBUILD_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// What an index update changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexUpdate {
    /// Everything was listed again: any folder may have changed.
    pub rebuilt: bool,
    /// Folders whose content changed.
    pub folders: Vec<DriveItemId>,
    /// The list of shared drives may have changed.
    pub drives_changed: bool,
}

/// Brings an account's index up to date: lists the whole Drive the first
/// time (and every `REBUILD_INTERVAL`), then reads only the change feed.
pub struct UpdateIndex<'a, A, C, D, I> {
    tokens: &'a TokenProvider<A, C>,
    drive: &'a D,
    index: &'a I,
}

impl<'a, A, C, D, I> UpdateIndex<'a, A, C, D, I>
where
    A: Authenticator,
    C: CredentialStore,
    D: DriveService,
    I: DriveIndex,
{
    pub fn new(tokens: &'a TokenProvider<A, C>, drive: &'a D, index: &'a I) -> Self {
        Self {
            tokens,
            drive,
            index,
        }
    }

    pub async fn execute(&self, account: &Account) -> Result<IndexUpdate, AppError> {
        let status = self.index.status(&account.id).map_err(AppError::Storage)?;
        let token = self.tokens.access_token(account).await?;
        // A build time in the future (clock changed) also rebuilds.
        let current = status.filter(|s| s.built.elapsed().is_ok_and(|age| age < REBUILD_INTERVAL));
        let Some(status) = current else {
            self.rebuild(&token, account).await?;
            return Ok(IndexUpdate {
                rebuilt: true,
                ..IndexUpdate::default()
            });
        };
        let changed = self
            .drive
            .changed_items(&token, &status.cursor)
            .await
            .map_err(AppError::Drive)?;
        if changed.drives_changed {
            let drives = self.drive.drives(&token).await.map_err(AppError::Drive)?;
            self.index
                .set_drives(&account.id, &drives)
                .map_err(AppError::Storage)?;
        }
        let folders = self
            .index
            .apply(&account.id, &changed.changes, Some(&changed.cursor))
            .map_err(AppError::Storage)?;
        Ok(IndexUpdate {
            rebuilt: false,
            folders,
            drives_changed: changed.drives_changed,
        })
    }

    async fn rebuild(&self, token: &AccessToken, account: &Account) -> Result<(), AppError> {
        // The feed position is taken first, so what changes while listing
        // is read from the feed afterwards.
        let cursor = self
            .drive
            .changes_since(token, None)
            .await
            .map_err(AppError::Drive)?
            .cursor;
        let drives = self.drive.drives(token).await.map_err(AppError::Drive)?;
        let mut items = Vec::new();
        for drive in &drives {
            items.extend(
                self.drive
                    .items_in(token, drive)
                    .await
                    .map_err(AppError::Drive)?,
            );
        }
        self.index
            .rebuild(&account.id, &drives, &items, &cursor, SystemTime::now())
            .map_err(AppError::Storage)
    }
}

/// Drive for a streamed account: listings come from the index once it is
/// built, and what this computer changes in Drive is recorded there at once,
/// so the index never lags behind local writes. The rest goes to Drive.
pub struct IndexedDrive<D, I> {
    drive: D,
    index: I,
    account: AccountId,
}

impl<D: DriveService, I: DriveIndex> IndexedDrive<D, I> {
    pub fn new(drive: D, index: I, account: AccountId) -> Self {
        Self {
            drive,
            index,
            account,
        }
    }

    #[cfg(test)]
    pub(crate) fn inner(&self) -> (&D, &I) {
        (&self.drive, &self.index)
    }

    fn ready(&self) -> bool {
        matches!(self.index.status(&self.account), Ok(Some(_)))
    }

    /// Ahead of the change feed. If recording fails, the feed brings the
    /// change a little later.
    fn record(&self, change: ItemChange) {
        if self.ready() {
            let _ = self.index.apply(&self.account, &[change], None);
        }
    }
}

impl<D: DriveService, I: DriveIndex> DriveService for IndexedDrive<D, I> {
    async fn account_info(&self, token: &AccessToken) -> Result<DriveAccountInfo, PortError> {
        self.drive.account_info(token).await
    }

    fn known_drives(&self) -> Option<Vec<Drive>> {
        let drives = self
            .ready()
            .then(|| self.index.drives(&self.account).ok())??;
        (!drives.is_empty()).then_some(drives)
    }

    fn known_children(&self, folder: &DriveItemId) -> Option<Vec<DriveItem>> {
        self.ready()
            .then(|| self.index.children(&self.account, folder).ok())?
    }

    async fn drives(&self, token: &AccessToken) -> Result<Vec<Drive>, PortError> {
        match self.known_drives() {
            Some(drives) => Ok(drives),
            None => self.drive.drives(token).await,
        }
    }

    async fn children(
        &self,
        token: &AccessToken,
        folder: &DriveItemId,
    ) -> Result<Vec<DriveItem>, PortError> {
        match self.known_children(folder) {
            Some(children) => Ok(children),
            None => self.drive.children(token, folder).await,
        }
    }

    /// Always from Drive: callers ask to check what is there now.
    async fn item(&self, token: &AccessToken, id: &DriveItemId) -> Result<DriveItem, PortError> {
        self.drive.item(token, id).await
    }

    async fn changes_since(
        &self,
        token: &AccessToken,
        cursor: Option<&str>,
    ) -> Result<Changes, PortError> {
        self.drive.changes_since(token, cursor).await
    }

    async fn changed_items(
        &self,
        token: &AccessToken,
        cursor: &str,
    ) -> Result<ChangedItems, PortError> {
        self.drive.changed_items(token, cursor).await
    }

    async fn items_in(
        &self,
        token: &AccessToken,
        drive: &Drive,
    ) -> Result<Vec<DriveItem>, PortError> {
        self.drive.items_in(token, drive).await
    }

    async fn download<W: FileWriter>(
        &self,
        token: &AccessToken,
        id: &DriveItemId,
        out: &mut W,
    ) -> Result<(), PortError> {
        self.drive.download(token, id, out).await
    }

    async fn create_folder(
        &self,
        token: &AccessToken,
        parent: &DriveItemId,
        name: &str,
    ) -> Result<DriveItem, PortError> {
        let item = self.drive.create_folder(token, parent, name).await?;
        self.record(ItemChange::Changed(item.clone()));
        Ok(item)
    }

    async fn upload<R: FileReader>(
        &self,
        token: &AccessToken,
        target: UploadTarget<'_>,
        size: u64,
        modified: SystemTime,
        content: &mut R,
    ) -> Result<DriveItem, PortError> {
        let item = self
            .drive
            .upload(token, target, size, modified, content)
            .await?;
        self.record(ItemChange::Changed(item.clone()));
        Ok(item)
    }

    async fn move_item(
        &self,
        token: &AccessToken,
        id: &DriveItemId,
        from: &DriveItemId,
        to: &DriveItemId,
        name: &str,
    ) -> Result<DriveItem, PortError> {
        let item = self.drive.move_item(token, id, from, to, name).await?;
        self.record(ItemChange::Changed(item.clone()));
        Ok(item)
    }

    async fn trash(&self, token: &AccessToken, id: &DriveItemId) -> Result<(), PortError> {
        self.drive.trash(token, id).await?;
        self.record(ItemChange::Removed(id.clone()));
        Ok(())
    }

    async fn remove_from_my_drive(
        &self,
        token: &AccessToken,
        id: &DriveItemId,
        parent: &DriveItemId,
    ) -> Result<(), PortError> {
        self.drive.remove_from_my_drive(token, id, parent).await?;
        self.record(ItemChange::Removed(id.clone()));
        Ok(())
    }

    async fn trashed_items(
        &self,
        token: &AccessToken,
    ) -> Result<Vec<nextgdrive_domain::TrashedItem>, PortError> {
        self.drive.trashed_items(token).await
    }

    /// What was inside a restored folder is still in the index: trashing
    /// removes only the trashed item from it.
    async fn restore(&self, token: &AccessToken, id: &DriveItemId) -> Result<DriveItem, PortError> {
        let item = self.drive.restore(token, id).await?;
        self.record(ItemChange::Changed(item.clone()));
        Ok(item)
    }

    async fn download_range(
        &self,
        token: &AccessToken,
        id: &DriveItemId,
        offset: u64,
        len: u64,
    ) -> Result<Vec<u8>, PortError> {
        self.drive.download_range(token, id, offset, len).await
    }
}
