//! `nextgdrive` command-line interface. Composition root for the CLI: wires
//! the adapters into the use cases and renders results with translations.

mod cli;

use std::process::{ExitCode, Stdio};

use nextgdrive_application::ports::PendingAuthorization;
use nextgdrive_application::use_cases::{
    AddAccount, GetAccountStatus, ListAccounts, RemoveAccount,
};
use nextgdrive_composition as services;
use nextgdrive_domain::Email;
use nextgdrive_i18n::Localizer;

use cli::{AccountCommand, Command};

fn main() -> ExitCode {
    let i18n = Localizer::from_system();
    let command = cli::parse(&i18n);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to start the async runtime");
    match runtime.block_on(run(command, &i18n)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            if !message.is_empty() {
                eprintln!("{message}");
            }
            ExitCode::FAILURE
        }
    }
}

/// Returns the localized error message on failure (empty if already reported).
async fn run(command: Command, i18n: &Localizer) -> Result<(), String> {
    let accounts = services::accounts(i18n)?;
    match command {
        Command::Account(AccountCommand::List) => {
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
        Command::Account(AccountCommand::Add) => {
            let (auth, drive) = services::google(i18n)?;
            let credentials = services::credentials(i18n).await?;
            let add = AddAccount::new(&auth, &drive, &credentials, &accounts);
            let pending = add.start().await.map_err(|e| i18n.app_error(&e))?;
            println!(
                "{}\n\n  {}\n",
                i18n.tr("cli-account-add-open-url"),
                pending.url()
            );
            open_in_browser(pending.url());
            println!("{}", i18n.tr("cli-account-add-waiting"));
            let info = add.finish(pending).await.map_err(|e| i18n.app_error(&e))?;
            println!(
                "{}",
                i18n.tr_args(
                    "account-added",
                    [("email", info.account.email.to_string().into())]
                )
            );
            println!("{}", i18n.quota(&info.quota));
        }
        Command::Account(AccountCommand::Status { email }) => {
            let (auth, drive) = services::google(i18n)?;
            let credentials = services::credentials(i18n).await?;
            let mut list = ListAccounts::new(&accounts)
                .execute()
                .map_err(|e| i18n.app_error(&e))?;
            if let Some(email) = email {
                let email = Email::parse(&email).map_err(|e| i18n.domain_error(&e))?;
                list.retain(|a| a.email == email);
                if list.is_empty() {
                    let error = nextgdrive_application::AppError::AccountNotFound(email);
                    return Err(i18n.app_error(&error));
                }
            } else if list.is_empty() {
                println!("{}", i18n.tr("cli-account-list-empty"));
            }
            let status = GetAccountStatus::new(&auth, &drive, &credentials, &accounts);
            let mut failed = false;
            for account in list {
                match status.execute(&account).await {
                    Ok(info) => println!("{}: {}", info.account.email, i18n.quota(&info.quota)),
                    Err(e) => {
                        failed = true;
                        eprintln!("{}: {}", account.email, i18n.app_error(&e));
                    }
                }
            }
            if failed {
                // Each failure was already reported above.
                return Err(String::new());
            }
        }
        Command::Account(AccountCommand::Remove { email }) => {
            let email = Email::parse(&email).map_err(|e| i18n.domain_error(&e))?;
            let credentials = services::credentials(i18n).await?;
            let removed = RemoveAccount::new(&credentials, &accounts)
                .execute(&email)
                .await
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

/// Best effort: the URL is also printed, for headless sessions.
fn open_in_browser(url: &str) {
    let _ = std::process::Command::new("xdg-open")
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}
