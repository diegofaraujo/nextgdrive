//! Command-line definition. Built with clap's builder API rather than derive,
//! because help texts must come from the translation files at runtime.

use std::path::PathBuf;

use clap::{Arg, ArgAction, ArgGroup, ArgMatches, value_parser};
use nextgdrive_application::use_cases::Deletions;
use nextgdrive_i18n::Localizer;

pub enum Command {
    Daemon,
    Status,
    Account(AccountCommand),
    Config(ConfigCommand),
    Sync(SyncCommand),
    Trash(TrashCommand),
    Stream { email: String, mountpoint: PathBuf },
    Offline(OfflineCommand),
    Cache(CacheCommand),
}

pub enum CacheCommand {
    Show,
    Clear,
    Limit { bytes: u64 },
}

pub enum OfflineCommand {
    Add { path: PathBuf },
    Remove { path: PathBuf },
    List,
}

pub enum TrashCommand {
    List { email: String },
    Restore { email: String, id: String },
}

pub enum AccountCommand {
    List,
    Add,
    Status { email: Option<String> },
    Remove { email: String },
}

pub enum ConfigCommand {
    Show,
    SyncRoot { path: PathBuf },
    Mode { email: String, stream: bool },
}

pub enum SyncCommand {
    Run {
        email: Option<String>,
        deletions: Deletions,
    },
    Folders {
        email: String,
        folder: Option<String>,
    },
    Select {
        email: String,
        folder: String,
    },
    Deselect {
        email: String,
        folder: String,
        remove_local: bool,
    },
}

pub fn parse(i18n: &Localizer) -> Command {
    from_matches(&command(i18n).get_matches())
}

