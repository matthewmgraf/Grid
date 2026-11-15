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
   - Title: e.g. `Grid 0.2.0`. Fill in the description from the template below (shown via the
     update prompt's link).
   - Attach `target\release\grid.exe`. It must be named exactly `grid.exe`.
   - Leave "Set as a pre-release" unticked: pre-releases aren't offered as updates.
   - Click *Publish release*.
6. Running Grids with an older version find it on their next start (or Tray → Check for
   updates).

## Release notes template

Before publishing, get the file's hash and a VirusTotal scan:

1. `Get-FileHash target\release\grid.exe` (PowerShell) prints the SHA-256. Write it in lower
   case below.
2. Upload `grid.exe` at <https://www.virustotal.com>. Its report page is
   `https://www.virustotal.com/gui/file/<sha256>/detection`; check that the hash shown there is
   the same.
3. Put the report's result in the badge: `0%2F72` means "0/72" (`%2F` is `/`). Use
   `brightgreen` when nothing flags it, `yellow` otherwise. The badge doesn't update itself.

```markdown
### What's new
- Fixed …
- Added …

[Install guide](https://github.com/matthewmgraf/Grid#download-and-install) | [Support Grid](https://github.com/sponsors/matthewmgraf)

| File | SHA-256 | VirusTotal Status | VirusTotal Report |
|:---:|:---:|:---:|:---:|
| grid.exe | `<sha256>` | [![VirusTotal](https://img.shields.io/badge/VirusTotal-0%2F72-brightgreen)](https://www.virustotal.com/gui/file/<sha256>/detection) | [View Report](https://www.virustotal.com/gui/file/<sha256>/detection) |

To check your download, run `Get-FileHash grid.exe` in PowerShell and compare the result with
the SHA-256 above. The VirusTotal report is for the file with that same hash.
```
