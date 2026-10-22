//! `nextgdrive` command-line interface. Composition root for the CLI: wires
//! the adapters into the use cases and renders results with translations.

mod cli;

use std::process::ExitCode;

use nextgdrive_adapter_config::TomlAccountRepository;
use nextgdrive_application::use_cases::{ListAccounts, RemoveAccount};
use nextgdrive_domain::Email;
use nextgdrive_i18n::Localizer;

use cli::Command;

fn main() -> ExitCode {
    let i18n = Localizer::from_system();
    let command = cli::parse(&i18n);
    match run(command, &i18n) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

/// Returns the localized error message on failure.
fn run(command: Command, i18n: &Localizer) -> Result<(), String> {
    let accounts =
        TomlAccountRepository::in_config_dir().ok_or_else(|| i18n.tr("error-no-config-dir"))?;
    match command {
        Command::AccountList => {
            let list = ListAccounts::new(&accounts)
                .execute()
                .map_err(|e| i18n.app_error(&e))?;
            if list.is_empty() {
                println!("{}", i18n.tr("cli-account-list-empty"));
            }
            for account in list {
                match account.display_name {
                    Some(name) => println!("{}  ({name})", account.email),
                    None => println!("{}", account.email),
                }
            }
        }
        Command::AccountRemove { email } => {
            let email = Email::parse(&email).map_err(|e| i18n.domain_error(&e))?;
            let removed = RemoveAccount::new(&accounts)
                .execute(&email)
                .map_err(|e| i18n.app_error(&e))?;
            println!(
                "{}",
                i18n.tr_args(
                    "account-removed",
                    [("email", removed.email.to_string().into())]
                )
            );
        }
    }
    Ok(())
}
