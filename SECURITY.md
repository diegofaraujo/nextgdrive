# Security policy

Next GDrive has access to your Google Drive, so security reports are taken seriously.

## Reporting a vulnerability

Please **do not open a public issue**. Report it privately through [GitHub's security advisories](https://github.com/diegofaraujo/nextgdrive/security/advisories/new), with the steps to reproduce and the impact you expect. You should get a first answer within a week.

## Supported versions

The project is in early development: fixes go to the latest release and `main`.

## Scope

Of particular interest: leaking OAuth tokens (they must only be stored in the system keyring), access to files outside the sync folder, and anything that could delete or overwrite data in Drive without the user asking.
