# Contributing to Next GDrive

Thanks for helping! Questions and ideas are welcome in [Discussions](https://github.com/diegofaraujo/nextgdrive/discussions); bugs and concrete feature requests go in [Issues](https://github.com/diegofaraujo/nextgdrive/issues/new/choose).

## Workflow

`main` is protected: every change reaches it through a pull request whose checks pass.

1. For anything beyond a small fix, open or comment on an issue first, so the approach can be agreed before you write code.
2. Create a branch from an up-to-date `main`, named after the kind of change:
   - `feat/<short-description>` for features (e.g. `feat/shared-drive-icons`)
   - `fix/<short-description>` for bugs (e.g. `fix/stream-rename-conflict`)
   - `docs/…`, `refactor/…`, `chore/…` (build, CI, dependencies) for the rest
3. Keep each pull request focused on one change, and link its issue (`Closes #123`).
4. Pull requests are squash-merged, so the PR title becomes the commit message on `main`: write it in the imperative, in English (e.g. "Restore streamed files deleted from the desktop trash").

## Building and checks

See the [README](README.md#building-from-source) for requirements. Before opening a pull request:

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs the same checks, plus a build with the minimum supported Rust version (1.92).

## Releases

Versions follow [Semantic Versioning](https://semver.org/) and are published by tag: pushing `vX.Y.Z` makes GitHub Actions build the packages and publish the release.

1. Open a pull request named "Release X.Y.Z" that:
   - sets `version` in the root `Cargo.toml` (and runs `cargo check` to update `Cargo.lock`);
   - moves the `Unreleased` entries of `CHANGELOG.md` under `## [X.Y.Z] - <date>`, and updates the links at the bottom;
   - adds a `<release version="X.Y.Z" date="<date>">` at the top of `<releases>` in `data/io.github.diegofaraujo.NextGDrive.metainfo.xml`, with the highlights.

   A test fails if those three disagree.
2. Merge it, then tag the merge commit on `main`:

   ```bash
   git switch main && git pull
   git tag -a vX.Y.Z -m "Next GDrive X.Y.Z"
   git push origin vX.Y.Z
   ```

Release builds embed the project's OAuth client from the repository secrets `NEXTGDRIVE_CLIENT_ID` and `NEXTGDRIVE_CLIENT_SECRET` (read at build time). Development builds have none, so each contributor uses their own client; your own client (environment variables or `oauth-client.json`) always takes precedence over the embedded one.

## Guidelines

- **Architecture.** The project follows Clean Architecture with Ports & Adapters, one crate per layer or adapter; dependencies point inward only. `domain` and `application` never depend on Google APIs, the filesystem, GTK or clap. [CLAUDE.md](CLAUDE.md) describes each crate, the patterns to follow, and the invariants the sync engine keeps.
- **Never lose data.** Changes to sync or streaming must keep the rules in CLAUDE.md (nothing is deleted permanently in Drive, a change on both sides keeps both versions, and so on), with tests. Check them against a real account in an isolated environment, as CLAUDE.md explains, using a test account and a dedicated folder.
- **English everywhere.** Code, comments, commit messages and documentation are in English. Only translation files contain other languages.
- **Translations.** No user-facing text is hardcoded: add each message to `crates/i18n/locales/{en-US,pt-BR,es}/main.ftl` (a test checks that all locales have the same messages). Improvements to translations, and new languages, are very welcome.
- **Dependencies** must have a license compatible with GPL-3.0-or-later (MIT, Apache-2.0, BSD, MPL-2.0, …).

## License

By contributing, you agree that your contributions are licensed under the [GPL-3.0-or-later](LICENSE).
