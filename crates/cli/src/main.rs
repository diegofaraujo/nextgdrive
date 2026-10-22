//! `nextgdrive` command-line interface: parses the command, runs the use
//! case, and prints the result with translations.

mod cli;

use std::process::{ExitCode, Stdio};

use nextgdrive_application::AppError;
use nextgdrive_application::ports::{AccountRepository, PendingAuthorization};
use nextgdrive_composition::{LocalServices, Services};
use nextgdrive_domain::{Account, DriveItemId, DriveKind, Email};
use nextgdrive_i18n::{FluentValue, Localizer};

use cli::{AccountCommand, Command, ConfigCommand, SyncCommand};

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
    let app_error = |e: AppError| i18n.app_error(&e);
    match command {
        Command::Account(AccountCommand::List) => {
            let local = LocalServices::new(i18n)?;
            let list = local.list_accounts().execute().map_err(app_error)?;
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
            let services = Services::new(i18n).await?;
            let add = services.add_account();
            let pending = add.start().await.map_err(app_error)?;
            println!(
                "{}\n\n  {}\n",
                i18n.tr("cli-account-add-open-url"),
                pending.url()
            );
            open_in_browser(pending.url());
            println!("{}", i18n.tr("cli-account-add-waiting"));
            let info = add.finish(pending).await.map_err(app_error)?;
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
            let services = Services::new(i18n).await?;
            let list = match email {
                Some(email) => vec![find_account(&services.local, &email, i18n)?],
                None => services
                    .local
                    .list_accounts()
                    .execute()
                    .map_err(app_error)?,
            };
            if list.is_empty() {
                println!("{}", i18n.tr("cli-account-list-empty"));
            }
            let status = services.account_status();
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
            let services = Services::new(i18n).await?;
            let removed = services
                .remove_account()
                .execute(&email)
                .await
                .map_err(app_error)?;
            println!(
                "{}",
                i18n.tr_args(
                    "account-removed",
                    [("email", removed.email.to_string().into())]
                )
            );
        }
        Command::Config(ConfigCommand::Show) => {
            let local = LocalServices::new(i18n)?;
            let settings = local.get_settings().execute().map_err(app_error)?;
            println!(
                "{}",
                i18n.tr_args(
                    "cli-settings-sync-root",
                    [("path", settings.sync_root.display().to_string().into())]
                )
            );
            for account in local.list_accounts().execute().map_err(app_error)? {
                let count = settings.selection(&account.id).count();
                println!(
                    "{}",
                    i18n.tr_args(
                        "cli-settings-selection",
                        [
                            ("email", account.email.to_string().into()),
                            ("count", FluentValue::from(count)),
                        ]
                    )
                );
            }
        }
        Command::Config(ConfigCommand::SyncRoot { path }) => {
            // Relative paths are a convenience of the command line: resolve them here.
            let path = std::path::absolute(&path).unwrap_or(path);
            let local = LocalServices::new(i18n)?;
            local
                .set_sync_root()
                .execute(path.clone())
                .map_err(app_error)?;
            println!(
                "{}",
                i18n.tr_args(
                    "cli-sync-root-changed",
                    [("path", path.display().to_string().into())]
                )
            );
        }
        Command::Sync(SyncCommand::Folders { email, folder }) => {
            let services = Services::new(i18n).await?;
            let account = find_account(&services.local, &email, i18n)?;
            let settings = services.local.get_settings().execute().map_err(app_error)?;
            let browse = services.browse_drive();
            let lines: Vec<(bool, String, DriveItemId)> = match folder {
                None => browse
                    .drives(&account)
                    .await
                    .map_err(app_error)?
                    .into_iter()
                    .map(|e| {
                        let name = match e.item.kind {
                            DriveKind::MyDrive => settings.my_drive_folder.clone(),
                            DriveKind::Shared { name } => {
                                format!("{}/{name}", settings.shared_drives_folder)
                            }
                        };
                        (e.selected, name, e.item.root)
                    })
                    .collect(),
                Some(folder) => browse
                    .folders(&account, &DriveItemId::new(folder))
                    .await
                    .map_err(app_error)?
                    .into_iter()
                    .map(|e| (e.selected, e.item.name, e.item.id))
                    .collect(),
            };
            if lines.is_empty() {
                println!("{}", i18n.tr("cli-no-folders"));
            }
            for (selected, name, id) in lines {
                println!("[{}] {name}  ({id})", if selected { 'x' } else { ' ' });
            }
        }
        Command::Sync(SyncCommand::Select { email, folder }) => {
            let services = Services::new(i18n).await?;
            let account = find_account(&services.local, &email, i18n)?;
            services
                .change_selection()
                .select(&account, &DriveItemId::new(folder))
                .await
                .map_err(app_error)?;
            println!("{}", i18n.tr("cli-folder-selected"));
        }
        Command::Sync(SyncCommand::Deselect { email, folder }) => {
            let services = Services::new(i18n).await?;
            let account = find_account(&services.local, &email, i18n)?;
            services
                .change_selection()
                .deselect(&account, &DriveItemId::new(folder))
                .map_err(app_error)?;
            println!("{}", i18n.tr("cli-folder-deselected"));
        }
    }
    Ok(())
}

fn find_account(local: &LocalServices, email: &str, i18n: &Localizer) -> Result<Account, String> {
    let email = Email::parse(email).map_err(|e| i18n.domain_error(&e))?;
    local
        .accounts
        .find_by_email(&email)
        .map_err(|e| i18n.app_error(&AppError::Storage(e)))?
        .ok_or_else(|| i18n.app_error(&AppError::AccountNotFound(email)))
}

/// Best effort: the URL is also printed, for headless sessions.
fn open_in_browser(url: &str) {
    let _ = std::process::Command::new("xdg-open")
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}
