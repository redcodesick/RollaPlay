# Release verification — 0.3.3

Published: 2026-11-25T11:16:50Z
Tag: `v0.3.3` → https://github.com/redcodesick/RollaPlay/releases/tag/v0.3.3
Workflow: https://github.com/redcodesick/RollaPlay/actions/runs/45601601533 (success, 9m8s)

## Why a new version

0.3.2 had already received downloads (2 NSIS, 1 MSI) when the full-size mark and the per-machine install mode were finalized. Overwriting those assets under the same version would have changed what existing downloaders received, so 0.3.2 was left untouched and these changes were published as 0.3.3.

## Published assets

- `RollaPlay_0.3.3_x64-setup.exe` (7,028,463 bytes), SHA-256 `6804858a2c218d9c98a9f2c354a3ca719a008c859aac2dbdf0a933a1dceb4ae9`, plus `.sig` (440 bytes)
- `RollaPlay_0.3.3_x64_en-US.msi` (9,203,712 bytes), SHA-256 `e344992aa4fe5f35f2567a463969841e755514baa3da94bae58547e861dee91b`, plus `.sig` (440 bytes)
- `latest.json` (updated 2026-11-25T11:18:52Z with the human-readable release notes, regenerated from the workflow-signed artifacts; GitHub's `latest/download` cache may take a few minutes to reflect edits)

Assets downloaded from GitHub were hash-verified against the local copies of the workflow artifacts.

## Manifest check

`https://github.com/redcodesick/RollaPlay/releases/latest/download/latest.json`:

- `version`: `0.3.3`
- `pub_date`: `2026-11-25T11:16:50.000Z` (RFC 3339)
- `notes`: real release body from GitHub, 1,224 characters, constrained Markdown only
- `windows-x86_64-nsis` → signed NSIS `.exe` (NSIS URL + signature)
- `windows-x86_64-msi` → signed MSI `.msi` (MSI URL + signature)
- `windows-x86_64` → NSIS fallback, same signed URL/signature
- Manifest signatures match the published `.sig` files
- No screenshots, credentials or private key material in any release asset

## Changes in this release

- The transparent cat-headphones PNG in `public/rolla-mark.png` was cropped to the artwork bounds (98% canvas fill) and is rendered at 100% inside the red `.rolla-mark` tile. The About and profile instances were enlarged (`size={54}` and `size={32}`) to fill their containers.
- `bundle.windows.nsis.installMode` is `perMachine`. The generated installer script contains `RequestExecutionLevel admin` and installs under `$PROGRAMFILES64\RollaPlay`. The MSI already installed per-machine.

### Install-scope migration

Changing install scope does **not** migrate an existing installation. Uninstall a copy under `%LOCALAPPDATA%\RollaPlay` before installing 0.3.3, otherwise two copies and two shortcuts can coexist. An in-app update from a per-user install cannot move the scope automatically. This is stated in the release notes.

## Validation

- `node scripts/verify-release-version.mjs v0.3.3` passed locally and in CI; all five version declarations are synchronized (`package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src/version.ts`).
- `pnpm install --frozen-lockfile`, `pnpm test` (12/12), `pnpm build` and CI Rust tests passed.
- CI verified signed artifact existence, non-empty size, release asset list and absence of screenshot assets.
- Earlier icon verification: the installed executable's 32×32 icon matches the new `icon.ico`, not the previous `R` artwork.

## Updater E2E status

No dedicated 0.3.2 → 0.3.3 interactive updater run has been recorded. Because this release changes the install scope, do not rely on the in-app updater to migrate a per-user installation: uninstall it first, then run the NSIS or MSI installer manually. After a fresh install, **About → Software updates → Check for Updates** should report version `0.3.3` with the published date and notes.
