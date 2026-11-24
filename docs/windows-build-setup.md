# Windows build setup

This machine was prepared to build the Tauri 2 app in `src-tauri/` on Windows.

## Installed toolchain

| Tool | Version | Notes |
| --- | --- | --- |
| Rust / Cargo | 1.100.0 | `stable-x86_64-pc-windows-msvc` via rustup (`%USERPROFILE%\.cargo\bin`) |
| Visual Studio Build Tools 2022 | 17.14.37906.3 | `Microsoft.VisualStudio.Workload.VCTools` + recommended components |
| MSVC | 14.44.35230 | Host x64 / x64, provides `cl.exe` and `link.exe` |
| Windows SDK | 10.0.26100.0 | Required by the `windows` crate and tauri-build |
| Node.js | 22.14.0 | System install |
| pnpm | 12.3.4 | Matches `packageManager` in `package.json` |
| Git | 2.56.1 | |
| WebView2 runtime | 157.0.4341.60 | Already present on Windows 11 |

Tauri's bundler downloads NSIS and WiX on demand the first time `tauri build` runs.

## exFAT constraint

`F:` is formatted as exFAT, which does not support symlinks or junctions. pnpm's
default isolated store therefore fails. `pnpm-workspace.yaml` sets:

```yaml
nodeLinker: hoisted
packageImportMethod: copy
```

Keep those settings if the project stays on `F:`. On NTFS they are unnecessary
but harmless.

## Commands

```powershell
pnpm install
pnpm build                 # TypeScript typecheck + Vite bundle
pnpm test                  # updater tests
cargo test --manifest-path src-tauri/Cargo.toml
pnpm tauri build           # release app + NSIS/MSI installers
```

For updater artifacts, set `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` before `pnpm tauri build`; `bundle.createUpdaterArtifacts` then emits `.sig` files beside the installers. See `docs/updater-release-guide.md`. Never commit or print the key or password.

`cargo`/`rustc` are not on the PATH of already-open shells. New terminals pick
them up; for the current session run:

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
```

## Source fixes required to compile

The Phase 4 source had never been compiled and needed these corrections:

- `src-tauri/Cargo.toml`: enable the `protocol-asset` Tauri feature (the config
  uses the asset protocol), and declare `[lib] name = "rollaplay_lib"`
  so `main.rs` resolves.
- `cast_sender.rs`: derive `Clone` for `CastBridgeState`, compare the
  `RwLock` guard with `as_str()`, and lock the database before
  `record_cast_media_loaded`.
- `lib.rs`: fix borrow lifetimes in two query blocks, remove a stray deref, and
  make Tauri async commands returning `State` references return `Result`.
- `media_server.rs`: wrap status returns in `Ok`, propagate the filename lookup
  result, compare `accept-ranges` on bytes, and use `stream!` with explicit
  `Result` items for the body stream. The per-grant absolute lifetime ceiling was
  also implemented (it was ignored).
- `lib.rs`: canonicalize paths without leaving Windows `\\?\` verbatim prefixes
  in SQLite/UI values, fix borrow lifetimes in two query blocks, remove a stray
  deref, and make Tauri async commands returning `State` references return
  `Result`.
- Tests: fix two use-after-move errors and an outdated sender-asset assertion.

An app icon set was generated in `src-tauri/icons/` (the archive omitted it) and
`bundle.icon` was pinned in `tauri.conf.json`.

## Verified

- `pnpm build`: passes.
- `pnpm test`: 12 updater tests pass.
- `cargo test --manifest-path src-tauri/Cargo.toml`: 82 passed, 0 failed, 1 ignored
  (the ignored test needs `WMS_TEST_VIDEO`).
- Signed `pnpm tauri build --ci`: produces the NSIS and MSI installers and their
  `.sig` files below. Attach only the installers, signatures and generated
  `latest.json` to GitHub releases; screenshots stay in `docs/screenshots/` for
  the README gallery and are not release assets.

```
src-tauri/target/release/bundle/nsis/RollaPlay_0.3.0_x64-setup.exe
src-tauri/target/release/bundle/nsis/RollaPlay_0.3.0_x64-setup.exe.sig
src-tauri/target/release/bundle/msi/RollaPlay_0.3.0_x64_en-US.msi
src-tauri/target/release/bundle/msi/RollaPlay_0.3.0_x64_en-US.msi.sig
```
