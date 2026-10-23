//! Translations shared by the GUI and the CLI, using Fluent (`.ftl` files
//! under `locales/`). English (`en-US`) is the reference and fallback locale.

use std::borrow::Cow;
use std::collections::HashMap;

pub use fluent_templates::fluent_bundle::FluentValue;
use fluent_templates::{LanguageIdentifier, Loader, langid};
use nextgdrive_application::AppError;
use nextgdrive_application::use_cases::{Conflict, ConflictReason, SyncReport};
use nextgdrive_domain::{DomainError, StorageQuota};

fluent_templates::static_loader! {
    static LOCALES = {
        locales: "./locales",
        fallback_language: "en-US",
        // Unicode isolation marks around arguments show up as garbage in terminals.
        customise: |bundle| bundle.set_use_isolating(false),
    };
}

const FALLBACK: LanguageIdentifier = langid!("en-US");

/// Supported locales. Any other variant of a supported language (e.g. `pt-PT`,
/// `es-MX`) is served by the entry with the same language.
static SUPPORTED: [LanguageIdentifier; 3] = [langid!("en-US"), langid!("pt-BR"), langid!("es")];

#[derive(Clone)]
pub struct Localizer {
    lang: LanguageIdentifier,
}

impl Localizer {
    /// Uses the system locale (`LC_ALL`, `LC_MESSAGES`, `LANG`).
    pub fn from_system() -> Self {
        Self::new(&sys_locale::get_locale().unwrap_or_default())
    }

    /// Picks the best supported match for a BCP 47 tag, falling back to English.
    pub fn new(tag: &str) -> Self {
        // POSIX locales look like `pt_BR.UTF-8`.
        let tag = tag
            .split(['.', '@'])
            .next()
            .unwrap_or_default()
            .replace('_', "-");
        let lang = tag
            .parse::<LanguageIdentifier>()
            .ok()
            .and_then(|requested| {
                SUPPORTED
                    .iter()
                    .find(|s| **s == requested)
                    .or_else(|| SUPPORTED.iter().find(|s| s.language == requested.language))
                    .cloned()
            })
            .unwrap_or(FALLBACK);
        Self { lang }
    }

    pub fn language(&self) -> &LanguageIdentifier {
        &self.lang
    }

    pub fn tr(&self, id: &str) -> String {
        LOCALES.lookup(&self.lang, id)
    }

