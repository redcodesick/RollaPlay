# Release verification — 0.4.4

Published on 2026-11-26 by explicit user approval so installed apps can use the new Downloader retry and skip actions. The source commit is `bb23769` (which includes `6b654a7`); the stable tag `v0.4.4` was pushed to `github/main` and the signed release workflow completed successfully (run `45903916119`). The `v0.4.0`, `v0.4.1`, `v0.4.2`, and `v0.4.3` releases and their assets remain untouched.

Release: https://github.com/redcodesick/RollaPlay/releases/tag/v0.4.4

## Why this version

`0.4.4` adds real **Retry** and **Skip** actions to failed Downloader rows, including the fixed sign-in/membership failure. Retry reruns the failed link and any unfinished links after it; Skip marks the link skipped and continues. Resumed batches keep their original `Link N of M` numbering and never repeat completed, skipped or cancelled links. Library indexing waits until every original link is settled and at least one file completed, so a failed link is never silently treated as done.

## Published signed artifacts

Built, signed, and uploaded by `.github/workflows/release.yml` from the `v0.4.4` tag.

- `RollaPlay_0.4.4_x64-setup.exe` — 7,217,556 bytes, SHA-256 `fd5f005e8e2b68e37e3a9fac886f5e92ab51300d1fb2ab6778535b91a5db78b1`
- `RollaPlay_0.4.4_x64-setup.exe.sig` — 440 bytes, SHA-256 `ef986335358f34da2b38e540523dcfdaf5924467bef8d9ec48914c0c9bfe4805`
- `RollaPlay_0.4.4_x64_en-US.msi` — 9,490,432 bytes, SHA-256 `47a0eb2e8efb46455da109fefd3e175906a58057b4f1cf132c905de9d55f6758`
- `RollaPlay_0.4.4_x64_en-US.msi.sig` — 440 bytes, SHA-256 `6ac8d2a751d75b1d68dcfd6e0e01b224108507f05ddbac99a49b09a645c77fda`

## Published manifest

`docs/release-0.4.4-latest.json` is byte-for-byte identical to the published `latest.json`. The workflow initially generated a manifest from GitHub’s automatically generated release body; the release body was then replaced with the curated `docs/release-notes-0.4.4.md` and the manifest was regenerated from the real publication time and the downloaded published signatures.

- `latest.json` — 2,745 bytes, SHA-256 `00eb9c970fd500af57d2af37927d1d114eb77bd7606b7792efd7d4a93e0a55c2`
- `version`: `0.4.4`
- `pub_date`: `2026-11-26T22:29:14.000Z` — the real GitHub Release publication time
- `notes`: 805 characters, constrained Markdown, no raw HTML and no URLs; matches the published release body after trimming its trailing newline
- `windows-x86_64-nsis`, `windows-x86_64-msi`, and `windows-x86_64` entries are present
- Both installer signatures match the published `.sig` assets byte-for-byte
- URLs point to the live `v0.4.4` release assets
- No screenshots are attached to the release

## Verification completed locally before tagging

- `pnpm test` — 25 passed across 4 files
- `pnpm build` — passed
- `cargo check --locked --manifest-path src-tauri/Cargo.toml` — passed with pre-existing warnings only
- `cargo test --locked --manifest-path src-tauri/Cargo.toml` — 120 passed, 0 failed, 1 pre-existing ignored
- `node scripts/verify-release-version.mjs v0.4.4` — passed; all five version declarations were synchronized
- `git diff --check` — passed with CRLF normalization warnings only

## Verification after publication

- Workflow run `45903916119`: all steps passed, including stable tag/version verification, signing-secret presence, signed artifact verification, release creation, manifest generation/upload, and published asset verification.
- Published release is public, not a draft and not a prerelease.
- Published assets are exactly `latest.json`, the NSIS installer and signature, and the MSI installer and signature. No screenshots are attached.
- Published `latest.json` matches the local copy byte-for-byte, with real notes, an RFC 3339 date, live installer URLs, and signatures matching the published `.sig` files.

## Update note

Windows SmartScreen will still report an unknown publisher because no OV/EV Authenticode code-signing certificate is configured. Updater artifacts remain minisigned and verified by the embedded public key.
