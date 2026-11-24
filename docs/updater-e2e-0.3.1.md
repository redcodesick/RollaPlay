# Controlled updater E2E test: 0.3.0 → 0.3.1

**Status: PASS — recorded 2026-11-25.** The maintainer completed the core update flow on Windows from signed 0.3.0 NSIS to signed 0.3.1. About detected 0.3.1, the signed artifact verified, installation completed without an unrequested restart, the explicit restart/reopen showed 0.3.1, and library folders, videos, playlists, thumbnails, history, settings and SQLite state were preserved. The updater is production-validated for this flow.

This release exists only to validate the signed in-app updater shipped in 0.3.0.

Release: `v0.3.1` (published 2026-11-24T21:58:04Z), `https://github.com/redcodesick/RollaPlay/releases/tag/v0.3.1`.

## Result scope

| Scope | Result |
| --- | --- |
| Signed release assets and public `latest.json` | PASS — version `0.3.1`, real NSIS/MSI URLs and signatures |
| Core steps 1–8 below | PASS — reported by maintainer |
| Data preservation after restart/reopen | PASS |
| Optional MSI parity check | Not recorded in this pass; still recommended for future releases |
| Negative/error-path checks | Not recorded in this pass; still recommended for future releases |

## Prerequisites

- Windows with WebView2 and internet access to GitHub.
- The signed 0.3.0 NSIS installer: `https://github.com/redcodesick/RollaPlay/releases/download/v0.3.0/RollaPlay_0.3.0_x64-setup.exe`
- App data lives in `%LOCALAPPDATA%\app.windowsmediastation.local`. Do not delete it during the test.
- Do not rotate signing keys or attach screenshots to GitHub releases.

## Manual steps and expected results

| # | Action | Expected result |
| --- | --- | --- |
| 1 | Install RollaPlay 0.3.0 from the NSIS `.exe` above and launch it. | Install completes; About shows `0.3.0`. |
| 2 | Add a real folder containing at least one video, create a playlist with that video, and change one setting such as the compact sidebar or thumbnail preference. | Library, playlist and setting are visible immediately. Record the playlist name and setting for step 8. |
| 3 | Close and reopen RollaPlay. | Version is still `0.3.0`; the folder, video, playlist and setting persist. |
| 4 | Open **About → Software updates → Check for Updates**. | Status shows Checking, then **Update available** with installed `0.3.0` and latest `0.3.1`. |
| 5 | Inspect the displayed release date and notes. | Date and notes come from the real GitHub Release; no placeholder text or unsigned warning appears. |
| 6 | Choose **Download & Install**. | Actual byte progress advances, status becomes Installing, and no restart happens without the explicit **Restart Now** action. Windows may close the app while launching the installer. |
| 7 | Choose **Restart Now**, or reopen from the Start menu if Windows exited the app first. | About shows `0.3.1`. |
| 8 | Reopen Library, the recorded playlist, playback history and the changed setting. | Folders, videos, playlists, thumbnails, history, settings and SQLite state are intact. |

Optional parity check (not recorded in this pass): repeat the same flow from the 0.3.0 MSI installer to confirm the MSI updater target stays on signed MSI artifacts and never cross-grades to NSIS.

## Troubleshooting

- **No update found:** confirm the endpoint `https://github.com/redcodesick/RollaPlay/releases/latest/download/latest.json` returns version `0.3.1`, the machine has internet access, and the 0.3.1 release is published and not a prerelease. NSIS installations must receive the NSIS asset.
- **Signature, manifest or missing-asset error:** do not bypass verification. Record the error text, release URL and workflow run ID, then report them. Local data is not modified by a rejected update.
- **Download interrupted:** choose Retry once the network is stable. A failed or cancelled download must leave the installed 0.3.0 build and user data untouched.
- **App closes during install:** this is a documented Windows Tauri updater limitation. Reopen RollaPlay from the Start menu; install mode is `passive` and `restartAfterInstall` is false.
- **Version remains 0.3.0:** verify the NSIS asset URL in `latest.json` and that the `.sig` file is present. Do not accept an unsigned or mismatched artifact.
- **Library data is missing:** stop the test, preserve `%LOCALAPPDATA%\app.windowsmediastation.local` and report the exact steps. Do not delete or rebuild the database.

Negative checks worth repeating (not recorded in this pass): offline mode, GitHub unavailable, missing `latest.json`, invalid manifest, missing installer asset, deliberately bad signature and interrupted download. Each must show a safe error and leave local data untouched.
