# Signed updater and release guide

RollaPlay uses the official Tauri 2 updater and process plugins. Updates are checked manually from **About → Software updates**; the app never installs silently, never downgrades, and never accepts unsigned artifacts.

## Trust model

- Manifest URL: `https://github.com/redcodesick/RollaPlay/releases/latest/download/latest.json`
- Stable releases only. Tags containing `-` never trigger the release workflow.
- The public minisign key is embedded in `src-tauri/tauri.conf.json` under `plugins.updater.pubkey`.
- The private key is never committed. It is stored only in GitHub Actions Secrets and in a protected local backup outside the repository.
- `bundle.createUpdaterArtifacts` and `plugins.updater.requireSignedVersion` are enabled; verification cannot be disabled through the UI.
- `latest.json` is generated from the real signed NSIS and MSI files produced by the tagged build. It contains the tag-derived version, the GitHub Release body as notes, the RFC 3339 publication date, and separate URLs/signatures for `windows-x86_64-nsis` and `windows-x86_64-msi`. An NSIS installation receives NSIS; an MSI installation receives MSI.
- The frontend renders only a constrained set of Markdown from release notes: headings, paragraphs, bullets, code, strong text and HTTPS links to GitHub. Raw HTML is stripped and `dangerouslySetInnerHTML` is not used.
- No GitHub token is embedded in the app. The frontend has no network access; the updater plugin performs manifest and artifact requests in Rust. The GitHub link opens through the opener plugin.

## Generate a dedicated signing key

Run this once on a trusted Windows machine, outside the repository:

```powershell
$keyDir = Join-Path $env:USERPROFILE '.tauri\rollaplay-updater'
New-Item -ItemType Directory -Force -Path $keyDir | Out-Null
$privateKey = Join-Path $keyDir 'rollaplay.key'

# The CLI writes rollaplay.key (private) and rollaplay.key.pub (public).
# Keep the password in a protected, offline-accessible backup as well as in GitHub Secrets.
pnpm exec tauri signer generate -- -w $privateKey -p '<strong-unique-password>'
```

Restrict the private key to your own Windows account:

```powershell
$identity = "$env:USERDOMAIN\$env:USERNAME"
icacls $privateKey /inheritance:r /grant:r "${identity}:(F)"
```

Copy the generated public key text into `plugins.updater.pubkey`. Never paste the private key or password into a file tracked by Git, chat, issues, logs, or screenshots.

### Local password backup on Windows

This project currently keeps a DPAPI-protected password backup at `C:\Users\sic\.tauri\rollaplay-updater\rollaplay.password.dpapi`. It is user-scoped to this Windows account and is not a portable backup. Keep a separate offline copy of both the private key and password in a password manager or encrypted external backup. If either is lost, future signed updates cannot be produced.

## GitHub Secrets

In **Settings → Secrets and variables → Actions**, create:

| Secret | Value |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | The private key contents, or a path available to the runner. Prefer the literal key contents from the password manager. |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | The password used when generating the key. |

The workflow fails immediately if either secret is absent and never echoes either value. Do not put the private key in a repository variable, environment file, release asset, or installer.

Current status: both secrets are configured on `redcodesick/RollaPlay` (uploaded 2026-11-24). Any older personal access token used for manual release cleanup should be revoked separately in GitHub developer settings.

### Rotation

Rotate the key if it may have leaked, if a maintenance account with access is removed, or before a major release line that must not accept old artifacts:

1. Generate a new key pair with the command above.
2. Replace both GitHub Secrets.
3. Replace `plugins.updater.pubkey` and commit that change.
4. Build and publish a new signed release with the new key.
5. Keep the old private key offline for recovery of already published artifacts; users on the new build no longer trust artifacts signed by the old key, so every future update must be signed with the new one.

Losing the private key or password is unrecoverable for future updates: the embedded public key will reject replacement signatures. Plan a migration release before rotating, and store recovery material in at least two secure locations.

## Release procedure

1. Ensure `package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` and `src/version.ts` all use the same semantic version.
2. Run locally:
   ```powershell
   pnpm install --frozen-lockfile
   pnpm test
   pnpm build
   cargo test --locked --manifest-path src-tauri/Cargo.toml
   ```
3. Commit and push the release commit to the default branch.
4. Create and push a stable tag:
   ```powershell
   git tag v0.3.1
   git push github v0.3.1
   ```
   Use the GitHub remote (`github` on this machine); the local `origin` points to a backup repository. Never tag a prerelease with `v*`; the workflow rejects tags containing `-`.
5. `.github/workflows/release.yml` verifies the tag and all five version declarations (`package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` and `src/version.ts`), checks the signing secrets, runs the tests, builds signed NSIS and MSI installers, creates a GitHub Release with generated notes, writes `latest.json` from the actual `.exe`, `.msi` and `.sig` files, uploads it, and verifies the published asset list.
6. Open **About → Software updates → Check for Updates** in an installed older build and complete the real update test before calling a new release production-ready. The 0.3.0 → 0.3.1 flow has passed; its recorded results are in `docs/updater-e2e-0.3.1.md`.

The workflow uses `${{ github.repository }}` to build URLs, so it does not depend on the developer machine's local `origin` path.

## Local signed build inspection

To inspect the exact updater artifact names before tagging:

```powershell
$keyDir = Join-Path $env:USERPROFILE '.tauri\rollaplay-updater'
$privateKey = Join-Path $keyDir 'rollaplay.key'
$passwordFile = Join-Path $keyDir 'rollaplay.password.dpapi'
$secure = Get-Content -LiteralPath $passwordFile -Raw | ConvertTo-SecureString
$plain = [Runtime.InteropServices.Marshal]::PtrToStringBSTR(
  [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure))
$env:TAURI_SIGNING_PRIVATE_KEY = $privateKey
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $plain
pnpm tauri build --ci
```

Expected output under `src-tauri/target/release/bundle/`:

```text
nsis/RollaPlay_<version>_x64-setup.exe
nsis/RollaPlay_<version>_x64-setup.exe.sig
msi/RollaPlay_<version>_x64_en-US.msi
msi/RollaPlay_<version>_x64_en-US.msi.sig
```

Clear the two environment variables after the build. Do not include their values in issue reports.

## Manual E2E checklist for future releases

The official updater is production-validated for the signed 0.3.0 → 0.3.1 NSIS flow on Windows. Repeat this checklist with a real GitHub Release before calling any later release production-ready:

1. Install the previous signed version from GitHub.
2. Add real folders/videos, create a playlist, set a thumbnail preference, and record the installed version.
3. Publish a newer signed test release through the workflow.
4. In the older app, open **About → Software updates**, check, verify the version, date and real notes, then choose **Download & Install**.
5. Confirm actual byte progress, a safe rejection for a deliberately bad signature in a test manifest, and that no restart happens without the user choosing it.
6. Choose **Restart Now** (or reopen from Start if Windows closed the app during installer launch), verify the new version, and confirm folders, videos, playlists, thumbnails, settings and SQLite state are intact.
7. Test offline/GitHub-unavailable, missing `latest.json`, invalid manifest, missing asset and interrupted download paths. All must show a safe error and leave local data untouched.

Windows updater installs may close the app after launching the installer; this is a documented Tauri limitation. The UI requests `restartAfterInstall: false`, shows an explicit **Restart Now** action when the install call returns, and also tells the user to reopen from Start if Windows exited the app first.
