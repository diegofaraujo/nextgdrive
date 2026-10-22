app-name = Next GDrive

## Shared

# Used to format sizes such as "1.5 GB".
decimal-separator = .
account-removed = Account { $email } unlinked. Its local files were kept.
account-added = Account { $email } linked.
quota-used = { $used } of { $total } used ({ $free } free)
quota-used-unlimited = { $used } used
auth-callback-success = You are signed in. You can close this tab and return to Next GDrive.
auth-callback-failure = Sign-in was not completed. Return to Next GDrive and try again.

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
cli-account-add-about = Link a Google account (opens your browser to sign in)
cli-account-status-about = Show storage usage of linked accounts
cli-account-status-email = Only this account (default: all)
cli-account-add-open-url = Open this address in your browser to sign in:
cli-account-add-waiting = Waiting for you to sign in…
cli-account-list-empty = No accounts linked.

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
gui-add-account = Add Account
gui-add-waiting-title = Sign In With Google
gui-add-waiting-body = Continue in your browser. If it did not open, use the link below.
gui-open-browser = Open sign-in page
gui-quota-loading = Checking storage…
gui-quota-error = Could not check storage

## Errors

error-account-not-found = No account with email { $email }.
error-invalid-email = Invalid email address: { $email }
error-storage = Could not access stored data: { $details }
error-no-config-dir = Could not determine the configuration directory (is $HOME set?).
error-missing-credentials = No stored credentials for { $email }. Add the account again.
error-auth = Sign-in failed: { $details }
error-credentials = Could not access the system keyring: { $details }
error-drive = Google Drive request failed: { $details }
error-oauth-client-missing = The Google OAuth client is not configured. Save the JSON file downloaded from Google Cloud Console as { $path }, or set { $id_var } and { $secret_var }.
error-oauth-client-unreadable = Could not read the OAuth client file { $path }: { $details }
