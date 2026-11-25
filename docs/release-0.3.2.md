# Release verification — 0.3.2

Published: 2026-11-25T10:12:13Z
Tag: `v0.3.2` → https://github.com/redcodesick/RollaPlay/releases/tag/v0.3.2
Workflow: https://github.com/redcodesick/RollaPlay/actions/runs/45591088440 (success, 10m39s)

## Published assets

- `RollaPlay_0.3.2_x64-setup.exe` (6,785,759 bytes) and `.sig` (440 bytes)
- `RollaPlay_0.3.2_x64_en-US.msi` (8,888,320 bytes) and `.sig` (440 bytes)
- `latest.json` (updated 2026-11-25T10:14:57Z with the human-readable release notes; GitHub's `latest/download` cache may take a few minutes to reflect the edit)

All five version declarations were verified against the tag: `package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src/version.ts`.

## Manifest check

`https://github.com/redcodesick/RollaPlay/releases/latest/download/latest.json`:

- `version`: `0.3.2`
- `pub_date`: `2026-11-25T10:12:13.000Z` (RFC 3339)
- `notes`: real release body from GitHub, 1,110 characters, constrained Markdown only
- `windows-x86_64-nsis` → signed NSIS `.exe` (NSIS URL + signature)
- `windows-x86_64-msi` → signed MSI `.msi` (MSI URL + signature)
- `windows-x86_64` → NSIS fallback, same signed URL/signature
- No screenshots, credentials or private key material in any release asset

## Local and CI validation

- `node scripts/verify-release-version.mjs v0.3.2` passed locally and in CI.
- `pnpm install --frozen-lockfile`, `pnpm test` (12/12), `pnpm build`, `cargo test --locked` (84 passed, 0 failed, 1 pre-existing ignored) passed locally and in CI.
- `cargo clippy` exit 0 with only pre-existing warnings; `git diff --check` clean.
- Real GitHub refresh check: a deliberately stale 5-byte `yt-dlp.exe` was replaced with the 23,627,865-byte official binary, which reported version `2026.11.23`.
- CI additionally verified signed artifact existence, non-empty size, release asset list and absence of screenshot assets.

## Updater E2E status

The updater implementation is unchanged from the validated 0.3.0 → 0.3.1 signed NSIS flow recorded in `docs/updater-e2e-0.3.1.md`. A dedicated 0.3.1 → 0.3.2 interactive E2E run has **not** been recorded. Before calling this release production-ready in an existing install, open **About → Software updates → Check for Updates**, confirm version `0.3.2`, the date and the release notes, then complete Download & Install and the explicit Restart Now step. MSI parity and deliberate negative/error-path checks also remain unrecorded for this version.
