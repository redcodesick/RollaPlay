# Release verification — 0.4.0

Published: 2026-11-25T16:32:11Z
Tag: `v0.4.0` → https://github.com/redcodesick/RollaPlay/releases/tag/v0.4.0
Workflow: https://github.com/redcodesick/RollaPlay/actions/runs/45653014938 (success, 11m5s)

## Why a new version

`0.3.x` assets had already been downloaded, so they remain untouched. `0.4.0` is a fresh stable tag with a new signed NSIS/MSI asset set and a new `latest.json`.

## Published assets

- `RollaPlay_0.4.0_x64-setup.exe` (7,161,847 bytes), SHA-256 `7801c6e00a4e17bb57d4e40ff81ab338d8c8f5625f3068e9eea919ec5e3ebeae`, plus `.sig`
- `RollaPlay_0.4.0_x64_en-US.msi` (9,412,608 bytes), SHA-256 `09fdcf21676ad0e0175e8d075802e4e7a68242dba22feb5e08261cb92b935e10`, plus `.sig`
- `latest.json`

No screenshots are attached to the release.

## Manifest check

`https://github.com/redcodesick/RollaPlay/releases/latest/download/latest.json` was fetched and compared with the uploaded release asset; they are byte-identical.

- `version`: `0.4.0`
- `pub_date`: `2026-11-25T16:32:11.000Z` (RFC 3339)
- `notes`: 1,253 characters, constrained Markdown, no raw HTML, matching the published release body
- `windows-x86_64-nsis` → signed NSIS `.exe` URL and signature
- `windows-x86_64-msi` → signed MSI `.msi` URL and signature
- `windows-x86_64` → signed NSIS fallback URL and signature
- Both manifest signatures match the published `.sig` files

The workflow initially generated a minimal release body. It was replaced with the human-readable `0.4.0` notes from `CHANGELOG.md`, and `latest.json` was regenerated from the published body and the same signed artifacts. Installer assets were not replaced.

## Changes in this release

- Real DLNA/UPnP discovery and AVTransport playback for smart TVs.
- Local LG webOS SSAP pairing and controls; SSAP is control-only, not the video transport.
- Transport-aware Connect, Cast, player bar and Home UI.
- Correlated poll generation/session checks and scoped grant retirement for DLNA playback.
- Existing Google Cast flow, RollaPlay identity, palette, fonts and signed updater are preserved.

## Validation

- `node scripts/verify-release-version.mjs v0.4.0` passed locally and in CI; all five version declarations are synchronized.
- `pnpm install --frozen-lockfile`, `pnpm test` (12/12), `pnpm build`, and `cargo test --locked` (94 passed, 1 pre-existing ignored) passed locally; CI passed the same gates.
- The user confirmed real-device validation on their LG webOS TV and a second DLNA TV before authorizing this release. Detailed per-device observations were not supplied to the assistant and were not invented.
- No dedicated interactive `0.3.3` → `0.4.0` updater E2E run was recorded. The workflow verified artifact signing, manifest generation, asset presence and absence of screenshots.

Windows SmartScreen still reports an unknown publisher because no OV/EV Authenticode code-signing certificate is configured. Updater artifacts remain minisigned and verified by the embedded public key.
