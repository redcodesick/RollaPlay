# Updater audit — pre-implementation snapshot

This document records the repository state before the GitHub Releases updater work. It is deliberately written before changing packaging, dependencies or release automation.

## Repository and release source

- Public GitHub repository: `redcodesick/RollaPlay`.
- The local NTFS clone used for this work has `origin` pointed at `F:\rocking\project`; the release workflow must use `${{ github.repository }}` on GitHub, not the local `origin` path.
- Published stable releases at audit time: `v0.1.0`, `v0.1.1`, `v0.2.0`.
- `v0.2.0` is `latest`. It is a stable release published on 2026-11-24T10:16:41Z.
- No prerelease was found.
- Release notes are plain Markdown in the GitHub Release body.

## Version declarations

All application version declarations are synchronized at `0.3.0`:

| Location | Value |
| --- | --- |
| `package.json` | `0.3.0` |
| `src-tauri/tauri.conf.json` | `0.3.0` |
| `src-tauri/Cargo.toml` | `0.3.0` |
| `src-tauri/Cargo.lock` (`rollaplay` package) | `0.3.0` |
| `src/version.ts` | `APP_VERSION = '0.3.0'` |
| Current published GitHub release | `v0.2.0` |

The `0.3.0` work is uncommitted on `feat/visual-polish-0.3.0`; no `v0.3.0` tag or GitHub Release exists.

## Tauri and dependency versions

| Component | Audited version |
| --- | --- |
| `@tauri-apps/cli` | `^2.12.5` (installed CLI reports `2.12.5`) |
| `@tauri-apps/api` | `^2.12.5` |
| `@tauri-apps/plugin-dialog` | `^2.8.4` |
| Rust `tauri` crate (locked) | `2.12.7` |
| Rust `tauri-build` crate (locked) | `2.7.6` |
| Rust `tauri-plugin-dialog` crate (locked) | `2.8.4` |
| Rust toolchain | `1.100.0` |
| Node.js | `22.14.0` |
| pnpm | `12.3.4` |

The updater plugin requires Rust >= 1.90, so the installed toolchain is compatible. Compatible stable plugin lines are `tauri-plugin-updater` 2.x / `@tauri-apps/plugin-updater` 2.x. The 3.x releases are prereleases and are not used.

## Tauri configuration

`src-tauri/tauri.conf.json` at audit time:

- `productName`: `RollaPlay`
- `identifier`: `app.windowsmediastation.local`
- `bundle.active`: `true`
- `bundle.targets`: `["nsis", "msi"]`
- Windows NSIS: `displayLanguageSelector: false`
- Windows WiX/MSI: `language: "en-US"`
- No `bundle.createUpdaterArtifacts`.
- No `plugins.updater` block.
- CSP: `default-src 'self'; img-src 'self' asset: http://asset.localhost data:; style-src 'self' 'unsafe-inline'; font-src 'self' data:; connect-src ipc: http://ipc.localhost`.

The updater plugin performs manifest and artifact requests in Rust, so the frontend CSP does not need to allow GitHub network access. Opening an external GitHub page from the UI does need an explicit opener plugin call rather than relying on arbitrary webview navigation.

## Rust plugin registration

`src-tauri/src/lib.rs` registers only `tauri_plugin_dialog::init()` before the existing application setup. No updater, process or opener plugin was registered.

## Capabilities

`src-tauri/capabilities/default.json` grants:

- `core:default`
- `core:window:allow-close`
- `dialog:allow-open`
- `dialog:allow-save`

No updater, process or opener permission existed.

## About section

`src/App.tsx` contains `AboutDialog`. It reads installed name/version/Tauri runtime with `getName()`, `getVersion()` and `getTauriVersion()` from `@tauri-apps/api/app`, falling back to `APP_VERSION`. It shows the exact version, runtime, platform and local library counts. There was no update state, update action or release-note UI.

## Packaging and release setup

### Installers already published

Every published release contains two Windows x64 installers:

| Format | v0.2.0 asset | Notes |
| --- | --- | --- |
| NSIS | `RollaPlay_0.2.0_x64-setup.exe` | per-user style installer, no admin needed for normal use |
| MSI | `RollaPlay_0.2.0_x64_en-US.msi` | WiX MSI, separate installation technology |

### GitHub Actions

There were no files under `.github/workflows/`. Releases were created manually. There was no version check, signing, `latest.json` generation or asset verification automation.

### Updater format decision

More than one installer format has been distributed publicly since `v0.1.0`, so the updater must not assume one format. Both formats are supported through one static manifest using the plugin's installer-specific target keys:

- `windows-x86_64-nsis` → signed NSIS `setup.exe`
- `windows-x86_64-msi` → signed MSI
- `windows-x86_64` → NSIS fallback for development/unknown bundle type

Tauri's updater resolves the installer-specific key first. Each entry carries its own URL and signature, so an NSIS installation never receives the MSI artifact and vice versa. The normal installers remain available for fresh installs.

### Updater artifact model

With `bundle.createUpdaterArtifacts: true`, Tauri 2 generates signatures next to the normal Windows installers:

- `RollaPlay_<version>_x64-setup.exe` + `.sig`
- `RollaPlay_<version>_x64_en-US.msi` + `.sig`

The manifest URL must point to the actual signed installer artifact, not to a fake or placeholder file. The manifest itself is generated from the tag-derived version and those build outputs.

## Security baseline before implementation

- No updater signing key existed in the repository or workflow.
- No `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secrets were configured by this project.
- No release workflow existed that could silently publish unsigned artifacts.
- The public key will be embedded in `tauri.conf.json`; the private key will be stored only in GitHub Actions Secrets and a secure local backup outside the repository.

## Documentation state

`README.md` documents NSIS/MSI build commands and installers. `CHANGELOG.md` uses semantic versions. `docs/windows-validation.md` records native validation. No updater documentation existed.
