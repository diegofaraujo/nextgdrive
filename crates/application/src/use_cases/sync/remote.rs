//! Listing what should exist locally, according to Drive.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;

use nextgdrive_domain::local_name::{assign_names, local_name};
use nextgdrive_domain::{Account, Drive, DriveItem, DriveItemId, DriveKind, ItemKind, Settings};

use super::{SkipReason, SyncReport};
use crate::AppError;
use crate::ports::{AccessToken, DriveService, PortError};

/// Guards against malformed parent chains in Drive.
const MAX_DEPTH: usize = 256;

/// A Drive item that should exist locally.
#[derive(Debug, Clone)]
pub(super) struct Wanted {
    pub item: DriveItem,
    /// Relative to the account folder.
    pub path: PathBuf,
    /// Selected folder it is synced under.
    pub selection: DriveItemId,
}

/// Local folder of a selected Drive folder.
#[derive(Debug, Clone)]
pub(super) struct Root {
    pub selection: DriveItemId,
    pub path: PathBuf,
}

/// Lists every selected folder recursively. Any API error aborts the sync.
pub(super) async fn list<D: DriveService>(
    drive: &D,
    token: &AccessToken,
    account: &Account,
    settings: &Settings,
    report: &mut SyncReport,
) -> Result<(BTreeMap<DriveItemId, Wanted>, Vec<Root>), AppError> {
    let drives = drive.drives(token).await.map_err(AppError::Drive)?;
    let mut wanted = BTreeMap::new();
    let mut roots = Vec::new();
    for selection in settings.selection(&account.id) {
        if wanted.contains_key(selection) {
            continue; // Inside another selected folder.
        }
        let (root, path) = locate(drive, token, settings, &drives, selection).await?;
        roots.push(Root {
            selection: selection.clone(),
            path: path.clone(),
        });
        wanted.insert(
            root.id.clone(),
            Wanted {
                item: root,
                path: path.clone(),
                selection: selection.clone(),
            },
        );
        let mut queue = VecDeque::from([(selection.clone(), path)]);
        while let Some((folder, path)) = queue.pop_front() {
            let children = drive
                .children(token, &folder)
                .await
                .map_err(AppError::Drive)?;
            let (local, other): (Vec<_>, Vec<_>) = children.into_iter().partition(has_local_copy);
            for item in other {
                let reason = match item.kind {
                    ItemKind::Shortcut => SkipReason::Shortcut,
                    _ => SkipReason::NotDownloadable,
                };
                report.skipped.push((path.join(&item.name), reason));
            }
            let names = assign_names(local.iter().map(|i| (&i.id, local_name(&i.name, &i.kind))));
            for item in local {
                if wanted.contains_key(&item.id) {
                    continue; // Seen through another parent or selection.
                }
                let child = path.join(&names[&item.id]);
                if item.kind == ItemKind::Folder {
                    queue.push_back((item.id.clone(), child.clone()));
                }
                wanted.insert(
                    item.id.clone(),
                    Wanted {
                        item,
                        path: child,
                        selection: selection.clone(),
                    },
                );
            }
        }
    }
    Ok((wanted, roots))
}

/// Finds a selected folder and its path relative to the account folder:
/// the drive's folder, then each ancestor's local name.
async fn locate<D: DriveService>(
    drive: &D,
    token: &AccessToken,
    settings: &Settings,
    drives: &[Drive],
    id: &DriveItemId,
) -> Result<(DriveItem, PathBuf), AppError> {
    let target = drive.item(token, id).await.map_err(AppError::Drive)?;
    if target.kind != ItemKind::Folder {
        return Err(AppError::NotAFolder(id.clone()));
    }
    // Climb to the drive root, remembering the chain.
    let mut chain = vec![target.clone()];
    let top_drive = loop {
        let top = chain.last().expect("chain is never empty");
        if let Some(d) = drives.iter().find(|d| d.root == top.id) {
            break d;
        }
        let parent = match top.parents.first() {
            Some(parent) if chain.len() < MAX_DEPTH => parent.clone(),
            // Shared with the user but outside any of their drives.
            _ => {
                return Err(AppError::Drive(PortError::new(format!(
                    "{id} is not inside a drive"
                ))));
            }
        };
        chain.push(drive.item(token, &parent).await.map_err(AppError::Drive)?);
    };
    let mut path = drive_folder(settings, drives, top_drive);
    // Walk back down; each name must be unique among its siblings.
    for pair in chain.windows(2).rev() {
        let (child, parent) = (&pair[0], &pair[1]);
        let siblings = drive
            .children(token, &parent.id)
            .await
            .map_err(AppError::Drive)?;
        let names = assign_names(
            siblings
                .iter()
                .filter(|s| has_local_copy(s))
                .map(|s| (&s.id, local_name(&s.name, &s.kind))),
        );
        let name = names
            .get(&child.id)
            .cloned()
            .unwrap_or_else(|| local_name(&child.name, &child.kind));
        path.push(name);
    }
    Ok((target, path))
}

pub(super) fn has_local_copy(item: &DriveItem) -> bool {
    matches!(
        item.kind,
        ItemKind::Folder | ItemKind::File { .. } | ItemKind::GoogleDoc { .. }
    )
}

/// Local folder of a drive, relative to the account folder.
fn drive_folder(settings: &Settings, drives: &[Drive], drive: &Drive) -> PathBuf {
    match &drive.kind {
        DriveKind::MyDrive => PathBuf::from(&settings.my_drive_folder),
        DriveKind::Shared { .. } => {
            let shared = drives.iter().filter_map(|d| match &d.kind {
                DriveKind::Shared { name } => Some((&d.root, local_name(name, &ItemKind::Folder))),
                DriveKind::MyDrive => None,
            });
            let names = assign_names(shared);
            PathBuf::from(&settings.shared_drives_folder).join(&names[&drive.root])
        }
    }
}
