# RollaPlay visual redesign plan

## Goal

Unify the existing app into one premium, warm, high-contrast Windows media-center identity without changing the Tauri/Rust backend, SQLite schema, Cast flow, downloader implementation, playlist semantics, routing model, or persisted state keys.

## Audit findings (summary)

- All views are state-driven in `src/App.tsx`; there is no router to migrate.
- Visual identity is fragmented: purple shell, red/orange Connect/About, blue Z-bolt app icon, and Segoe UI system panels.
- `src/styles.css` has only four decorative CSS variables and 263 distinct hardcoded colors across 616 hex occurrences.
- Fonts load from Google at runtime; no local font files are bundled.
- Branding is assembled from `Aperture`, text `RP`, CSS gradients, and generated app icons; no active logo asset exists.
- Navigation is complete, but the top bar has inert controls (`SlidersHorizontal`, `⌘ K`) and toast icons ignore error/warning levels.
- Playback controls, history rows, folder-add logic, and formatting helpers are duplicated across components.
- Connect already models real mDNS devices correctly; its device semantics and sender lifecycle must be preserved.
- Downloader is functional but local state resets on unmount while the backend job continues.
- Touch targets, focus states, disabled/loading states, and empty states are inconsistent.

## Design decisions

- Use one tokenized palette: `#101010` background, `#151515` sidebar, `#1B1B1B` surfaces, `#242424` elevated, `#303030` borders, `#F2F0EC` text, `#A3A3A3` secondary, `#707070` muted, `#E53935` primary red, `#FF8C32` secondary orange.
- Red is for primary actions, active navigation, progress, connection, and destructive confirmations. Orange is restrained: hover, warnings, secondary affordances, and tool install hints.
- Use local Windows typography (`Bahnschrift` / `Segoe UI Variable` / `Segoe UI`) with no runtime network font requests; remove the Google Fonts import and narrow CSP to self-hosted fonts only.
- Create one compact inline Rolla mark for sidebar, About, Connect, splash/status surfaces, and regenerate app icons from the same geometry where tooling allows.
- Keep existing class names, component boundaries, event names, SWR keys, and localStorage keys. Change structure only where a three-zone player or reusable primitive requires it.
- Remove visibly inert controls instead of inventing behavior: the sort icon and `⌘ K` hint go away; search remains Library-scoped and clearly labeled.
- Toast icon changes by level; errors/warnings are visually distinct without new dependencies.
- All state derived from real backend data remains authoritative; no mock devices, mock progress, estimated durations, or fake playback state.

## Phase A — tokens and foundations

- Replace `src/styles.css` with a semantic token system, shared primitives, responsive rules, and component-specific sections.
- Add a small shared `BrandMark` and UI helpers (toast icon, formatting) without adding dependencies.
- Update `index.html` theme color and `tauri.conf.json` CSP to remove external font origins.
- Verify with `pnpm build`.

## Phase B — app shell

- Refactor the sidebar into a refined collapsible rail with tooltips when collapsed and preserved `wms.sidebar.collapsed`.
- Simplify the top bar to contextual breadcrumb, page-appropriate search, real Cast status, and no duplicate controls.
- Extract the persistent player bar into a three-zone component: track, transport/progress, volume/destination/queue. Preserve every handler and real event contract.
- Verify with `pnpm build`.

## Phase C — Home and Library

- Home: bold hero copy, Browse Library primary action, Open Downloader secondary action, Continue Playing when a real snapshot exists, Recently Added shelf, real playlist cards with metadata, and setup call-to-action below the shelf hidden after setup.
- Library: consistent poster grid, hover elevation, selected/focus states, contextual actions, skeleton loading, missing-thumbnail fallback, and toolbar with search/sort/filter/grid-list toggle using existing controls only.
- Verify with `pnpm build`.

## Phase D — Playlists and Queue

- Premium playlist detail shell with generated artwork, metadata, play/shuffle, track table, context actions, and real drag reorder where supported.
- Keep playlists above queue, preserve authoritative backend state, and align actions.
- Verify with `pnpm build`.

## Phase E — Cast Connect

- Restyle the app-owned device picker with one identity, real mDNS devices only, clear local-vs-receiver wording, connection states, active device in red, refresh, empty/error states, and actual discovered names.
- Preserve hidden Chrome sender, native picker gesture, and all event contracts.
- Verify with `pnpm build`.

## Phase F — Downloader and Settings

- Focus the Downloader on URL input, destination, quality, action, real progress, completed downloads, tool status, and Standard vs HD explanation.
- Preserve tool auto-install, private-link errors, library scan/add behavior, and live progress events.
- Reorganize Settings into General, Media Library, Playback, Network and Cast, Downloader, Diagnostics, and About sections without removing existing diagnostics or exposing credentials/paths.
- Verify with `pnpm build`.

## Phase G — Setup and empty states

- Restyle first-run setup as a welcoming five-step flow: Welcome, Media folders, Scan, Cast devices, Finish; preserve native folder picker and persistence.
- Keep optional existing playlist/Chrome/network checks reachable without overwhelming first run.
- Unify empty, loading, error, and success states.
- Verify with `pnpm build`.

## Phase H — final polish and validation

- Touch targets, focus rings, tooltips, hover/pressed/disabled states, reduced motion, responsive layout, no layout shift.
- Run TypeScript/build checks, Rust tests, Tauri production build, and real app screenshot capture for Home, Library, Playlist, Connect, and Downloader.
- Report changed files, dependencies, results, limitations, and screenshot paths.
