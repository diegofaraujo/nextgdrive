app-name = Next GDrive

## CLI

cli-about = Sync your Google Drive accounts with local folders.
cli-heading-usage = Usage:
cli-heading-commands = Commands
cli-heading-arguments = Arguments
cli-heading-options = Options
cli-help = Print help
cli-version = Print version
cli-account-about = Manage linked Google accounts
cli-account-list-about = List linked accounts
cli-account-remove-about = Unlink an account (local files are kept)
cli-account-remove-email = Email of the account to unlink
cli-account-list-empty = No accounts linked.
account-removed = Account { $email } unlinked. Its local files were kept.

## GUI

gui-accounts-title = Accounts
gui-no-accounts-title = No Accounts
gui-no-accounts-description = Linked Google accounts will appear here.
gui-load-error-title = Could Not Load Accounts
gui-remove-account = Unlink account
gui-remove-confirm-title = Unlink { $email }?
gui-remove-confirm-body = The account will stop syncing. Files already on this computer are kept.
gui-cancel = Cancel
gui-remove = Unlink

## Errors

error-account-not-found = No account with email { $email }.
error-invalid-email = Invalid email address: { $email }
error-storage = Could not access stored data: { $details }
error-no-config-dir = Could not determine the configuration directory (is $HOME set?).
