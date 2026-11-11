# Publishing a Grid release

Grid checks `https://api.github.com/repos/matthewmgraf/Grid/releases/latest` for updates.
That only works once the repository is **public**: GitHub answers "not found" to apps asking
about a private repository's releases.

1. **Pick the new version number**, e.g. `0.2.0`, and set it in the root `Cargo.toml`
   (`[workspace.package] version`). Grid compares this number with the release's tag.
2. **Build**: `cargo build --release`. The program is `target\release\grid.exe`.
3. **Check it**: `cargo test` and `cargo clippy` pass, and you've run through `TESTING.md`.
4. **Commit** the version change and push (GitHub Desktop: Commit, then Push origin).
5. **Create the release on GitHub**: the repository page → Releases → *Draft a new release*.
   - *Choose a tag*: type `v0.2.0` (a `v` plus the version) and pick "Create new tag".
   - Title: e.g. `Grid 0.2.0`. Describe what changed (shown via the update prompt's link).
   - Attach `target\release\grid.exe`. It must be named exactly `grid.exe`.
   - Leave "Set as a pre-release" unticked: pre-releases aren't offered as updates.
   - Click *Publish release*.
6. Running Grids with an older version find it on their next start (or Tray → Check for
   updates).
