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
sync-part-downloaded = { $count ->
    [one] 1 downloaded ({ $bytes })
   *[other] { $count } downloaded ({ $bytes })
}
sync-part-uploaded = { $count ->
    [one] 1 uploaded ({ $bytes })
   *[other] { $count } uploaded ({ $bytes })
}
sync-part-moved = { $count ->
    [one] 1 moved
   *[other] { $count } moved
}
sync-part-removed = { $count ->
    [one] 1 removed from this computer
   *[other] { $count } removed from this computer
}
sync-part-trashed = { $count ->
    [one] 1 moved to the Drive trash
   *[other] { $count } moved to the Drive trash
}
sync-part-copies = { $count ->
    [one] 1 conflict copy
   *[other] { $count } conflict copies
}
sync-part-nothing = nothing changed
sync-conflict-copy = Changed here and in Google Drive; your version was kept as { $path }
conflict-type-clash = Left as is: a file and a folder have the same name
conflict-read-only = Your changes stay on this computer: you cannot edit this file in Google Drive
conflict-cannot-delete = Restored: you cannot delete this item in Google Drive
sync-started = { $email }: syncing…
# Inserted in file names, e.g. "report (conflict copy 2026-10-23 14-30).pdf".
conflict-copy-label = conflict copy { $date }
sync-conflict = Kept the local version, which changed on this computer: { $path }
sync-skipped-shortcut = Skipped Drive shortcut: { $path }
sync-skipped-locked = Skipped, the owner does not allow downloading: { $path }
sync-failed = Failed: { $path }: { $error }

status-up-to-date = Up to date
status-checked-ago = { $minutes ->
    [0] checked just now
    [one] checked 1 minute ago
   *[other] checked { $minutes } minutes ago
}
status-starting = Starting…
status-checking = Checking { $email }…
status-syncing = Syncing { $email }: { $done } of { $total } files
status-paused = Sync paused
status-problem = Problem with { $email }: { $message }

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
cli-sync-root-changed = Sync folder changed to { $path }. Synced files were moved there.
cli-folder-selected = Folder selected. It will be downloaded on the next sync.
cli-folder-deselected = Folder deselected. Local copies were kept and will no longer be updated.
cli-no-folders = No subfolders.
cli-sync-run-about = Download the selected folders of one or all accounts
cli-sync-run-email = Only this account (default: all)
cli-sync-nothing-selected = { $email }: nothing selected. Choose folders with "nextgdrive sync select".
cli-sync-keep-local = Keep downloaded files as regular files, no longer updated
cli-sync-remove-local = Remove downloaded files from this computer (they stay in Google Drive)
cli-folder-deselected-removed = Folder deselected. { $removed ->
    [one] 1 item removed from this computer.
   *[other] { $removed } items removed from this computer.
}
cli-daemon-about = Keep accounts in sync in the foreground, without a graphical interface
cli-daemon-started = Next GDrive is running and checks Google Drive every { $secs } seconds. Press Ctrl+C to stop.
cli-daemon-stopped = Next GDrive stopped.
cli-status-about = Show what Next GDrive is doing in the background
cli-status-not-running = Next GDrive is not running in the background. Open the app, or run "nextgdrive daemon".
cli-sync-requested = Asked the Next GDrive running in the background to sync now.
cli-daemon-already-running = Next GDrive is already running in the background (the app in the system tray, or another "nextgdrive daemon").
cli-sync-confirm-deletions = Move files deleted on this computer to the Google Drive trash, even if they are many
cli-sync-restore-deletions = Download files deleted on this computer again instead of trashing them in Google Drive

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
gui-sync-now = Sync Now
gui-sync-never = Not synced yet in this session
gui-syncing = Syncing…
gui-report-conflicts = { $count ->
    [one] 1 item needs attention
   *[other] { $count } items need attention
}
gui-report-copies = { $count ->
    [one] 1 file changed on both sides, kept as a conflict copy
   *[other] { $count } files changed on both sides, kept as conflict copies
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
gui-sync-root-note = Each account gets its own folder inside it. Changing it moves the synced files.
gui-deselect-title = Stop Syncing “{ $name }”?
gui-deselect-body = Downloaded files can be removed from this computer, where they stay available in Google Drive, or kept as regular files that are no longer updated. Files you changed on this computer are always kept.
gui-deselect-keep = Keep Files
gui-deselect-remove = Remove Files
gui-deselect-removed = { $removed ->
    [one] 1 item removed from this computer
   *[other] { $removed } items removed from this computer
}
gui-deselect-kept = { $count ->
    [one] 1 file kept because it changed on this computer
   *[other] { $count } files kept because they changed on this computer
}
gui-deselect-kept-files = Folder deselected. Its files were kept and will no longer be updated.
gui-background-group = Background
gui-autostart = Start at Login
gui-autostart-subtitle = Keep files in sync from the system tray, without opening the window
gui-host-elsewhere = Another Next GDrive process (such as "nextgdrive daemon") is already syncing in the background.
tray-open = Open Next GDrive
tray-sync-now = Sync Now
tray-pause = Pause Syncing
tray-resume = Resume Syncing
tray-quit = Quit
gui-deletions-title = { $count } Files Deleted on This Computer
gui-deletions-body = Move them to the Google Drive trash too, where they can be restored for 30 days? Or download them again?
gui-deletions-restore = Restore Files
gui-deletions-trash = Move to Trash
gui-sync-title = Sync

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
error-sync-in-progress = This account is already being synced, by this or another Next GDrive process.
error-dbus = Could not use the session bus (D-Bus): { $details }
error-local-folder-missing = The synced folder { $path } is missing (an unmounted disk?). Nothing was changed in Google Drive; sync resumes when the folder is back.
error-mass-deletion = { $count } files were deleted on this computer. Confirm before they go to the Google Drive trash, or restore them.
error-folder-exists = A folder already exists at { $path }. Choose another location, or move or remove that folder first.