fn command(i18n: &Localizer) -> clap::Command {
    let email = |help: &str| {
        Arg::new("email")
            .value_name("EMAIL")
            .required(true)
            .help(i18n.tr(help))
    };
    let folder = || {
        Arg::new("folder")
            .value_name("FOLDER_ID")
            .required(true)
            .help(i18n.tr("cli-sync-folder-id"))
    };
    let group = |name: &'static str, about: &str| {
        clap::Command::new(name)
            .about(i18n.tr(about))
            .subcommand_required(true)
            .arg_required_else_help(true)
    };

    let root = clap::Command::new("nextgdrive")
        .version(clap::crate_version!())
        .about(i18n.tr("cli-about"))
        .subcommand_required(true)
        .arg_required_else_help(true)
        // Replace clap's built-in help/version flags, whose texts are English-only.
        .disable_help_flag(true)
        .disable_version_flag(true)
        .disable_help_subcommand(true)
        .arg(
            Arg::new("help")
                .short('h')
                .long("help")
                .global(true)
                .action(ArgAction::Help)
                .help(i18n.tr("cli-help")),
        )
        .arg(
            Arg::new("version")
                .short('V')
                .long("version")
                .action(ArgAction::Version)
                .help(i18n.tr("cli-version")),
        )
        .subcommand(clap::Command::new("status").about(i18n.tr("cli-status-about")))
        .subcommand(clap::Command::new("daemon").about(i18n.tr("cli-daemon-about")))
        .subcommand(
            group("account", "cli-account-about")
                .subcommand(clap::Command::new("list").about(i18n.tr("cli-account-list-about")))
                .subcommand(clap::Command::new("add").about(i18n.tr("cli-account-add-about")))
                .subcommand(
                    clap::Command::new("status")
                        .about(i18n.tr("cli-account-status-about"))
                        .arg(email("cli-account-status-email").required(false)),
                )
                .subcommand(
                    clap::Command::new("remove")
                        .about(i18n.tr("cli-account-remove-about"))
                        .arg(email("cli-account-remove-email")),
                ),
        )
        .subcommand(
            group("config", "cli-config-about")
                .subcommand(clap::Command::new("show").about(i18n.tr("cli-config-show-about")))
                .subcommand(
                    clap::Command::new("sync-root")
                        .about(i18n.tr("cli-config-sync-root-about"))
                        .arg(
                            Arg::new("path")
                                .value_name("PATH")
                                .required(true)
                                .value_parser(value_parser!(PathBuf))
                                .help(i18n.tr("cli-config-sync-root-path")),
                        ),
                )
                .subcommand(
                    clap::Command::new("mode")
                        .about(i18n.tr("cli-config-mode-about"))
                        .arg(email("cli-account-email"))
                        .arg(
                            Arg::new("mode")
                                .value_name("MODE")
                                .required(true)
                                .value_parser(["mirror", "stream"])
                                .help(i18n.tr("cli-config-mode-value")),
                        ),
                ),
        )
        .subcommand(
            group("sync", "cli-sync-about")
                .subcommand(
                    clap::Command::new("run")
                        .about(i18n.tr("cli-sync-run-about"))
                        .arg(email("cli-sync-run-email").required(false))
                        .arg(
                            Arg::new("confirm-deletions")
                                .long("confirm-deletions")
                                .action(ArgAction::SetTrue)
                                .help(i18n.tr("cli-sync-confirm-deletions")),
                        )
                        .arg(
                            Arg::new("restore-deletions")
                                .long("restore-deletions")
                                .action(ArgAction::SetTrue)
                                .help(i18n.tr("cli-sync-restore-deletions")),
                        )
                        .group(
                            ArgGroup::new("deletions")
                                .args(["confirm-deletions", "restore-deletions"]),
                        ),
                )
                .subcommand(
                    clap::Command::new("folders")
                        .about(i18n.tr("cli-sync-folders-about"))
                        .arg(email("cli-account-email"))
                        .arg(
                            folder()
                                .required(false)
                                .help(i18n.tr("cli-sync-folders-folder")),
                        ),
                )
                .subcommand(
                    clap::Command::new("select")
                        .about(i18n.tr("cli-sync-select-about"))
                        .arg(email("cli-account-email"))
                        .arg(folder()),
                )
                .subcommand(
                    clap::Command::new("deselect")
                        .about(i18n.tr("cli-sync-deselect-about"))
                        .arg(email("cli-account-email"))
                        .arg(folder())
                        .arg(
                            Arg::new("keep-local")
                                .long("keep-local")
                                .action(ArgAction::SetTrue)
                                .help(i18n.tr("cli-sync-keep-local")),
                        )
                        .arg(
                            Arg::new("remove-local")
                                .long("remove-local")
                                .action(ArgAction::SetTrue)
                                .help(i18n.tr("cli-sync-remove-local")),
                        )
                        // The user must choose explicitly what happens to downloaded files.
                        .group(
                            ArgGroup::new("local-copies")
                                .args(["keep-local", "remove-local"])
                                .required(true),
                        ),
                ),
        )
        .subcommand(
            clap::Command::new("stream")
                .about(i18n.tr("cli-stream-about"))
                .arg(email("cli-account-email"))
                .arg(
                    Arg::new("mountpoint")
                        .value_name("FOLDER")
                        .required(true)
                        .value_parser(value_parser!(PathBuf))
                        .help(i18n.tr("cli-stream-mountpoint")),
                ),
        )
        .subcommand({
            let path = || {
                Arg::new("path")
                    .value_name("PATH")
                    .required(true)
                    .value_parser(value_parser!(PathBuf))
                    .help(i18n.tr("cli-offline-path"))
            };
            group("offline", "cli-offline-about")
                .subcommand(
                    clap::Command::new("add")
                        .about(i18n.tr("cli-offline-add-about"))
                        .arg(path()),
                )
                .subcommand(
                    clap::Command::new("remove")
                        .about(i18n.tr("cli-offline-remove-about"))
                        .arg(path()),
                )
                .subcommand(clap::Command::new("list").about(i18n.tr("cli-offline-list-about")))
        })
        .subcommand({
            let invalid = i18n.tr("cli-cache-invalid-size");
            group("cache", "cli-cache-about")
                .subcommand(clap::Command::new("show").about(i18n.tr("cli-cache-show-about")))
                .subcommand(clap::Command::new("clear").about(i18n.tr("cli-cache-clear-about")))
                .subcommand(
                    clap::Command::new("limit")
                        .about(i18n.tr("cli-cache-limit-about"))
                        .arg(
                            Arg::new("size")
                                .value_name("SIZE")
                                .required(true)
                                .value_parser(move |s: &str| parse_size(s).ok_or(invalid.clone()))
                                .help(i18n.tr("cli-cache-size")),
                        ),
                )
        })
        .subcommand(
            group("trash", "cli-trash-about")
                .subcommand(
                    clap::Command::new("list")
                        .about(i18n.tr("cli-trash-list-about"))
                        .arg(email("cli-account-email")),
                )
                .subcommand(
                    clap::Command::new("restore")
                        .about(i18n.tr("cli-trash-restore-about"))
                        .arg(email("cli-account-email"))
                        .arg(
                            Arg::new("id")
                                .value_name("ITEM_ID")
                                .required(true)
                                .help(i18n.tr("cli-trash-item-id")),
                        ),
                ),
        );
    localize_headings(root, i18n)
}

