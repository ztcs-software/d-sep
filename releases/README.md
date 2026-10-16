# Release notes

One markdown file per version, named `X.Y.Z.md` (matching the `version` field in `package.json` /
`tauri.conf.json` / `Cargo.toml` — no leading `v`).

The release workflow (`.github/workflows/release.yml`) reads the file matching the merged version and uses its
content as the GitHub release body. D-Sep has no in-app updater, so the release page is where people read
them.

## How a release happens

1. On your dev branch, when the work is ready to ship:
   - bump the version: `npm version X.Y.Z --no-git-tag-version` (updates `package.json` and
     `package-lock.json`), then edit `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml`, and refresh the
     lockfile with `cargo update -p d-sep` from `src-tauri/`. `npm test` fails until all of them agree;
   - create `releases/X.Y.Z.md` with user-facing notes (markdown is fine — headings, lists and links render
     on the release page).
2. Merge the branch to `main`.
3. The workflow detects the new version + matching notes file and starts the build. If the notes file is
   missing, or a tag or release for the version already exists, it silently skips — normal merges don't
   trigger anything.
4. When the build finishes, inspect the **draft** release on GitHub. It should have exactly two assets:

   | asset | what it is |
   | --- | --- |
   | `D-Sep_X.Y.Z_x64-setup.exe` | NSIS installer (per-user, no admin prompt) |
   | `D-Sep_X.Y.Z_x64-portable.exe` | the single `.exe`, same as `npm run exe` |

   Check that the body matches `releases/X.Y.Z.md`, and that CI is green for the merge commit — the release
   workflow doesn't wait for it.
5. Click **Publish release**. That creates the `vX.Y.Z` tag. A version with a pre-release suffix (`-beta-1`,
   `-rc-1`) publishes as a GitHub pre-release, which the repo page doesn't show as the latest release.

## Style

Aim for user-facing language, not commit-message language. Compare:

- ❌ `Refactored sort::free_stem to check sidecars before numbering`
- ✅ `A clip's .LRF and .SRT files now always get the same (2) suffix as the clip.`

A 5–10 line summary is plenty. Group by **What's new** / **Fixes** / **Breaking changes** if the release is
meaty; skip the headings if it's a small one.
