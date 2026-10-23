//! `nextgdrive` command-line interface: parses the command, runs the use
//! case, and prints the result with translations.

mod cli;

use std::io::{IsTerminal, Write};
use std::process::{ExitCode, Stdio};
use std::sync::Arc;

use nextgdrive_adapter_dbus::Client;
use nextgdrive_application::AppError;
use nextgdrive_application::ports::{AccountRepository, PendingAuthorization};
use nextgdrive_application::use_cases::{
    Deletions, LocalCopies, SkipReason, SyncOptions, SyncProgress, SyncReport,
};
use nextgdrive_composition::host::{self, StartError, SyncHost};
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
        Command::Daemon => daemon(i18n).await?,
        Command::Status => match Client::connect().await {
            None => println!("{}", i18n.tr("cli-status-not-running")),
            Some(client) => {
                let status = client
                    .status()
                    .await
                    .map_err(|e| i18n.tr_args("error-dbus", [("details", e.to_string().into())]))?;
                println!("{}", host::describe(i18n, &status));
                for (email, message) in status.errors.iter().skip(1) {
                    println!("{email}: {message}");
                }
            }
        },
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
        Command::Sync(SyncCommand::Run { email, deletions }) => {
            // Let the process running in the background do it, if there is one.
            if email.is_none()
                && deletions == Deletions::Ask
                && let Some(client) = Client::connect().await
            {
                client
                    .sync_now()
                    .await
                    .map_err(|e| i18n.tr_args("error-dbus", [("details", e.to_string().into())]))?;
                println!("{}", i18n.tr("cli-sync-requested"));
                return Ok(());
            }
            let services = Services::new(i18n).await?;
            let accounts = match email {
                Some(email) => vec![find_account(&services.local, &email, i18n)?],
                None => services
                    .local
                    .list_accounts()
                    .execute()
                    .map_err(app_error)?,
            };
            if accounts.is_empty() {
                println!("{}", i18n.tr("cli-account-list-empty"));
            }
            let settings = services.local.get_settings().execute().map_err(app_error)?;
            let mut failed = false;
            for account in accounts {
                if settings.selection(&account.id).next().is_none() {
                    println!(
                        "{}",
                        i18n.tr_args(
                            "cli-sync-nothing-selected",
                            [("email", account.email.to_string().into())]
                        )
                    );
                    continue;
                }
                let show_progress = std::io::stderr().is_terminal();
                let on_progress = |p: &SyncProgress| {
                    if show_progress && p.files_total > 0 {
                        eprint!("\r\x1b[2K{}", progress_line(i18n, p));
                        let _ = std::io::stderr().flush();
                    }
                };
                let options = SyncOptions {
                    conflict_label: nextgdrive_composition::conflict_label(i18n),
                    deletions,
                };
                let result = services
                    .sync_account()
                    .execute(&account, &options, &on_progress)
                    .await;
                if show_progress {
                    eprint!("\r\x1b[2K");
                }
                match result {
                    Ok(report) => failed |= print_report(i18n, &account, &report),
                    Err(e) => {
                        failed = true;
                        eprintln!("{}: {}", account.email, i18n.app_error(&e));
                    }
                }
            }
            if failed {
                return Err(String::new());
            }
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
        Command::Sync(SyncCommand::Deselect {
            email,
            folder,
            remove_local,
        }) => {
            let services = Services::new(i18n).await?;
            let account = find_account(&services.local, &email, i18n)?;
            let copies = if remove_local {
                LocalCopies::Remove
            } else {
                LocalCopies::Keep
            };
            let report = services
                .change_selection()
                .deselect(&account, &DriveItemId::new(folder), copies)
                .map_err(app_error)?;
            let path = |p: &std::path::Path| FluentValue::from(p.display().to_string());
            for kept in &report.kept {
                eprintln!("{}", i18n.tr_args("sync-conflict", [("path", path(kept))]));
            }
            for (failed, error) in &report.failures {
                eprintln!(
                    "{}",
                    i18n.tr_args(
                        "sync-failed",
                        [
                            ("path", path(failed)),
                            ("error", i18n.app_error(error).into())
                        ]
                    )
                );
            }
            match copies {
                LocalCopies::Keep => println!("{}", i18n.tr("cli-folder-deselected")),
                LocalCopies::Remove => println!(
                    "{}",
                    i18n.tr_args(
                        "cli-folder-deselected-removed",
                        [("removed", FluentValue::from(report.removed))]
                    )
                ),
            }
            if !report.failures.is_empty() {
                return Err(String::new());
            }
        }
    }
    Ok(())
}

/// Runs background sync in the foreground until interrupted.
async fn daemon(i18n: &Localizer) -> Result<(), String> {
    use tokio::signal::unix::{SignalKind, signal};

    let services = Arc::new(Services::new(i18n).await?);
    let interval = services
        .local
        .get_settings()
        .execute()
        .map_err(|e| i18n.app_error(&e))?
        .poll_interval_secs;
    let log = Box::new(|line: String| println!("{line}"));
    let host = match SyncHost::start(services, i18n.clone(), log).await {
        Ok(host) => host,
        Err(StartError::AlreadyRunning) => return Err(i18n.tr("cli-daemon-already-running")),
        Err(StartError::Bus(details)) => {
            return Err(i18n.tr_args("error-dbus", [("details", details.into())]));
        }
    };
    println!(
        "{}",
        i18n.tr_args(
            "cli-daemon-started",
            [("secs", FluentValue::from(interval))]
        )
    );
    let mut terminate = signal(SignalKind::terminate()).map_err(|e| e.to_string())?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate.recv() => {}
    }
    drop(host);
    println!("{}", i18n.tr("cli-daemon-stopped"));
    Ok(())
}

fn progress_line(i18n: &Localizer, p: &SyncProgress) -> String {
    i18n.tr_args(
        "sync-progress",
        [
            ("done", FluentValue::from(p.files_done)),
            ("total", FluentValue::from(p.files_total)),
            ("bytes_done", i18n.bytes(p.bytes_done).into()),
            ("bytes_total", i18n.bytes(p.bytes_total).into()),
        ],
    )
}

/// Prints the outcome of an account's sync. Returns whether anything failed.
fn print_report(i18n: &Localizer, account: &Account, report: &SyncReport) -> bool {
    let path = |p: &std::path::Path| FluentValue::from(p.display().to_string());
    for copy in &report.conflict_copies {
        eprintln!(
            "{}",
            i18n.tr_args("sync-conflict-copy", [("path", path(copy))])
        );
    }
    for conflict in &report.conflicts {
        eprintln!("{}", i18n.conflict_line(conflict));
    }
    for (skipped, reason) in &report.skipped {
        let id = match reason {
            SkipReason::Shortcut => "sync-skipped-shortcut",
            SkipReason::NotDownloadable => "sync-skipped-locked",
        };
        println!("{}", i18n.tr_args(id, [("path", path(skipped))]));
    }
    for (failed, error) in &report.failures {
        eprintln!(
            "{}",
            i18n.tr_args(
                "sync-failed",
                [
                    ("path", path(failed)),
                    ("error", i18n.app_error(error).into())
                ]
            )
        );
    }
    println!("{}: {}", account.email, i18n.sync_summary(report));
    !report.failures.is_empty()
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
