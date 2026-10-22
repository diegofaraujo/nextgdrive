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

# Folders created inside each account folder. Chosen once, on first use.
folder-my-drive = My Drive
folder-shared-drives = Shared drives
sync-progress = { $done } of { $total } files, { $bytes_done } of { $bytes_total }
sync-summary = { $email }: { $downloaded } downloaded ({ $bytes }), { $moved } moved, { $removed } removed
sync-conflict = Kept the local version, which changed on this computer: { $path }
sync-skipped-shortcut = Skipped Drive shortcut: { $path }
sync-skipped-locked = Skipped, the owner does not allow downloading: { $path }
sync-failed = Failed: { $path }: { $error }

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
cli-config-about = Show or change settings
cli-config-show-about = Show current settings
cli-config-sync-root-about = Change the folder that holds synced accounts
cli-config-sync-root-path = New sync folder
cli-sync-about = Choose what to sync
cli-sync-folders-about = List drives, or subfolders of a folder, marking selected ones with [x]
cli-sync-folders-folder = Folder id (default: list the account's drives)
cli-sync-select-about = Sync a folder and everything inside it
cli-sync-deselect-about = Stop syncing a folder
cli-sync-folder-id = Folder or drive id, as shown by "sync folders"
cli-account-email = Account email
cli-settings-sync-root = Sync folder: { $path }
cli-settings-selection = { $email }: { $count ->
    [0] nothing selected
    [one] 1 folder selected
   *[other] { $count } folders selected
}
cli-sync-root-changed = Sync folder changed to { $path }. Files already synced were not moved.
cli-folder-selected = Folder selected. It will be downloaded on the next sync.
cli-folder-deselected = Folder deselected. Local copies were kept and will no longer be updated.
cli-no-folders = No subfolders.
cli-sync-run-about = Download the selected folders of one or all accounts
cli-sync-run-email = Only this account (default: all)
cli-sync-nothing-selected = { $email }: nothing selected. Choose folders with "nextgdrive sync select".

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
gui-main-menu = Main Menu
gui-preferences = Preferences
gui-storage = Storage
gui-sync-group = Sync
gui-folders = Folders to Sync
gui-folders-count = { $count ->
    [0] Nothing selected
    [one] 1 folder selected
   *[other] { $count } folders selected
}
gui-folders-description = Everything inside a selected folder is downloaded to this computer.
gui-folders-inherited = Synced because a folder above it is selected.
gui-no-subfolders = No Subfolders
gui-sync-title = Download
gui-sync-now = Sync Now
gui-sync-never = Not synced yet in this session
gui-syncing = Syncing…
gui-sync-result = { $downloaded } downloaded ({ $bytes }), { $moved } moved, { $removed } removed
gui-report-conflicts = { $count ->
    [one] 1 file kept with changes made on this computer
   *[other] { $count } files kept with changes made on this computer
}
gui-report-failures = { $count ->
    [one] 1 item failed
   *[other] { $count } items failed
}
gui-report-skipped = { $count ->
    [one] 1 item skipped
   *[other] { $count } items skipped
}
gui-skipped-shortcut = Drive shortcut
gui-skipped-locked = The owner does not allow downloading
gui-open-folder = Open Folder
gui-sync-root = Sync Folder
gui-sync-root-change = Change…
gui-sync-root-note = Each account gets its own folder inside it. Files already synced stay where they are; the next sync downloads into the new folder.

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
error-invalid-sync-root = The sync folder must be an absolute path: { $path }
error-not-a-folder = Not a folder: { $id }
