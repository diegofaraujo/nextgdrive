//! Rules mapping Drive names to local file names.
//!
//! Drive allows names Linux cannot store (`/`, `.`, empty, very long) and
//! several items with the same name in one folder; local names must be valid
//! and unique among siblings.

use std::collections::{BTreeMap, HashSet};

use crate::{DriveItemId, ItemKind};

/// Linux limit for one path component, in bytes.
const MAX_NAME_BYTES: usize = 255;
/// Extension of the shortcut files that stand in for Google Docs. An HTML page
/// opens in the browser from any file manager, unlike `.desktop` links.
pub const GOOGLE_DOC_EXTENSION: &str = "html";

/// Contents of the file that stands in for a Google Doc: a page that
/// redirects to the document. Only `https` URLs are linked, so a crafted URL
/// cannot run script in the page.
pub fn google_doc_shortcut(name: &str, url: &str) -> String {
    let url = if url.starts_with("https://") {
        url
    } else {
        "about:blank"
    };
    let (name, url) = (escape_html(name), escape_html(url));
    format!(
        "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\">\
         <meta http-equiv=\"refresh\" content=\"0; url={url}\">\
         <title>{name}</title></head>\n<body><a href=\"{url}\">{name}</a></body></html>\n"
    )
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Local files never uploaded: temporary and lock files of common editors and
/// file managers, and Next GDrive's own partial downloads.
pub fn is_ignored(name: &str) -> bool {
    const PREFIXES: [&str; 3] = [".~lock.", "~$", ".goutputstream-"];
    const SUFFIXES: [&str; 5] = [".swp", ".swo", ".swx", "~", PARTIAL_SUFFIX];
    const NAMES: [&str; 3] = [".DS_Store", "Thumbs.db", "desktop.ini"];
    PREFIXES.iter().any(|p| name.starts_with(p))
        || SUFFIXES.iter().any(|s| name.ends_with(s))
        || NAMES.contains(&name)
}

/// Suffix of files being downloaded (see the local filesystem adapter).
pub const PARTIAL_SUFFIX: &str = ".nextgdrive-part";

/// Name of the copy that keeps local changes when a file also changed in
/// Drive: `report (conflicted copy 2026-10-23 14-30).pdf`.
pub fn conflict_copy_name(name: &str, label: &str) -> String {
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    };
    let suffix = format!(" ({label})");
    let stem = truncate(
        stem,
        MAX_NAME_BYTES.saturating_sub(suffix.len() + ext.len()),
    );
    format!("{stem}{suffix}{ext}")
}

/// Drive name for a local file name: Google Doc shortcuts drop their extension.
pub fn drive_name(local: &str, kind: &ItemKind) -> String {
    match kind {
        ItemKind::GoogleDoc { .. } => local
            .strip_suffix(&format!(".{GOOGLE_DOC_EXTENSION}"))
            .unwrap_or(local)
            .to_owned(),
        _ => local.to_owned(),
    }
}

/// Valid local file name for an item, before resolving clashes between siblings.
pub fn local_name(name: &str, kind: &ItemKind) -> String {
    let mut local: String = name
        .chars()
        .filter(|c| *c != '\0')
        // Lookalike that keeps the name readable.
        .map(|c| if c == '/' { '／' } else { c })
        .collect();
    if local.is_empty() || local == "." || local == ".." {
        local = format!("_{local}");
    }
    if matches!(kind, ItemKind::GoogleDoc { .. }) {
        local = format!("{local}.{GOOGLE_DOC_EXTENSION}");
    }
    truncate(&local, MAX_NAME_BYTES)
}

/// Assigns unique local names to the items of one folder. Clashing names get a
/// numeric suffix (`report (2).pdf`). Deterministic: the same input always
/// yields the same names, so items do not swap names between syncs.
pub fn assign_names<'a>(
    items: impl IntoIterator<Item = (&'a DriveItemId, String)>,
) -> BTreeMap<DriveItemId, String> {
    let mut by_name: BTreeMap<String, Vec<&DriveItemId>> = BTreeMap::new();
    for (id, name) in items {
        by_name.entry(name).or_default().push(id);
    }
    let mut taken: HashSet<String> = by_name.keys().cloned().collect();
    let mut assigned = BTreeMap::new();
    for (name, mut ids) in by_name {
        ids.sort();
        let mut ids = ids.into_iter();
        if let Some(first) = ids.next() {
            assigned.insert(first.clone(), name.clone());
        }
        let mut n = 2;
        for id in ids {
            let candidate = loop {
                let candidate = with_suffix(&name, n);
                n += 1;
                if taken.insert(candidate.clone()) {
                    break candidate;
                }
            };
            assigned.insert(id.clone(), candidate);
        }
    }
    assigned
}

