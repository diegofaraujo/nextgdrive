//! Translations shared by the GUI and the CLI, using Fluent (`.ftl` files
//! under `locales/`). English (`en-US`) is the reference and fallback locale.

use std::borrow::Cow;
use std::collections::HashMap;

pub use fluent_templates::fluent_bundle::FluentValue;
use fluent_templates::{LanguageIdentifier, Loader, langid};
use nextgdrive_application::AppError;
use nextgdrive_domain::DomainError;

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
        match error {
            AppError::AccountNotFound(email) => self.tr_args(
                "error-account-not-found",
                [("email", email.to_string().into())],
            ),
            AppError::Repository(e) => {
                self.tr_args("error-storage", [("details", e.to_string().into())])
            }
        }
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
