//! Staging area for streamed files being written: `<key>.data` holds the
//! content, `<key>.json` where it goes in Drive.

use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use nextgdrive_application::ports::{PortError, StagedFile, StagedTarget, StagingArea};
use nextgdrive_domain::DriveItemId;
use serde::{Deserialize, Serialize};

pub struct DiskStaging {
    dir: PathBuf,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TargetFile {
    Existing { id: String, base_version: String },
    New { parent: String },
}

#[derive(Serialize, Deserialize)]
struct MetaFile {
    key: String,
    name: String,
    target: TargetFile,
    modified_nanos: u64,
}

fn error(path: &Path, e: impl std::fmt::Display) -> PortError {
    PortError::new(format!("{}: {e}", path.display()))
}

impl DiskStaging {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, PortError> {
        let dir = dir.into();
        fs::create_dir_all(&dir).map_err(|e| error(&dir, e))?;
        Ok(Self { dir })
    }

    /// Keys become file names: keep only safe characters.
    fn path(&self, key: &str, extension: &str) -> PathBuf {
        let safe: String = key
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.dir.join(format!("{safe}.{extension}"))
    }

    fn data(&self, key: &str, write: bool) -> Result<File, PortError> {
        let path = self.path(key, "data");
        OpenOptions::new()
            .read(true)
            .write(write)
            .open(&path)
            .map_err(|e| error(&path, e))
    }

    fn save_meta(&self, file: &StagedFile) -> Result<(), PortError> {
        let meta = MetaFile {
            key: file.key.clone(),
            name: file.name.clone(),
            target: match &file.target {
                StagedTarget::Existing { id, base_version } => TargetFile::Existing {
                    id: id.to_string(),
                    base_version: base_version.clone(),
                },
                StagedTarget::New { parent } => TargetFile::New {
                    parent: parent.to_string(),
                },
            },
            modified_nanos: u64::try_from(
                file.modified
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
            )
            .unwrap_or(u64::MAX),
        };
        let path = self.path(&file.key, "json");
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_vec(&meta).map_err(|e| error(&path, e))?;
        // Written aside and synced, then renamed: never half a description.
        let mut out = File::create(&tmp).map_err(|e| error(&tmp, e))?;
        out.write_all(&text)
            .and_then(|()| out.sync_all())
            .map_err(|e| error(&tmp, e))?;
        fs::rename(&tmp, &path).map_err(|e| error(&path, e))
    }
}

fn load_meta(path: &Path) -> io::Result<StagedFile> {
    let mut text = String::new();
    File::open(path)?.read_to_string(&mut text)?;
    let meta: MetaFile = serde_json::from_str(&text).map_err(io::Error::other)?;
    Ok(StagedFile {
        key: meta.key,
        name: meta.name,
        target: match meta.target {
            TargetFile::Existing { id, base_version } => StagedTarget::Existing {
                id: DriveItemId::new(id),
                base_version,
            },
            TargetFile::New { parent } => StagedTarget::New {
                parent: DriveItemId::new(parent),
            },
        },
        modified: SystemTime::UNIX_EPOCH + Duration::from_nanos(meta.modified_nanos),
    })
}

impl StagingArea for DiskStaging {
    fn create(&self, file: &StagedFile) -> Result<(), PortError> {
        let path = self.path(&file.key, "data");
        File::create(&path).map_err(|e| error(&path, e))?;
        self.save_meta(file)
    }

    fn update(&self, file: &StagedFile) -> Result<(), PortError> {
        self.save_meta(file)
    }

    fn list(&self) -> Result<Vec<StagedFile>, PortError> {
        let mut files = Vec::new();
        for entry in fs::read_dir(&self.dir).map_err(|e| error(&self.dir, e))? {
            let path = entry.map_err(|e| error(&self.dir, e))?.path();
            if path.extension().is_some_and(|e| e == "json") {
                // A description without content is useless; skip it.
                match load_meta(&path) {
                    Ok(file) if self.path(&file.key, "data").is_file() => files.push(file),
                    Ok(_) => {}
                    Err(e) => return Err(error(&path, e)),
                }
            }
        }
        Ok(files)
    }

    fn len(&self, key: &str) -> Result<u64, PortError> {
        let path = self.path(key, "data");
        fs::metadata(&path)
            .map(|m| m.len())
            .map_err(|e| error(&path, e))
    }

    fn read_at(&self, key: &str, offset: u64, size: usize) -> Result<Vec<u8>, PortError> {
        let mut file = self.data(key, false)?;
        file.seek(SeekFrom::Start(offset)).map_err(PortError::new)?;
        let mut data = Vec::with_capacity(size);
        file.take(size as u64)
            .read_to_end(&mut data)
            .map_err(PortError::new)?;
        Ok(data)
    }

    fn write_at(&self, key: &str, offset: u64, data: &[u8]) -> Result<(), PortError> {
        self.data(key, true)?
            .write_all_at(data, offset)
            .map_err(PortError::new)
    }

    fn set_len(&self, key: &str, len: u64) -> Result<(), PortError> {
        self.data(key, true)?.set_len(len).map_err(PortError::new)
    }

    fn remove(&self, key: &str) -> Result<(), PortError> {
        // Description first: content without one is never picked up again.
        for extension in ["json", "data"] {
            let path = self.path(key, extension);
            match fs::remove_file(&path) {
                Err(e) if e.kind() != ErrorKind::NotFound => return Err(error(&path, e)),
                _ => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(key: &str) -> StagedFile {
        StagedFile {
            key: key.into(),
            name: "relatório.txt".into(),
            target: StagedTarget::Existing {
                id: DriveItemId::new("abc"),
                base_version: "md5".into(),
            },
            modified: SystemTime::UNIX_EPOCH + Duration::from_nanos(1_700_000_000_123_456_789),
        }
    }

    #[test]
    fn keeps_content_and_details_across_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let staging = DiskStaging::open(dir.path()).unwrap();
        staging.create(&file("k1")).unwrap();
        staging.write_at("k1", 0, b"hello world").unwrap();
        staging.write_at("k1", 6, b"there").unwrap();
        staging.set_len("k1", 8).unwrap();

        let reopened = DiskStaging::open(dir.path()).unwrap();
        assert_eq!(reopened.list().unwrap(), [file("k1")]);
        assert_eq!(reopened.len("k1").unwrap(), 8);
        assert_eq!(reopened.read_at("k1", 4, 100).unwrap(), b"o th");

        reopened.remove("k1").unwrap();
        assert!(reopened.list().unwrap().is_empty());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn keys_cannot_escape_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let staging = DiskStaging::open(dir.path().join("staging")).unwrap();
        staging.create(&file("../../evil")).unwrap();
        assert!(
            staging
                .path("../../evil", "data")
                .starts_with(dir.path().join("staging"))
        );
    }
}