/// `report.pdf` → `report (2).pdf`; names without extension get the suffix at the end.
fn with_suffix(name: &str, n: u32) -> String {
    let suffix = format!(" ({n})");
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    };
    let stem = truncate(
        stem,
        MAX_NAME_BYTES.saturating_sub(suffix.len() + ext.len()),
    );
    format!("{stem}{suffix}{ext}")
}

/// Cuts to at most `max` bytes on a char boundary, keeping the extension when possible.
fn truncate(name: &str, max: usize) -> String {
    if name.len() <= max {
        return name.to_owned();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 && name.len() - dot <= 16 => name.split_at(dot),
        _ => (name, ""),
    };
    let mut end = max.saturating_sub(ext.len());
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{ext}", &stem[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file() -> ItemKind {
        ItemKind::File { size: 0, md5: None }
    }

    #[test]
    fn replaces_characters_linux_cannot_store() {
        assert_eq!(local_name("a/b", &file()), "a／b");
        assert_eq!(local_name("a\0b", &file()), "ab");
        assert_eq!(local_name("", &file()), "_");
        assert_eq!(local_name(".", &file()), "_.");
        assert_eq!(local_name("..", &file()), "_..");
    }

    #[test]
    fn google_docs_become_html_files() {
        let doc = ItemKind::GoogleDoc {
            url: "https://docs.google.com/x".into(),
        };
        assert_eq!(local_name("Budget", &doc), "Budget.html");
    }

    #[test]
    fn shortcut_redirects_to_the_document() {
        let page = google_doc_shortcut("Budget", "https://docs.google.com/d/1?a=1&b=2");
        assert!(page.contains(r#"content="0; url=https://docs.google.com/d/1?a=1&amp;b=2""#));
        assert!(page.contains("<title>Budget</title>"));
    }

    #[test]
    fn shortcut_cannot_inject_markup_or_script() {
        let page = google_doc_shortcut("<script>alert(1)</script>\"", "javascript:alert(1)");
        assert!(!page.contains("<script>"));
        assert!(!page.contains("javascript:"));
        assert!(page.contains("url=about:blank"));
    }

    #[test]
    fn ignores_temporary_files() {
        for name in [
            ".~lock.report.odt#",
            "~$report.docx",
            ".goutputstream-AB12",
            "notes.txt.swp",
            "a.txt~",
            ".a.txt.nextgdrive-part",
            ".DS_Store",
        ] {
            assert!(is_ignored(name), "{name}");
        }
        for name in ["report.odt", "lock.txt", ".bashrc", "~notes"] {
            assert!(!is_ignored(name), "{name}");
        }
    }

    #[test]
    fn conflict_copies_keep_the_extension() {
        assert_eq!(
            conflict_copy_name("report.pdf", "conflicted copy 2026-10-23 14-30"),
            "report (conflicted copy 2026-10-23 14-30).pdf"
        );
        assert_eq!(conflict_copy_name("README", "copy"), "README (copy)");
    }

    #[test]
    fn shortcut_names_map_back_to_document_names() {
        let doc = ItemKind::GoogleDoc { url: String::new() };
        assert_eq!(drive_name("Budget 2025.html", &doc), "Budget 2025");
        assert_eq!(
            drive_name("page.html", &ItemKind::File { size: 0, md5: None }),
            "page.html"
        );
    }

    #[test]
    fn long_names_are_truncated_keeping_extension() {
        let name = format!("{}.pdf", "é".repeat(200));
        let local = local_name(&name, &file());
        assert!(local.len() <= MAX_NAME_BYTES);
        assert!(local.ends_with("é.pdf"));
    }

    #[test]
    fn clashing_names_get_numeric_suffixes() {
        let ids: Vec<_> = ["c", "a", "b", "d"].map(DriveItemId::new).into();
        let names = assign_names([
            (&ids[0], "report.pdf".to_owned()),
            (&ids[1], "report.pdf".to_owned()),
            (&ids[2], "report (2).pdf".to_owned()),
            (&ids[3], "notes".to_owned()),
        ]);
        // "a" sorts first and keeps the name; "c" skips the taken "(2)".
        assert_eq!(names[&ids[1]], "report.pdf");
        assert_eq!(names[&ids[0]], "report (3).pdf");
        assert_eq!(names[&ids[2]], "report (2).pdf");
        assert_eq!(names[&ids[3]], "notes");
    }

    #[test]
    fn suffix_goes_before_extension_but_not_for_dotfiles() {
        assert_eq!(with_suffix("a.tar", 2), "a (2).tar");
        assert_eq!(with_suffix(".bashrc", 2), ".bashrc (2)");
        assert_eq!(with_suffix("README", 3), "README (3)");
    }
}
