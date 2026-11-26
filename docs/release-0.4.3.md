# Release verification — 0.4.3

Published on 2026-11-26 by explicit user approval so installed apps can update. The source commit is `ffc0b90` (which includes the Downloader fix in `fe4e03d`); the stable tag `v0.4.3` was pushed to `github/main` and the signed release workflow completed successfully (run `45899581456`). The `v0.4.0`, `v0.4.1`, and `v0.4.2` releases and their assets remain untouched.

Release: https://github.com/redcodesick/RollaPlay/releases/tag/v0.4.3

## Why this version

`0.4.3` fixes Downloader progress that stayed at **Starting download…** in TV-friendly mode: `--print` made yt-dlp suppress its progress lines, so RollaPlay now explicitly requests progress and shows the real percentage, transfer stats, phase and sanitized video title. TV-friendly conversion also uses H.264 `preset slow` with `CRF 26` so compatible MP4 files are smaller. Tool installs remain manual, and updates still require explicit download, install and restart clicks.

## Published signed artifacts

Built, signed, and uploaded by `.github/workflows/release.yml` from the `v0.4.3` tag.

- `RollaPlay_0.4.3_x64-setup.exe` — 7,213,437 bytes, SHA-256 `bb7e453614bf1929d8aac939af8a6f1da094d60f83f02edd6ec72c56d5986305`
- `RollaPlay_0.4.3_x64-setup.exe.sig` — 440 bytes, SHA-256 `7b7c6bb78c35cb1e846563f918ae50c49d92a55a3c39d81de690fece34f25f38`
- `RollaPlay_0.4.3_x64_en-US.msi` — 9,478,144 bytes, SHA-256 `96fe289a3e9e31f1be3b86e2cb36a6f0b5ce22214e55e2e9021b1401e0f5055e`
- `RollaPlay_0.4.3_x64_en-US.msi.sig` — 440 bytes, SHA-256 `333abcffd656d7321534edf28374faba62850bd3ef09bb6fc6c0d6aef04419a6`

## Published manifest

`docs/release-0.4.3-latest.json` is byte-for-byte identical to the published `latest.json`. The workflow initially generated a manifest from GitHub’s automatically generated release body; the release body was then replaced with the curated `docs/release-notes-0.4.3.md` and the manifest was regenerated from the real publication time and the downloaded published signatures.

- `latest.json` — 2,867 bytes, SHA-256 `7f4fe393e296361ee7f79196dd72761f3e97af3ffa6385ac576295ae1ae81f95`
- `version`: `0.4.3`
- `pub_date`: `2026-11-26T21:49:24.000Z` — the real GitHub Release publication time
- `notes`: 927 characters, constrained Markdown, no raw HTML and no URLs; matches the published release body after trimming its trailing newline
- `windows-x86_64-nsis`, `windows-x86_64-msi`, and `windows-x86_64` entries are present
- Both installer signatures match the published `.sig` assets byte-for-byte
- URLs point to the live `v0.4.3` release assets
- No screenshots are attached to the release

## Verification completed locally before tagging

- `pnpm test` — 23 passed across 4 files
- `pnpm build` — passed
- `cargo check --locked --manifest-path src-tauri/Cargo.toml` — passed with pre-existing warnings only
- `cargo test --locked --manifest-path src-tauri/Cargo.toml` — 119 passed, 0 failed, 1 pre-existing ignored
- `node scripts/verify-release-version.mjs v0.4.3` — passed; all five version declarations were synchronized
- `git diff --check` — passed with CRLF normalization warnings only

## Verification after publication

- Workflow run `45899581456`: all steps passed, including stable tag/version verification, signing-secret presence, signed artifact verification, release creation, manifest generation/upload, and published asset verification.
- Published release is public, not a draft and not a prerelease.
- Published assets are exactly `latest.json`, the NSIS installer and signature, and the MSI installer and signature. No screenshots are attached.
- Published `latest.json` matches the local copy byte-for-byte, with real notes, an RFC 3339 date, live installer URLs, and signatures matching the published `.sig` files.

## Update note

Windows SmartScreen will still report an unknown publisher because no OV/EV Authenticode code-signing certificate is configured. Updater artifacts remain minisigned and verified by the embedded public key.
