# Release verification — 0.4.5

Published on 2026-11-26 by explicit user approval as a new stable version, so the already public `v0.4.4` assets stay immutable. The source commit is `aad8962`; the stable tag `v0.4.5` was pushed to `github/main` and the signed release workflow completed successfully (run `45906415740`). The `v0.4.0` through `v0.4.4` releases and their assets remain untouched.

Release: https://github.com/redcodesick/RollaPlay/releases/tag/v0.4.5

## Why this version

`0.4.5` adds a real Downloader **Maximum quality** selector: **Standard**, **720p max**, **1080p max** and **4K max**. Higher tiers pass a real `height<=N` filter to yt-dlp, so a lower choice cannot silently become 1080p or 4K. The default is now **1080p max** instead of the previous uncapped HD selection. FFmpeg is needed only above Standard, the opt-in TV-friendly conversion still handles AV1/VP9 sources, and Retry/Skip keep the quality chosen for the original batch.

## Published signed artifacts

Built, signed, and uploaded by `.github/workflows/release.yml` from the `v0.4.5` tag.

- `RollaPlay_0.4.5_x64-setup.exe` — 7,217,834 bytes, SHA-256 `68df9c521aa7edbe777a05081c972427cf4ccce16bdc1381b70b4f76873ddb53`
- `RollaPlay_0.4.5_x64-setup.exe.sig` — 440 bytes, SHA-256 `5fa3c159799731aea8933eaad5d2d1d11c3b8af205c33aeca7aff6c04b4b0b90`
- `RollaPlay_0.4.5_x64_en-US.msi` — 9,490,432 bytes, SHA-256 `aa4cb0631197890ce1ce8e5b6561648b98fadcb43248fd97650347867b77a73a`
- `RollaPlay_0.4.5_x64_en-US.msi.sig` — 440 bytes, SHA-256 `5640a8d83d49c51f062f29cd9a0e81c917077decde9973f75d0c704b2ac0fb2c`

## Published manifest

`docs/release-0.4.5-latest.json` is byte-for-byte identical to the published `latest.json`. The workflow initially generated a manifest from GitHub’s automatically generated release body; the release body was then replaced with the curated `docs/release-notes-0.4.5.md` and the manifest was regenerated from the real publication time and the downloaded published signatures.

- `latest.json` — 2,870 bytes, SHA-256 `efbb0b1f47959c3b78b4019d1ec202a9b1e4d2dc4af7aad69b5b013ef407301d`
- `version`: `0.4.5`
- `pub_date`: `2026-11-26T22:54:43.000Z` — the real GitHub Release publication time
- `notes`: 929 characters, constrained Markdown, no raw HTML and no URLs; matches the published release body after trimming its trailing newline
- `windows-x86_64-nsis`, `windows-x86_64-msi`, and `windows-x86_64` entries are present
- Both installer signatures match the published `.sig` assets byte-for-byte
- URLs point to the live `v0.4.5` release assets
- No screenshots are attached to the release

## Verification completed locally before tagging

- `pnpm test` — 25 passed across 4 files
- `pnpm build` — passed
- `cargo check --locked --manifest-path src-tauri/Cargo.toml` — passed with pre-existing warnings only
- `cargo test --locked --manifest-path src-tauri/Cargo.toml` — 122 passed, 0 failed, 1 pre-existing ignored
- `node scripts/verify-release-version.mjs v0.4.5` — passed; all five version declarations were synchronized
- `git diff --check` — passed with CRLF normalization warnings only

## Verification after publication

- Workflow run `45906415740`: all steps passed, including stable tag/version verification, signing-secret presence, signed artifact verification, release creation, manifest generation/upload, and published asset verification.
- Published release is public, not a draft and not a prerelease.
- Published assets are exactly `latest.json`, the NSIS installer and signature, and the MSI installer and signature. No screenshots are attached.
- Published `latest.json` matches the local copy byte-for-byte, with real notes, an RFC 3339 date, live installer URLs, and signatures matching the published `.sig` files.

## Update note

Windows SmartScreen will still report an unknown publisher because no OV/EV Authenticode code-signing certificate is configured. Updater artifacts remain minisigned and verified by the embedded public key.
