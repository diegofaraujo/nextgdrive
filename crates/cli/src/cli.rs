//! Command-line definition. Built with clap's builder API rather than derive,
//! because help texts must come from the translation files at runtime.

use clap::{Arg, ArgAction, ArgMatches};
use nextgdrive_i18n::Localizer;

pub enum Command {
    AccountList,
    AccountRemove { email: String },
}

pub fn parse(i18n: &Localizer) -> Command {
    from_matches(&command(i18n).get_matches())
}

fn command(i18n: &Localizer) -> clap::Command {
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
        .subcommand(
            clap::Command::new("account")
                .about(i18n.tr("cli-account-about"))
                .subcommand_required(true)
                .arg_required_else_help(true)
                .subcommand(clap::Command::new("list").about(i18n.tr("cli-account-list-about")))
                .subcommand(
                    clap::Command::new("remove")
                        .about(i18n.tr("cli-account-remove-about"))
                        .arg(
                            Arg::new("email")
                                .value_name("EMAIL")
                                .required(true)
                                .help(i18n.tr("cli-account-remove-email")),
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

fn from_matches(matches: &ArgMatches) -> Command {
    match matches.subcommand() {
        Some(("account", account)) => match account.subcommand() {
            Some(("list", _)) => Command::AccountList,
            Some(("remove", remove)) => Command::AccountRemove {
                email: remove
                    .get_one::<String>("email")
                    .cloned()
                    .unwrap_or_default(),
            },
            _ => unreachable!("clap enforces a subcommand"),
        },
        _ => unreachable!("clap enforces a subcommand"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_definition_is_valid() {
        command(&Localizer::new("en-US")).debug_assert();
    }

    #[test]
    fn parses_account_remove() {
        let matches = command(&Localizer::new("en-US"))
            .try_get_matches_from(["nextgdrive", "account", "remove", "ana@x.com"])
            .unwrap();
        assert!(
            matches!(from_matches(&matches), Command::AccountRemove { email } if email == "ana@x.com")
        );
    }
}