    pub fn tr_args<'a>(
        &self,
        id: &str,
        args: impl IntoIterator<Item = (&'static str, FluentValue<'a>)>,
    ) -> String {
        let args: HashMap<Cow<'static, str>, FluentValue<'a>> = args
            .into_iter()
            .map(|(k, v)| (Cow::Borrowed(k), v))
            .collect();
        LOCALES.lookup_with_args(&self.lang, id, &args)
    }

    /// User-facing message for an application error.
    pub fn app_error(&self, error: &AppError) -> String {
        let details =
            |id, e: &dyn std::fmt::Display| self.tr_args(id, [("details", e.to_string().into())]);
        match error {
            AppError::AccountNotFound(email) => self.tr_args(
                "error-account-not-found",
                [("email", email.to_string().into())],
            ),
            AppError::SyncInProgress(_) => self.tr("error-sync-in-progress"),
            AppError::PendingUploads(count) => self.tr_args(
                "error-pending-uploads",
                [("count", FluentValue::from(*count))],
            ),
            AppError::FolderExists(path) => self.tr_args(
                "error-folder-exists",
                [("path", path.display().to_string().into())],
            ),
            AppError::LocalFolderMissing(path) => self.tr_args(
                "error-local-folder-missing",
                [("path", path.display().to_string().into())],
            ),
            AppError::MassDeletion { count } => self.tr_args(
                "error-mass-deletion",
                [("count", FluentValue::from(*count))],
            ),
            AppError::MissingCredentials(email) => self.tr_args(
                "error-missing-credentials",
                [("email", email.to_string().into())],
            ),
            AppError::InvalidSyncRoot(path) => self.tr_args(
                "error-invalid-sync-root",
                [("path", path.display().to_string().into())],
            ),
            AppError::NotAFolder(id) => {
                self.tr_args("error-not-a-folder", [("id", id.to_string().into())])
            }
            AppError::Storage(e) => details("error-storage", e),
            AppError::Auth(e) => details("error-auth", e),
            AppError::Credentials(e) => details("error-credentials", e),
            AppError::Drive(e) => details("error-drive", e),
        }
    }

    /// Size in the units Google uses: powers of 1024 labeled KB, MB, GB, TB.
    pub fn bytes(&self, bytes: u64) -> String {
        const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
        #[allow(clippy::cast_precision_loss)] // Display only.
        let mut value = bytes as f64;
        let mut unit = 0;
        while value >= 1024.0 && unit < UNITS.len() - 1 {
            value /= 1024.0;
            unit += 1;
        }
        // One decimal for small values ("1.5 GB"), none for large ones ("15 GB").
        let number = if unit > 0 && value < 10.0 && value.fract() >= 0.05 {
            format!("{value:.1}").replace('.', &self.tr("decimal-separator"))
        } else {
            format!("{value:.0}")
        };
        format!("{number} {}", UNITS[unit])
    }

    pub fn quota(&self, quota: &StorageQuota) -> String {
        let used = ("used", self.bytes(quota.usage).into());
        match (quota.limit, quota.available()) {
            (Some(limit), Some(free)) => self.tr_args(
                "quota-used",
                [
                    used,
                    ("total", self.bytes(limit).into()),
                    ("free", self.bytes(free).into()),
                ],
            ),
            _ => self.tr_args("quota-used-unlimited", [used]),
        }
    }

    /// What a sync did, e.g. "3 downloaded (1.2 MB), 1 uploaded (40 KB)".
    pub fn sync_summary(&self, report: &SyncReport) -> String {
        let count = |id: &str, n: usize| self.tr_args(id, [("count", FluentValue::from(n))]);
        let sized = |id: &str, n: usize, bytes: u64| {
            self.tr_args(
                id,
                [
                    ("count", FluentValue::from(n)),
                    ("bytes", self.bytes(bytes).into()),
                ],
            )
        };
        let mut parts = Vec::new();
        if report.downloaded > 0 {
            parts.push(sized(
                "sync-part-downloaded",
                report.downloaded,
                report.downloaded_bytes,
            ));
        }
        if report.uploaded > 0 {
            parts.push(sized(
                "sync-part-uploaded",
                report.uploaded,
                report.uploaded_bytes,
            ));
        }
        for (id, n) in [
            ("sync-part-moved", report.moved + report.moved_in_drive),
            ("sync-part-removed", report.removed),
            ("sync-part-trashed", report.trashed),
            ("sync-part-copies", report.conflict_copies.len()),
        ] {
            if n > 0 {
                parts.push(count(id, n));
            }
        }
        if parts.is_empty() {
            return self.tr("sync-part-nothing");
        }
        parts.join(", ")
    }

    /// Local date and time, in the format the translation chooses (`date-format`,
    /// strftime syntax).
    pub fn date_time(&self, time: std::time::SystemTime) -> String {
        let format = self.tr("date-format");
        jiff::Timestamp::try_from(time)
            .map(|t| {
                t.to_zoned(jiff::tz::TimeZone::system())
                    .strftime(&format)
                    .to_string()
            })
            .unwrap_or_default()
    }

    /// Short explanation of why an item was left alone.
    pub fn conflict_reason(&self, reason: ConflictReason) -> String {
        self.tr(match reason {
            ConflictReason::TypeClash => "conflict-type-clash",
            ConflictReason::ReadOnly => "conflict-read-only",
            ConflictReason::CannotDelete => "conflict-cannot-delete",
        })
    }

    /// One line about an item left alone, for logs and the command line.
    pub fn conflict_line(&self, conflict: &Conflict) -> String {
        format!(
            "{}: {}",
            self.conflict_reason(conflict.reason),
            conflict.path.display()
        )
    }

    /// User-facing message for a domain error.
    pub fn domain_error(&self, error: &DomainError) -> String {
        match error {
            DomainError::InvalidEmail(raw) => {
                self.tr_args("error-invalid-email", [("email", raw.clone().into())])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;

    use super::*;

    #[test]
    fn negotiates_supported_locale() {
        for (tag, expected) in [
            ("pt-BR", "pt-BR"),
            ("pt_BR.UTF-8", "pt-BR"),
            ("pt-PT", "pt-BR"),
            ("es-MX", "es"),
            ("en-GB", "en-US"),
            ("de-DE", "en-US"),
            ("C", "en-US"),
            ("", "en-US"),
        ] {
            assert_eq!(
                Localizer::new(tag).language().to_string(),
                expected,
                "{tag:?}"
            );
        }
    }

    #[test]
    fn translates_with_arguments() {
        let pt = Localizer::new("pt-BR");
        assert_eq!(
            pt.tr_args("error-account-not-found", [("email", "ana@x.com".into())]),
            "Nenhuma conta com o e-mail ana@x.com."
        );
    }

    #[test]
    fn formats_dates_per_locale() {
        let time =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_761_000_000);
        let pt = Localizer::new("pt-BR").date_time(time);
        let en = Localizer::new("en-US").date_time(time);
        assert_eq!(pt.len(), "20/10/2025 19:40".len(), "{pt}");
        assert!(pt.contains("/2025"), "{pt}");
        assert!(en.starts_with("2025-10-"), "{en}");
    }

    #[test]
    fn formats_sizes_per_locale() {
        let (en, pt) = (Localizer::new("en-US"), Localizer::new("pt-BR"));
        assert_eq!(en.bytes(0), "0 B");
        assert_eq!(en.bytes(1536), "1.5 KB");
        assert_eq!(pt.bytes(1536), "1,5 KB");
        assert_eq!(pt.bytes(5 << 30), "5 GB");
        assert_eq!(pt.bytes(15 << 30), "15 GB");
        assert_eq!(
            pt.quota(&StorageQuota {
                usage: 3 << 29,
                limit: Some(15 << 30)
            }),
            "1,5 GB de 15 GB usados (14 GB livres)"
        );
        assert_eq!(
            en.quota(&StorageQuota {
                usage: 10,
                limit: None
            }),
            "10 B used"
        );
    }

    fn message_ids(locale: &str) -> BTreeSet<String> {
        let path = format!("{}/locales/{locale}/main.ftl", env!("CARGO_MANIFEST_DIR"));
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .filter_map(|line| line.split_once(" =").map(|(id, _)| id))
            .filter(|id| id.starts_with(|c: char| c.is_ascii_lowercase()))
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn all_locales_define_the_same_messages() {
        let reference = message_ids("en-US");
        for locale in SUPPORTED.iter().map(ToString::to_string) {
            assert_eq!(
                message_ids(&locale),
                reference,
                "messages differ in {locale}"
            );
        }
    }
}