/// Replaces clap's English section headings, recursively.
fn localize_headings(command: clap::Command, i18n: &Localizer) -> clap::Command {
    let template = format!(
        "{{about-with-newline}}\n{} {{usage}}\n\n{{all-args}}",
        i18n.tr("cli-heading-usage")
    );
    let subcommands: Vec<_> = command.get_subcommands().cloned().collect();
    let mut command = command
        .help_template(template)
        .subcommand_help_heading(i18n.tr("cli-heading-commands"))
        .mut_args(|arg| {
            let heading = if arg.is_positional() {
                "cli-heading-arguments"
            } else {
                "cli-heading-options"
            };
            arg.help_heading(i18n.tr(heading))
        });
    for sub in subcommands {
        let name = sub.get_name().to_owned();
        let localized = localize_headings(sub, i18n);
        command = command.mut_subcommand(name, |_| localized);
    }
    command
}

/// A size such as `5G`, `500MB` or `1.5GiB` (binary units), or plain bytes.
fn parse_size(text: &str) -> Option<u64> {
    let text = text.trim().to_ascii_uppercase();
    let split = text
        .find(|c: char| !c.is_ascii_digit() && c != '.' && c != ',')
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let number: f64 = number.replace(',', ".").parse().ok()?;
    let shift = match unit.trim().trim_end_matches("IB").trim_end_matches('B') {
        "" => 0,
        "K" => 10,
        "M" => 20,
        "G" => 30,
        "T" => 40,
        _ => return None,
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    // Checked finite and positive.
    (number.is_finite() && number > 0.0).then(|| (number * (1u64 << shift) as f64) as u64)
}

fn string(matches: &ArgMatches, id: &str) -> String {
    matches.get_one::<String>(id).cloned().unwrap_or_default()
}

fn no_subcommand() -> ! {
    unreachable!("clap enforces a subcommand")
}

fn from_matches(matches: &ArgMatches) -> Command {
    match matches.subcommand() {
        Some(("daemon", _)) => Command::Daemon,
        Some(("status", _)) => Command::Status,
        Some(("account", m)) => Command::Account(match m.subcommand() {
            Some(("list", _)) => AccountCommand::List,
            Some(("add", _)) => AccountCommand::Add,
            Some(("status", m)) => AccountCommand::Status {
                email: m.get_one::<String>("email").cloned(),
            },
            Some(("remove", m)) => AccountCommand::Remove {
                email: string(m, "email"),
            },
            _ => no_subcommand(),
        }),
        Some(("config", m)) => Command::Config(match m.subcommand() {
            Some(("show", _)) => ConfigCommand::Show,
            Some(("sync-root", m)) => ConfigCommand::SyncRoot {
                path: m.get_one::<PathBuf>("path").cloned().unwrap_or_default(),
            },
            Some(("mode", m)) => ConfigCommand::Mode {
                email: string(m, "email"),
                stream: string(m, "mode") == "stream",
            },
            _ => no_subcommand(),
        }),
        Some(("sync", m)) => Command::Sync(match m.subcommand() {
            Some(("run", m)) => SyncCommand::Run {
                email: m.get_one::<String>("email").cloned(),
                deletions: if m.get_flag("confirm-deletions") {
                    Deletions::Trash
                } else if m.get_flag("restore-deletions") {
                    Deletions::Restore
                } else {
                    Deletions::Ask
                },
            },
            Some(("folders", m)) => SyncCommand::Folders {
                email: string(m, "email"),
                folder: m.get_one::<String>("folder").cloned(),
            },
            Some(("select", m)) => SyncCommand::Select {
                email: string(m, "email"),
                folder: string(m, "folder"),
            },
            Some(("deselect", m)) => SyncCommand::Deselect {
                email: string(m, "email"),
                folder: string(m, "folder"),
                remove_local: m.get_flag("remove-local"),
            },
            _ => no_subcommand(),
        }),
        Some(("stream", m)) => Command::Stream {
            email: string(m, "email"),
            mountpoint: m
                .get_one::<PathBuf>("mountpoint")
                .cloned()
                .unwrap_or_default(),
        },
        Some(("offline", m)) => Command::Offline(match m.subcommand() {
            Some(("add", m)) => OfflineCommand::Add {
                path: m.get_one::<PathBuf>("path").cloned().unwrap_or_default(),
            },
            Some(("remove", m)) => OfflineCommand::Remove {
                path: m.get_one::<PathBuf>("path").cloned().unwrap_or_default(),
            },
            Some(("list", _)) => OfflineCommand::List,
            _ => no_subcommand(),
        }),
        Some(("cache", m)) => Command::Cache(match m.subcommand() {
            Some(("show", _)) => CacheCommand::Show,
            Some(("clear", _)) => CacheCommand::Clear,
            Some(("limit", m)) => CacheCommand::Limit {
                bytes: m.get_one::<u64>("size").copied().unwrap_or_default(),
            },
            _ => no_subcommand(),
        }),
        Some(("trash", m)) => Command::Trash(match m.subcommand() {
            Some(("list", m)) => TrashCommand::List {
                email: string(m, "email"),
            },
            Some(("restore", m)) => TrashCommand::Restore {
                email: string(m, "email"),
                id: string(m, "id"),
            },
            _ => no_subcommand(),
        }),
        _ => no_subcommand(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Command {
        let matches = command(&Localizer::new("en-US"))
            .try_get_matches_from(std::iter::once("nextgdrive").chain(args.iter().copied()))
            .unwrap();
        from_matches(&matches)
    }

    #[test]
    fn deselect_requires_choosing_what_happens_to_local_copies() {
        let parse = |args: &[&str]| {
            command(&Localizer::new("en-US"))
                .try_get_matches_from(std::iter::once("nextgdrive").chain(args.iter().copied()))
                .is_ok()
        };
        assert!(!parse(&["sync", "deselect", "ana@x.com", "abc"]));
        assert!(!parse(&[
            "sync",
            "deselect",
            "ana@x.com",
            "abc",
            "--keep-local",
            "--remove-local"
        ]));
        assert!(parse(&[
            "sync",
            "deselect",
            "ana@x.com",
            "abc",
            "--keep-local"
        ]));
    }

    #[test]
    fn cache_sizes_take_binary_units() {
        assert_eq!(parse_size("5G"), Some(5 << 30));
        assert_eq!(parse_size("500mb"), Some(500 << 20));
        assert_eq!(parse_size("1.5GiB"), Some(3 << 29));
        assert_eq!(parse_size("1,5 G"), Some(3 << 29));
        assert_eq!(parse_size("4096"), Some(4096));
        assert_eq!(parse_size("5X"), None);
        assert_eq!(parse_size("0"), None);
        assert!(matches!(
            parse_args(&["cache", "limit", "2G"]),
            Command::Cache(CacheCommand::Limit { bytes }) if bytes == 2 << 30
        ));
    }

    #[test]
    fn command_definition_is_valid() {
        command(&Localizer::new("en-US")).debug_assert();
    }

    #[test]
    fn parses_subcommands() {
        assert!(matches!(
            parse_args(&["account", "remove", "ana@x.com"]),
            Command::Account(AccountCommand::Remove { email }) if email == "ana@x.com"
        ));
        assert!(matches!(
            parse_args(&["sync", "folders", "ana@x.com"]),
            Command::Sync(SyncCommand::Folders { folder: None, .. })
        ));
        assert!(matches!(
            parse_args(&["sync", "select", "ana@x.com", "abc"]),
            Command::Sync(SyncCommand::Select { folder, .. }) if folder == "abc"
        ));
        assert!(matches!(
            parse_args(&["sync", "deselect", "ana@x.com", "abc", "--remove-local"]),
            Command::Sync(SyncCommand::Deselect {
                remove_local: true,
                ..
            })
        ));
        assert!(matches!(
            parse_args(&["trash", "restore", "ana@x.com", "abc"]),
            Command::Trash(TrashCommand::Restore { id, .. }) if id == "abc"
        ));
        assert!(matches!(
            parse_args(&["config", "sync-root", "/data"]),
            Command::Config(ConfigCommand::SyncRoot { path }) if path.as_os_str() == "/data"
        ));
    }
}
