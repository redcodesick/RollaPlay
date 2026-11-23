use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager, State};
use walkdir::WalkDir;

mod cast_discovery;
mod cast_sender;
mod media_server;
mod network;
mod playback_orchestrator;
mod playlist;
mod playlist_file;
mod secret;
mod sender_embed;
mod thumbnail;

use cast_discovery::CastDevice;
use cast_sender::{CastDiagnosticsSnapshot, CastEventReport, CastPlaybackSnapshot, CastSenderManager, CastSenderStatus};
use media_server::{ConnectionTestResult, MediaServerManager, MediaServerStatus, StreamDiagnosticsSnapshot};
use network::NetworkDiagnostics;
use playback_orchestrator::PlaybackOrchestrator;
use playlist::{
    AddMode, EngineEffect, PlaybackEngine, PlaybackMode, PlaylistItem, PlaylistSummary, RepeatMode,
};
use playlist_file::{
    export_m3u, export_wpl, parse_playlist_file, reference_to_local_path, ExportItem,
    ParsedPlaylistFile, PlaylistFileKind,
};
use tauri::Listener;

/// Canonicalizes a path without leaving Windows verbatim (`\\?\`) prefixes in
/// the values stored in SQLite or shown in the UI. UNC paths keep their
/// network form (`\\server\share`).
fn canonicalize_media_path(path: &Path) -> std::io::Result<PathBuf> {
    let canonical = path.canonicalize()?;
    #[cfg(windows)]
    {
        let text = canonical.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            if let Some(unc) = rest.strip_prefix("UNC\\") {
                return Ok(PathBuf::from(format!(r"\\{unc}")));
            }
            return Ok(PathBuf::from(rest));
        }
    }
    Ok(canonical)
}

struct AppDatabase(Arc<Mutex<Connection>>);

struct ScanRegistry(Arc<Mutex<HashMap<i64, Arc<AtomicBool>>>>);

#[derive(Serialize)]
struct CastLoadOutcome {
    media_file_id: i64,
    filename: String,
    mime_type: String,
    expires_in_seconds: u64,
}

#[derive(Clone)]
struct ThumbnailService {
    cache_dir: PathBuf,
    queue: Arc<Mutex<ThumbnailQueue>>,
}

#[derive(Default)]
struct ThumbnailQueue {
    pending: VecDeque<i64>,
    queued: HashSet<i64>,
    rerun: HashSet<i64>,
    active: Option<i64>,
    running: bool,
}

struct ScanGuard {
    directory_id: i64,
    registry: Arc<Mutex<HashMap<i64, Arc<AtomicBool>>>>,
}

impl Drop for ScanGuard {
    fn drop(&mut self) {
        if let Ok(mut scans) = self.registry.lock() {
            scans.remove(&self.directory_id);
        }
    }
}

const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "wmv", "webm", "mpeg", "mpg", "m4v", "3gp",
];
const PLAYLIST_EXTENSIONS: &[&str] = &["m3u", "m3u8", "wpl"];

#[derive(Serialize)]
struct LocalStatus {
    database_ready: bool,
}

#[derive(Serialize)]
struct MediaDirectory {
    id: i64,
    path: String,
    added_at: String,
    last_scanned_at: Option<String>,
    availability_state: String,
}

#[derive(Serialize)]
struct MediaFile {
    id: i64,
    directory_id: i64,
    path: String,
    filename: String,
    extension: String,
    size_bytes: u64,
    modified_at: String,
    indexed_at: String,
    availability_state: String,
    thumbnail_path: Option<String>,
    thumbnail_status: String,
    thumbnail_error: Option<String>,
}

#[derive(Serialize, Clone)]
struct ScanProgress {
    directory_id: i64,
    files_found: usize,
    inaccessible_entries: usize,
    phase: String,
}

#[derive(Serialize)]
struct ScanResult {
    directory_id: i64,
    files_indexed: usize,
    files_removed: usize,
    renamed_files: usize,
    temporarily_unavailable_files: usize,
    permanently_missing_files: usize,
    inaccessible_entries: usize,
    playlists_imported: usize,
    cancelled: bool,
}

#[derive(Serialize, Clone)]
struct ThumbnailProgress {
    media_file_id: i64,
    status: String,
    error: Option<String>,
}

#[derive(Serialize)]
struct ThumbnailSettings {
    width: u32,
    height: u32,
    quality: u8,
}

#[derive(Serialize)]
struct CacheCleanupResult {
    files_removed: usize,
    bytes_removed: u64,
}

struct IndexedFile {
    path: String,
    filename: String,
    extension: String,
    size_bytes: i64,
    modified_at: String,
}

#[derive(Clone)]
struct PlaylistFileCandidate {
    path: PathBuf,
    filename: String,
    extension: String,
    size_bytes: i64,
    modified_at: String,
    directory_id: Option<i64>,
}

#[derive(Serialize, Clone, Debug, Default)]
struct PlaylistFileImportSummary {
    files_seen: usize,
    playlists_created: usize,
    playlists_updated: usize,
    items_imported: usize,
    skipped: usize,
    failed: usize,
}

#[derive(Serialize, Clone, Debug)]
struct PlaylistFileExportResult {
    items_written: usize,
    format: String,
}

#[derive(Clone)]
struct ExistingPlaylistFileImport {
    playlist_id: Option<i64>,
    size_bytes: i64,
    modified_at: String,
    status: String,
}

#[tauri::command]
fn get_local_status(database: State<'_, AppDatabase>) -> Result<LocalStatus, String> {
    let connection = database.0.lock().map_err(|_| "Local database is unavailable".to_string())?;
    connection
        .query_row("SELECT 1", [], |_| Ok(()))
        .map_err(|_| "Local database is unavailable".to_string())?;
    Ok(LocalStatus { database_ready: true })
}

/// Temporary first-run diagnostic sink. The webview console is not visible in
/// packaged builds, so the wizard forwards redacted control-flow lines here.
#[tauri::command]
fn wizard_diag_log(
    app: tauri::AppHandle,
    area: String,
    action: String,
    details: Option<String>,
) -> Result<(), String> {
    let directory = app.path().app_local_data_dir().map_err(|error| error.to_string())?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let clean = |value: &str| value.replace(['\r', '\n'], " ");
    let line = format!(
        "{} [{}] {} {}\n",
        millis,
        clean(&area),
        clean(&action),
        details.map(|value| clean(&value)).unwrap_or_default(),
    );
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("wizard-diagnostics.log"))
        .map_err(|error| error.to_string())?;
    use std::io::Write;
    file.write_all(line.as_bytes()).map_err(|error| error.to_string())
}

#[tauri::command]
fn add_media_directory(database: State<'_, AppDatabase>, path: String) -> Result<MediaDirectory, String> {
    let canonical_path = canonicalize_media_path(Path::new(&path))
        .map_err(|_| "Choose a folder that exists on this device".to_string())?;
    if !canonical_path.is_dir() {
        return Err("The selected path is not a folder".to_string());
    }
    let canonical_path = canonical_path.to_string_lossy().into_owned();
    let connection = database.0.lock().map_err(|_| "Local database is unavailable".to_string())?;
    let active_paths = {
        let mut statement = connection
            .prepare("SELECT path FROM media_directories WHERE is_active = 1")
            .map_err(|_| "Could not check existing library folders".to_string())?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| "Could not check existing library folders".to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "Could not check existing library folders".to_string())?;
        rows
    };
    let selected_path = PathBuf::from(&canonical_path);
    for active_path in active_paths {
        let active_path = PathBuf::from(active_path);
        if selected_path != active_path
            && (selected_path.starts_with(&active_path) || active_path.starts_with(&selected_path))
        {
            return Err("This folder overlaps an existing library folder. Choose the parent folder or a separate folder instead.".to_string());
        }
    }
    connection
        .execute(
            "INSERT INTO media_directories(path, is_active, availability_state) VALUES (?1, 1, 'available')
             ON CONFLICT(path) DO UPDATE SET is_active = 1, availability_state = 'available'",
            [&canonical_path],
        )
        .map_err(|_| "Could not save the selected folder".to_string())?;
    connection
        .query_row(
            "SELECT id, path, added_at, last_scanned_at, availability_state FROM media_directories WHERE path = ?1",
            [&canonical_path],
            |row| {
                Ok(MediaDirectory {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    added_at: row.get(2)?,
                    last_scanned_at: row.get(3)?,
                    availability_state: row.get(4)?,
                })
            },
        )
        .map_err(|_| "Could not load the selected folder".to_string())
}

#[tauri::command]
fn list_media_directories(database: State<'_, AppDatabase>) -> Result<Vec<MediaDirectory>, String> {
    let connection = database.0.lock().map_err(|_| "Local database is unavailable".to_string())?;
    let mut statement = connection
        .prepare("SELECT id, path, added_at, last_scanned_at, availability_state FROM media_directories WHERE is_active = 1 ORDER BY path COLLATE NOCASE")
        .map_err(|_| "Could not read media folders".to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(MediaDirectory {
                id: row.get(0)?,
                path: row.get(1)?,
                added_at: row.get(2)?,
                last_scanned_at: row.get(3)?,
                availability_state: row.get(4)?,
            })
        })
        .map_err(|_| "Could not read media folders".to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|_| "Could not read media folders".to_string())
}

#[tauri::command]
fn remove_media_directory(
    database: State<'_, AppDatabase>,
    scans: State<'_, ScanRegistry>,
    directory_id: i64,
) -> Result<(), String> {
    let active_scans = scans.0.lock().map_err(|_| "Could not check folder scanning".to_string())?;
    if active_scans.contains_key(&directory_id) {
        return Err("Cancel the active scan before removing this folder".to_string());
    }
    let mut connection = database.0.lock().map_err(|_| "Local database is unavailable".to_string())?;
    remove_directory_from_database(&mut connection, directory_id)
}

fn remove_directory_from_database(connection: &mut Connection, directory_id: i64) -> Result<(), String> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(|_| "Could not remove the folder from your library".to_string())?;
    transaction
        .execute("UPDATE media_directories SET is_active = 0, availability_state = 'removed_from_library' WHERE id = ?1", [directory_id])
        .map_err(|_| "Could not remove the folder from your library".to_string())?;
    transaction
        .execute("UPDATE media_files SET is_available = 0, availability_state = 'removed_from_library' WHERE directory_id = ?1", [directory_id])
        .map_err(|_| "Could not update folder items".to_string())?;
    transaction
        .execute("UPDATE thumbnail_cache SET status = 'pending', cache_path = NULL, error_message = NULL WHERE media_file_id IN (SELECT id FROM media_files WHERE directory_id = ?1)", [directory_id])
        .map_err(|_| "Could not invalidate folder thumbnails".to_string())?;
    transaction.commit().map_err(|_| "Could not finish removing the folder".to_string())
}

#[tauri::command]
fn list_media_files(
    database: State<'_, AppDatabase>,
    search: Option<String>,
    extension: Option<String>,
    sort_by: Option<String>,
) -> Result<Vec<MediaFile>, String> {
    let sort_column = match sort_by.as_deref() {
        Some("size") => "size_bytes DESC, filename COLLATE NOCASE",
        Some("date") => "indexed_at DESC, filename COLLATE NOCASE",
        _ => "filename COLLATE NOCASE",
    };
    let sql = format!(
        "SELECT f.id, f.directory_id, f.path, f.filename, f.extension, f.size_bytes, f.modified_at, f.indexed_at,
                f.availability_state, t.cache_path, COALESCE(t.status, 'pending'), t.error_message
         FROM media_files f
         LEFT JOIN thumbnail_cache t ON t.media_file_id = f.id
         WHERE f.is_available = 1
           AND EXISTS (SELECT 1 FROM media_directories d WHERE d.id = f.directory_id AND d.is_active = 1)
           AND (?1 IS NULL OR f.filename LIKE ?1 ESCAPE '\\')
           AND (?2 IS NULL OR f.extension = ?2)
         ORDER BY {sort_column}"
    );
    let search = search.map(|value| format!("%{}%", escape_like(&value)));
    let extension = extension.map(|value| value.trim_start_matches('.').to_ascii_lowercase());
    let connection = database.0.lock().map_err(|_| "Local database is unavailable".to_string())?;
    let mut statement = connection.prepare(&sql).map_err(|_| "Could not read your library".to_string())?;
    let rows = statement
        .query_map(params![search, extension], |row| {
            Ok(MediaFile {
                id: row.get(0)?,
                directory_id: row.get(1)?,
                path: row.get(2)?,
                filename: row.get(3)?,
                extension: row.get(4)?,
                size_bytes: row.get::<_, i64>(5)? as u64,
                modified_at: row.get(6)?,
                indexed_at: row.get(7)?,
                availability_state: row.get(8)?,
                thumbnail_path: row.get(9)?,
                thumbnail_status: row.get(10)?,
                thumbnail_error: row.get(11)?,
            })
        })
        .map_err(|_| "Could not read your library".to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|_| "Could not read your library".to_string())
}

fn escape_like(value: &str) -> String {
    value.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

fn register_scan(scans: &ScanRegistry, directory_id: i64) -> Result<Arc<AtomicBool>, String> {
    let mut active = scans.0.lock().map_err(|_| "Could not start folder scanning".to_string())?;
    if active.contains_key(&directory_id) {
        return Err("This folder is already being scanned".to_string());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    active.insert(directory_id, Arc::clone(&cancel));
    Ok(cancel)
}

#[tauri::command]
async fn scan_media_directory(
    app: tauri::AppHandle,
    database: State<'_, AppDatabase>,
    scans: State<'_, ScanRegistry>,
    thumbnails: State<'_, ThumbnailService>,
    directory_id: i64,
) -> Result<ScanResult, String> {
    let cancel = register_scan(scans.inner(), directory_id)?;
    let _scan_guard = ScanGuard {
        directory_id,
        registry: Arc::clone(&scans.0),
    };
    let database = Arc::clone(&database.0);
    let root = {
        let connection = database.lock().map_err(|_| "Local database is unavailable".to_string())?;
        connection
            .query_row(
                "SELECT path FROM media_directories WHERE id = ?1 AND is_active = 1",
                [directory_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|_| "Could not load the selected folder".to_string())?
            .ok_or_else(|| "This folder is no longer in your library".to_string())?
    };
    let root = PathBuf::from(root);
    let progress_app = app.clone();
    let scan_database = Arc::clone(&database);
    let scan_cancel = Arc::clone(&cancel);
    let result = tauri::async_runtime::spawn_blocking(move || {
        let progress_app_for_callback = progress_app.clone();
        scan_directory(
            &scan_database,
            directory_id,
            root,
            &scan_cancel,
            move |progress| {
                let _ = progress_app_for_callback.emit("media-scan-progress", progress);
            },
        )
    })
    .await
    .map_err(|_| "Folder scanning stopped unexpectedly".to_string())??;
    if !result.cancelled {
        let _ = enqueue_pending_thumbnails(app, database, thumbnails.inner().clone(), directory_id);
    }
    Ok(result)
}

#[tauri::command]
fn cancel_media_scan(scans: State<'_, ScanRegistry>, directory_id: i64) -> Result<bool, String> {
    let active = scans.0.lock().map_err(|_| "Could not cancel folder scanning".to_string())?;
    if let Some(cancel) = active.get(&directory_id) {
        cancel.store(true, Ordering::Relaxed);
        Ok(true)
    } else {
        Ok(false)
    }
}

#[tauri::command]
fn import_playlist_files(
    database: State<'_, AppDatabase>,
    paths: Vec<String>,
) -> Result<PlaylistFileImportSummary, String> {
    let mut candidates = Vec::new();
    let mut invalid = 0;
    {
        let connection = database.0.lock().map_err(|_| "Local database is unavailable".to_string())?;
        for raw in &paths {
            let path = canonicalize_media_path(Path::new(raw)).unwrap_or_else(|_| PathBuf::from(raw));
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if PlaylistFileKind::from_extension(&extension).is_none() || !path.is_file() {
                invalid += 1;
                continue;
            }
            let (size_bytes, modified_at, filename) = match file_metadata(&path) {
                Ok(metadata) => metadata,
                Err(_) => {
                    invalid += 1;
                    continue;
                }
            };
            let directory_id = directory_id_for_playlist_path(&connection, &path)?;
            candidates.push(PlaylistFileCandidate {
                path,
                filename,
                extension,
                size_bytes,
                modified_at,
                directory_id,
            });
        }
    }
    let mut summary =
        import_playlist_file_candidates(&database.0, &candidates, &AtomicBool::new(false))?;
    summary.files_seen = paths.len();
    summary.failed += invalid;
    Ok(summary)
}

#[tauri::command]
fn export_playlist_file(
    orchestrator: State<'_, PlaybackOrchestrator>,
    playlist_id: i64,
    path: String,
) -> Result<PlaylistFileExportResult, String> {
    let name = orchestrator
        .engine()
        .list_playlists()?
        .into_iter()
        .find(|playlist| playlist.id == playlist_id)
        .map(|playlist| playlist.name)
        .ok_or_else(|| "That playlist no longer exists.".to_string())?;
    let items = orchestrator.engine().export_items(playlist_id)?;
    let output = PathBuf::from(&path);
    let extension = output
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let kind = PlaylistFileKind::from_extension(&extension)
        .ok_or_else(|| "Choose an M3U, M3U8, or WPL file name.".to_string())?;
    let output_directory = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let export_items = items
        .into_iter()
        .map(|item| ExportItem {
            path: PathBuf::from(item.path),
            filename: item.filename,
            duration_seconds: item.duration_seconds,
        })
        .collect::<Vec<_>>();

    let content = match kind {
        PlaylistFileKind::M3u | PlaylistFileKind::M3u8 => {
            export_m3u(&name, &export_items, output_directory)
        }
        PlaylistFileKind::Wpl => export_wpl(&name, &export_items, output_directory),
    };
    // Windows Media Player expects UTF-16LE for WPL; M3U/M3U8 stay UTF-8.
    let bytes = if kind == PlaylistFileKind::Wpl {
        let mut bytes = vec![0xff, 0xfe];
        for unit in content.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    } else {
        content.into_bytes()
    };
    fs::write(&output, bytes).map_err(|_| "Could not save the playlist file.".to_string())?;
    Ok(PlaylistFileExportResult {
        items_written: export_items.len(),
        format: kind.extension().to_string(),
    })
}

#[derive(Clone)]
struct ExistingMediaFile {
    id: i64,
    path: String,
    size_bytes: i64,
    modified_at: String,
}

fn scan_directory(
    database: &Arc<Mutex<Connection>>,
    directory_id: i64,
    root: PathBuf,
    cancel: &AtomicBool,
    mut emit_progress: impl FnMut(ScanProgress),
) -> Result<ScanResult, String> {
    let canonical_root = match canonicalize_media_path(&root) {
        Ok(path) if path.is_dir() => path,
        _ => {
            mark_directory_temporarily_unavailable(database, directory_id)?;
            emit_progress(ScanProgress {
                directory_id,
                files_found: 0,
                inaccessible_entries: 0,
                phase: "temporarily_unavailable".to_string(),
            });
            return Err("Folder is temporarily unavailable. Existing library and playlist entries were kept.".to_string());
        }
    };
    let mut indexed_files = Vec::new();
    let mut playlist_files = Vec::new();
    let mut inaccessible_entries = 0;
    emit_progress(ScanProgress {
        directory_id,
        files_found: 0,
        inaccessible_entries: 0,
        phase: "scanning".to_string(),
    });

    for entry in WalkDir::new(&canonical_root).follow_links(false).into_iter() {
        if cancel.load(Ordering::Relaxed) {
            emit_progress(ScanProgress {
                directory_id,
                files_found: indexed_files.len(),
                inaccessible_entries,
                phase: "cancelled".to_string(),
            });
            return Ok(cancelled_scan_result(directory_id, indexed_files.len(), inaccessible_entries));
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                inaccessible_entries += 1;
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let extension = match path.extension().and_then(|value| value.to_str()) {
            Some(extension) => extension.to_ascii_lowercase(),
            None => continue,
        };
        let is_video = VIDEO_EXTENSIONS.iter().any(|known| extension == *known);
        let is_playlist = PLAYLIST_EXTENSIONS.iter().any(|known| extension == *known);
        if !is_video && !is_playlist {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => {
                inaccessible_entries += 1;
                continue;
            }
        };
        let modified_at = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos().to_string())
            .unwrap_or_else(|| "0".to_string());
        let filename = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let size_bytes = match i64::try_from(metadata.len()) {
            Ok(size_bytes) => size_bytes,
            Err(_) => {
                inaccessible_entries += 1;
                continue;
            }
        };
        if is_video {
            indexed_files.push(IndexedFile {
                path: path.to_string_lossy().into_owned(),
                filename,
                extension,
                size_bytes,
                modified_at,
            });
        } else {
            playlist_files.push(PlaylistFileCandidate {
                path: path.to_path_buf(),
                filename,
                extension,
                size_bytes,
                modified_at,
                directory_id: Some(directory_id),
            });
        }
        if indexed_files.len() % 25 == 0 {
            emit_progress(ScanProgress {
                directory_id,
                files_found: indexed_files.len(),
                inaccessible_entries,
                phase: "scanning".to_string(),
            });
        }
    }
    if cancel.load(Ordering::Relaxed) {
        emit_progress(ScanProgress {
            directory_id,
            files_found: indexed_files.len(),
            inaccessible_entries,
            phase: "cancelled".to_string(),
        });
        return Ok(cancelled_scan_result(directory_id, indexed_files.len(), inaccessible_entries));
    }

    let mut connection = database.lock().map_err(|_| "Local database is unavailable".to_string())?;
    let transaction = connection
        .transaction()
        .map_err(|_| "Could not update the media index".to_string())?;
    let mut existing_files = HashMap::new();
    {
        let mut statement = transaction
            .prepare("SELECT id, path, size_bytes, modified_at FROM media_files WHERE directory_id = ?1")
            .map_err(|_| "Could not read the current media index".to_string())?;
        let rows = statement
            .query_map([directory_id], |row| {
                Ok(ExistingMediaFile {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    size_bytes: row.get(2)?,
                    modified_at: row.get(3)?,
                })
            })
            .map_err(|_| "Could not read the current media index".to_string())?;
        for row in rows {
            let existing = row.map_err(|_| "Could not read the current media index".to_string())?;
            existing_files.insert(existing.path.clone(), existing);
        }
    }

    let incoming_paths = indexed_files.iter().map(|file| file.path.clone()).collect::<HashSet<_>>();
    let mut incoming_signatures = HashMap::<(i64, String), usize>::new();
    for file in &indexed_files {
        if !existing_files.contains_key(&file.path) {
            *incoming_signatures
                .entry((file.size_bytes, file.modified_at.clone()))
                .or_default() += 1;
        }
    }
    let mut old_signatures = HashMap::<(i64, String), Vec<ExistingMediaFile>>::new();
    for existing in existing_files
        .values()
        .filter(|file| !incoming_paths.contains(&file.path))
    {
        old_signatures
            .entry((existing.size_bytes, existing.modified_at.clone()))
            .or_default()
            .push(existing.clone());
    }

    let mut current_paths = HashSet::with_capacity(indexed_files.len());
    let mut renamed_files = 0;
    for file in &indexed_files {
        if cancel.load(Ordering::Relaxed) {
            return Ok(cancelled_scan_result(directory_id, indexed_files.len(), inaccessible_entries));
        }
        current_paths.insert(file.path.clone());
        let exact_match = existing_files.get(&file.path);
        let signature = (file.size_bytes, file.modified_at.clone());
        let rename_match = if exact_match.is_none() && incoming_signatures.get(&signature) == Some(&1) {
            old_signatures.get(&signature).filter(|matches| matches.len() == 1).map(|matches| matches[0].id)
        } else {
            None
        };
        if let Some(media_file_id) = rename_match {
            transaction
                .execute(
                    "UPDATE media_files SET path = ?1, filename = ?2, extension = ?3, directory_id = ?4,
                     size_bytes = ?5, modified_at = ?6, is_available = 1, availability_state = 'available'
                     WHERE id = ?7",
                    params![file.path, file.filename, file.extension, directory_id, file.size_bytes, file.modified_at, media_file_id],
                )
                .map_err(|_| "Could not update a renamed media file".to_string())?;
            renamed_files += 1;
            continue;
        }
        transaction
            .execute(
                "INSERT INTO media_files(directory_id, path, filename, extension, size_bytes, modified_at, is_available, availability_state)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, 'available')
                 ON CONFLICT(path) DO UPDATE SET
                   directory_id = excluded.directory_id,
                   filename = excluded.filename,
                   extension = excluded.extension,
                   size_bytes = excluded.size_bytes,
                   modified_at = excluded.modified_at,
                   is_available = 1,
                   availability_state = 'available'
                 WHERE media_files.directory_id != excluded.directory_id
                    OR media_files.filename != excluded.filename
                    OR media_files.extension != excluded.extension
                    OR media_files.size_bytes != excluded.size_bytes
                    OR media_files.modified_at != excluded.modified_at
                    OR media_files.is_available != 1
                    OR media_files.availability_state != 'available'",
                params![directory_id, file.path, file.filename, file.extension, file.size_bytes, file.modified_at],
            )
            .map_err(|_| "Could not update a media file in the library".to_string())?;
        let media_file_id = transaction
            .query_row("SELECT id FROM media_files WHERE path = ?1", [&file.path], |row| row.get::<_, i64>(0))
            .map_err(|_| "Could not load an indexed media file".to_string())?;
        transaction
            .execute(
                "INSERT INTO thumbnail_cache(media_file_id, source_size, source_modified_at, status)
                 VALUES (?1, ?2, ?3, 'pending')
                 ON CONFLICT(media_file_id) DO UPDATE SET
                   source_size = excluded.source_size,
                   source_modified_at = excluded.source_modified_at,
                   cache_path = NULL,
                   status = 'pending',
                   error_message = NULL,
                   updated_at = CURRENT_TIMESTAMP
                 WHERE thumbnail_cache.source_size != excluded.source_size
                    OR thumbnail_cache.source_modified_at != excluded.source_modified_at
                    OR thumbnail_cache.status = 'removed_from_library'",
                params![media_file_id, file.size_bytes, file.modified_at],
            )
            .map_err(|_| "Could not update thumbnail cache metadata".to_string())?;
    }

    if cancel.load(Ordering::Relaxed) {
        return Ok(cancelled_scan_result(directory_id, indexed_files.len(), inaccessible_entries));
    }
    let (files_removed, temporarily_unavailable_files, permanently_missing_files) =
        apply_missing_file_states(&transaction, directory_id, &current_paths, inaccessible_entries)?;
    if cancel.load(Ordering::Relaxed) {
        return Ok(cancelled_scan_result(directory_id, indexed_files.len(), inaccessible_entries));
    }
    let directory_state = if inaccessible_entries > 0 { "partially_unavailable" } else { "available" };
    transaction
        .execute(
            "UPDATE media_directories SET last_scanned_at = CURRENT_TIMESTAMP, availability_state = ?1 WHERE id = ?2 AND is_active = 1",
            params![directory_state, directory_id],
        )
        .map_err(|_| "Could not save scan completion".to_string())?;
    transaction.commit().map_err(|_| "Could not finish updating the media index".to_string())?;
    drop(connection);

    // Playlist files are imported only after the video transaction is durable,
    // so their references resolve against the freshly indexed media files.
    let imported = import_playlist_file_candidates(database, &playlist_files, cancel)?;

    let result = ScanResult {
        directory_id,
        files_indexed: indexed_files.len(),
        files_removed,
        renamed_files,
        temporarily_unavailable_files,
        permanently_missing_files,
        inaccessible_entries,
        playlists_imported: imported.playlists_created + imported.playlists_updated,
        cancelled: false,
    };
    emit_progress(ScanProgress {
        directory_id,
        files_found: result.files_indexed,
        inaccessible_entries,
        phase: "complete".to_string(),
    });
    Ok(result)
}

fn file_metadata(path: &Path) -> Result<(i64, String, String), String> {
    let metadata = fs::metadata(path).map_err(|_| "Could not read file metadata".to_string())?;
    let modified_at = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().to_string())
        .unwrap_or_else(|| "0".to_string());
    let size_bytes = i64::try_from(metadata.len())
        .map_err(|_| "This playlist file is too large to import".to_string())?;
    let filename = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok((size_bytes, modified_at, filename))
}

/// Uses the stored canonical form when possible and removes `.`/`..` segments
/// otherwise, so relative playlist references can be matched to indexed paths.
fn normalize_media_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = canonicalize_media_path(path) {
        return canonical;
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn media_index_key(path: &Path) -> String {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        text.replace('/', "\\").to_ascii_lowercase()
    } else {
        text.into_owned()
    }
}

fn playlist_exists(transaction: &Transaction<'_>, playlist_id: i64) -> Result<bool, String> {
    Ok(transaction
        .query_row("SELECT 1 FROM playlists WHERE id = ?1", [playlist_id], |_| Ok(()))
        .optional()
        .map_err(|error| error.to_string())?
        .is_some())
}

fn create_imported_playlist(transaction: &Transaction<'_>, requested_name: &str) -> Result<i64, String> {
    let name: String = requested_name.chars().take(120).collect();
    let name = if name.trim().is_empty() { "Imported playlist" } else { name.trim() };
    transaction
        .execute("INSERT INTO playlists(name) VALUES (?1)", [name])
        .map_err(|error| error.to_string())?;
    Ok(transaction.last_insert_rowid())
}

fn record_playlist_file_import(
    transaction: &Transaction<'_>,
    candidate: &PlaylistFileCandidate,
    playlist_id: Option<i64>,
    status: &str,
    error_message: Option<&str>,
) -> Result<(), String> {
    transaction
        .execute(
            "INSERT INTO playlist_file_imports(
                 path, directory_id, playlist_id, filename, extension, size_bytes, modified_at,
                 status, error_message
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(path) DO UPDATE SET
                 directory_id = excluded.directory_id,
                 playlist_id = excluded.playlist_id,
                 filename = excluded.filename,
                 extension = excluded.extension,
                 size_bytes = excluded.size_bytes,
                 modified_at = excluded.modified_at,
                 status = excluded.status,
                 error_message = excluded.error_message,
                 updated_at = CURRENT_TIMESTAMP",
            params![
                candidate.path.to_string_lossy().into_owned(),
                candidate.directory_id,
                playlist_id,
                candidate.filename,
                candidate.extension,
                candidate.size_bytes,
                candidate.modified_at,
                status,
                error_message,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Resolves only to media files that were already indexed under an active
/// folder. Remote references and unknown paths are ignored, never stored.
fn resolve_playlist_references(
    parsed: &ParsedPlaylistFile,
    playlist_path: &Path,
    roots: &[(PathBuf, String)],
    media_index: &HashMap<String, i64>,
) -> Vec<i64> {
    let parent = playlist_path.parent().unwrap_or_else(|| Path::new(""));
    let mut resolved = Vec::new();
    for entry in &parsed.entries {
        let Some(local) = reference_to_local_path(&entry.reference) else { continue };
        let mut candidates = Vec::new();
        if local.is_absolute() {
            candidates.push(local);
        } else {
            candidates.push(parent.join(&local));
            for (root, _) in roots {
                candidates.push(root.join(&local));
            }
        }
        for candidate in candidates {
            let key = media_index_key(&normalize_media_path(&candidate));
            if let Some(media_file_id) = media_index.get(&key) {
                if !resolved.contains(media_file_id) {
                    resolved.push(*media_file_id);
                }
                break;
            }
        }
    }
    resolved
}

fn directory_id_for_playlist_path(
    connection: &Connection,
    path: &Path,
) -> Result<Option<i64>, String> {
    let mut statement = connection
        .prepare("SELECT id, path FROM media_directories WHERE is_active = 1")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(statement);

    let file_key = media_index_key(path);
    let mut best: Option<(i64, usize)> = None;
    for (directory_id, root) in rows {
        let root_key = media_index_key(Path::new(&root));
        if !file_key.starts_with(&root_key) {
            continue;
        }
        let rest = &file_key[root_key.len()..];
        if !rest.is_empty() && !rest.starts_with('\\') && !rest.starts_with('/') {
            continue;
        }
        if best.map(|(_, length)| root_key.len() > length).unwrap_or(true) {
            best = Some((directory_id, root_key.len()));
        }
    }
    Ok(best.map(|(directory_id, _)| directory_id))
}

fn import_playlist_file_candidates(
    database: &Arc<Mutex<Connection>>,
    candidates: &[PlaylistFileCandidate],
    cancel: &AtomicBool,
) -> Result<PlaylistFileImportSummary, String> {
    let mut summary = PlaylistFileImportSummary::default();
    if candidates.is_empty() {
        return Ok(summary);
    }
    let connection = database
        .lock()
        .map_err(|_| "Local database is unavailable".to_string())?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|_| "Could not import playlist files".to_string())?;

    let roots = {
        let mut statement = transaction
            .prepare("SELECT path FROM media_directories WHERE is_active = 1")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows.into_iter().map(|path| (PathBuf::from(&path), path)).collect::<Vec<_>>()
    };

    let media_index = {
        let mut statement = transaction
            .prepare(
                "SELECT f.path, f.id FROM media_files f
                 JOIN media_directories d ON d.id = f.directory_id
                 WHERE d.is_active = 1 AND f.is_available = 1",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows.into_iter()
            .map(|(path, id)| (media_index_key(Path::new(&path)), id))
            .collect::<HashMap<_, _>>()
    };

    let existing = {
        let mut statement = transaction
            .prepare(
                "SELECT path, playlist_id, size_bytes, modified_at, status FROM playlist_file_imports",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    ExistingPlaylistFileImport {
                        playlist_id: row.get(1)?,
                        size_bytes: row.get(2)?,
                        modified_at: row.get(3)?,
                        status: row.get(4)?,
                    },
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows.into_iter().collect::<HashMap<_, _>>()
    };

    for candidate in candidates {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        summary.files_seen += 1;
        let path_key = candidate.path.to_string_lossy().into_owned();
        let existing_row = existing.get(&path_key).cloned();
        let unchanged = existing_row.as_ref().filter(|row| {
            row.size_bytes == candidate.size_bytes && row.modified_at == candidate.modified_at
        });

        // A tombstone is kept when a user deletes an imported playlist, so a
        // later rescan of the same file cannot resurrect it.
        if let Some(row) = unchanged {
            if row.status == "deleted" {
                summary.skipped += 1;
                continue;
            }
            if row.status == "imported" {
                let still_exists = row
                    .playlist_id
                    .map(|id| playlist_exists(&transaction, id))
                    .transpose()?
                    .unwrap_or(false);
                if still_exists {
                    summary.skipped += 1;
                    continue;
                }
            }
        }

        let parsed = match parse_playlist_file(&candidate.path) {
            Ok(parsed) => parsed,
            Err(_) => {
                record_playlist_file_import(
                    &transaction,
                    candidate,
                    None,
                    "error",
                    Some("This playlist file could not be read."),
                )?;
                summary.failed += 1;
                continue;
            }
        };
        let resolved = resolve_playlist_references(&parsed, &candidate.path, &roots, &media_index);
        if resolved.is_empty() {
            record_playlist_file_import(
                &transaction,
                candidate,
                None,
                "error",
                Some("No local files in this playlist are indexed yet."),
            )?;
            summary.failed += 1;
            continue;
        }

        let existing_playlist_id = existing_row
            .as_ref()
            .and_then(|row| row.playlist_id)
            .filter(|id| playlist_exists(&transaction, *id).unwrap_or(false));
        let playlist_id = if let Some(id) = existing_playlist_id {
            let name: String = parsed.name.clone().unwrap_or_else(|| "Imported playlist".to_string());
            transaction
                .execute(
                    "UPDATE playlists SET name = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
                    params![name.chars().take(120).collect::<String>(), id],
                )
                .map_err(|error| error.to_string())?;
            transaction
                .execute("DELETE FROM playlist_items WHERE playlist_id = ?1", [id])
                .map_err(|error| error.to_string())?;
            summary.playlists_updated += 1;
            id
        } else {
            let id = create_imported_playlist(
                &transaction,
                parsed.name.as_deref().unwrap_or("Imported playlist"),
            )?;
            summary.playlists_created += 1;
            id
        };

        let inserted =
            playlist::insert_playlist_items_in_transaction(&transaction, playlist_id, &resolved)?;
        summary.items_imported += inserted;
        record_playlist_file_import(
            &transaction,
            candidate,
            Some(playlist_id),
            "imported",
            None,
        )?;
    }

    transaction
        .commit()
        .map_err(|_| "Could not finish importing playlist files".to_string())?;
    Ok(summary)
}

fn missing_file_availability_state(inaccessible_entries: usize) -> &'static str {
    if inaccessible_entries > 0 { "temporarily_unavailable" } else { "permanently_missing" }
}

fn apply_missing_file_states(
    transaction: &Transaction<'_>,
    directory_id: i64,
    current_paths: &HashSet<String>,
    inaccessible_entries: usize,
) -> Result<(usize, usize, usize), String> {
    let mut statement = transaction
        .prepare("SELECT id, path, is_available FROM media_files WHERE directory_id = ?1")
        .map_err(|_| "Could not read the media index after scanning".to_string())?;
    let rows = statement
        .query_map([directory_id], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)? != 0)))
        .map_err(|_| "Could not read the media index after scanning".to_string())?;
    let missing_rows = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "Could not read the media index after scanning".to_string())?;
    drop(statement);
    let state = missing_file_availability_state(inaccessible_entries);
    let mut files_removed = 0;
    let mut temporarily_unavailable = 0;
    let mut permanently_missing = 0;
    for (media_file_id, path, was_available) in missing_rows {
        if current_paths.contains(&path) {
            continue;
        }
        if state == "temporarily_unavailable" {
            if was_available {
                temporarily_unavailable += 1;
            }
            transaction
                .execute("UPDATE media_files SET is_available = 0, availability_state = 'temporarily_unavailable' WHERE id = ?1", [media_file_id])
                .map_err(|_| "Could not preserve inaccessible media entries".to_string())?;
        } else {
            if was_available {
                files_removed += 1;
            }
            permanently_missing += 1;
            transaction
                .execute("UPDATE media_files SET is_available = 0, availability_state = 'permanently_missing' WHERE id = ?1", [media_file_id])
                .map_err(|_| "Could not mark missing media entries".to_string())?;
        }
    }
    Ok((files_removed, temporarily_unavailable, permanently_missing))
}

fn cancelled_scan_result(directory_id: i64, files_found: usize, inaccessible_entries: usize) -> ScanResult {
    ScanResult {
        directory_id,
        files_indexed: files_found,
        files_removed: 0,
        renamed_files: 0,
        temporarily_unavailable_files: 0,
        permanently_missing_files: 0,
        inaccessible_entries,
        playlists_imported: 0,
        cancelled: true,
    }
}

fn mark_directory_temporarily_unavailable(
    database: &Arc<Mutex<Connection>>,
    directory_id: i64,
) -> Result<(), String> {
    let mut connection = database.lock().map_err(|_| "Local database is unavailable".to_string())?;
    let transaction = connection
        .transaction()
        .map_err(|_| "Could not save folder availability".to_string())?;
    transaction
        .execute("UPDATE media_directories SET availability_state = 'temporarily_unavailable' WHERE id = ?1", [directory_id])
        .map_err(|_| "Could not save folder availability".to_string())?;
    transaction
        .execute("UPDATE media_files SET is_available = 0, availability_state = 'temporarily_unavailable' WHERE directory_id = ?1 AND is_available = 1", [directory_id])
        .map_err(|_| "Could not preserve offline media entries".to_string())?;
    transaction.commit().map_err(|_| "Could not finish saving folder availability".to_string())
}

#[tauri::command]
fn get_thumbnail_settings(database: State<'_, AppDatabase>) -> Result<ThumbnailSettings, String> {
    let connection = database.0.lock().map_err(|_| "Local database is unavailable".to_string())?;
    read_thumbnail_settings(&connection)
}

#[tauri::command]
fn set_thumbnail_settings(
    app: tauri::AppHandle,
    database: State<'_, AppDatabase>,
    thumbnails: State<'_, ThumbnailService>,
    width: u32,
    height: u32,
    quality: u8,
) -> Result<ThumbnailSettings, String> {
    if !(64..=1024).contains(&width) || !(64..=1024).contains(&height) || !(40..=95).contains(&quality) {
        return Err("Thumbnail dimensions must be 64–1024 px and quality 40–95".to_string());
    }
    let connection = Arc::clone(&database.0);
    let directories = {
        let mut connection = connection.lock().map_err(|_| "Local database is unavailable".to_string())?;
        let transaction = connection.transaction().map_err(|_| "Could not save thumbnail settings".to_string())?;
        for (key, value) in [
            ("thumbnail_width", width.to_string()),
            ("thumbnail_height", height.to_string()),
            ("thumbnail_quality", quality.to_string()),
        ] {
            transaction
                .execute(
                    "INSERT INTO application_settings(key, value) VALUES (?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
                    params![key, value],
                )
                .map_err(|_| "Could not save thumbnail settings".to_string())?;
        }
        transaction
            .execute(
                "UPDATE thumbnail_cache SET cache_path = NULL, status = 'pending', error_message = NULL,
                 updated_at = CURRENT_TIMESTAMP WHERE media_file_id IN
                 (SELECT f.id FROM media_files f JOIN media_directories d ON d.id = f.directory_id
                  WHERE f.is_available = 1 AND d.is_active = 1)",
                [],
            )
            .map_err(|_| "Could not invalidate old thumbnail sizes".to_string())?;
        let rows = {
            let mut statement = transaction
                .prepare("SELECT id FROM media_directories WHERE is_active = 1")
                .map_err(|_| "Could not load media folders".to_string())?;
            let rows = statement
                .query_map([], |row| row.get::<_, i64>(0))
                .map_err(|_| "Could not load media folders".to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|_| "Could not load media folders".to_string())?
        };
        transaction.commit().map_err(|_| "Could not save thumbnail settings".to_string())?;
        rows
    };
    for directory_id in directories {
        let _ = enqueue_pending_thumbnails(app.clone(), Arc::clone(&connection), thumbnails.inner().clone(), directory_id);
    }
    Ok(ThumbnailSettings { width, height, quality })
}

#[tauri::command]
fn refresh_thumbnail(
    app: tauri::AppHandle,
    database: State<'_, AppDatabase>,
    thumbnails: State<'_, ThumbnailService>,
    media_file_id: i64,
) -> Result<(), String> {
    let connection = Arc::clone(&database.0);
    {
        let connection = connection.lock().map_err(|_| "Local database is unavailable".to_string())?;
        connection
            .execute(
                "UPDATE thumbnail_cache SET status = 'pending', cache_path = NULL, error_message = NULL,
                 updated_at = CURRENT_TIMESTAMP WHERE media_file_id = ?1",
                [media_file_id],
            )
            .map_err(|_| "Could not queue thumbnail regeneration".to_string())?;
        if connection.changes() == 0 {
            let exists = connection
                .query_row("SELECT 1 FROM media_files WHERE id = ?1 AND is_available = 1", [media_file_id], |_| Ok(()))
                .optional()
                .map_err(|_| "Could not check the media file".to_string())?
                .is_some();
            if !exists {
                return Err("This file is unavailable. Rescan its folder before regenerating the thumbnail.".to_string());
            }
            connection
                .execute(
                    "INSERT INTO thumbnail_cache(media_file_id, source_size, source_modified_at, status)
                     SELECT id, size_bytes, modified_at, 'pending' FROM media_files WHERE id = ?1",
                    [media_file_id],
                )
                .map_err(|_| "Could not queue thumbnail regeneration".to_string())?;
        }
    }
    let directory_id = {
        let connection = connection.lock().map_err(|_| "Local database is unavailable".to_string())?;
        connection
            .query_row("SELECT directory_id FROM media_files WHERE id = ?1", [media_file_id], |row| row.get::<_, i64>(0))
            .map_err(|_| "This media file is no longer in the library".to_string())?
    };
    enqueue_pending_thumbnails(app, connection, thumbnails.inner().clone(), directory_id)
}

#[tauri::command]
fn prepare_thumbnail_cache(
    app: tauri::AppHandle,
    database: State<'_, AppDatabase>,
    thumbnails: State<'_, ThumbnailService>,
) -> Result<(), String> {
    let connection = Arc::clone(&database.0);
    let directories = {
        let connection = connection.lock().map_err(|_| "Local database is unavailable".to_string())?;
        let ready_paths = {
            let mut statement = connection
                .prepare("SELECT media_file_id, cache_path FROM thumbnail_cache WHERE status = 'ready'")
                .map_err(|_| "Could not inspect the thumbnail cache".to_string())?;
            let rows = statement
                .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)))
                .map_err(|_| "Could not inspect the thumbnail cache".to_string())?;
            rows.collect::<Result<Vec<_>, _>>().map_err(|_| "Could not inspect the thumbnail cache".to_string())?
        };
        let mut connection = connection;
        let transaction = connection.transaction().map_err(|_| "Could not repair the thumbnail cache".to_string())?;
        for (media_file_id, path) in ready_paths {
            if path.as_deref().map_or(true, |path| !Path::new(path).is_file()) {
                transaction
                    .execute("UPDATE thumbnail_cache SET status = 'pending', cache_path = NULL, error_message = NULL WHERE media_file_id = ?1", [media_file_id])
                    .map_err(|_| "Could not repair a missing thumbnail".to_string())?;
            }
        }
        let rows = {
            let mut statement = transaction
                .prepare("SELECT id FROM media_directories WHERE is_active = 1")
                .map_err(|_| "Could not load media folders".to_string())?;
            let rows = statement
                .query_map([], |row| row.get::<_, i64>(0))
                .map_err(|_| "Could not load media folders".to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|_| "Could not load media folders".to_string())?
        };
        transaction.commit().map_err(|_| "Could not repair the thumbnail cache".to_string())?;
        rows
    };
    for directory_id in directories {
        let _ = enqueue_pending_thumbnails(app.clone(), Arc::clone(&connection), thumbnails.inner().clone(), directory_id);
    }
    Ok(())
}

#[tauri::command]
fn cleanup_thumbnail_cache(
    database: State<'_, AppDatabase>,
    thumbnails: State<'_, ThumbnailService>,
) -> Result<CacheCleanupResult, String> {
    let connection = database.0.lock().map_err(|_| "Local database is unavailable".to_string())?;
    let mut statement = connection
        .prepare("SELECT cache_path FROM thumbnail_cache WHERE status = 'ready' AND cache_path IS NOT NULL")
        .map_err(|_| "Could not index thumbnail cache files".to_string())?;
    let paths = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| "Could not index thumbnail cache files".to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "Could not index thumbnail cache files".to_string())?
        .into_iter()
        .map(PathBuf::from)
        .collect::<HashSet<_>>();
    let result = thumbnail::cleanup_cache(&thumbnails.cache_dir, &paths)?;
    Ok(CacheCleanupResult {
        files_removed: result.files_removed,
        bytes_removed: result.bytes_removed,
    })
}

fn read_thumbnail_settings(connection: &Connection) -> Result<ThumbnailSettings, String> {
    let read = |key: &str, default: u32| -> Result<u32, String> {
        let value = connection
            .query_row("SELECT value FROM application_settings WHERE key = ?1", [key], |row| row.get::<_, String>(0))
            .optional()
            .map_err(|_| "Could not read thumbnail settings".to_string())?;
        Ok(value.and_then(|value| value.parse::<u32>().ok()).unwrap_or(default))
    };
    Ok(ThumbnailSettings {
        width: read("thumbnail_width", 320)?.clamp(64, 1024),
        height: read("thumbnail_height", 180)?.clamp(64, 1024),
        quality: read("thumbnail_quality", 82)?.clamp(40, 95) as u8,
    })
}

fn enqueue_pending_thumbnails(
    app: tauri::AppHandle,
    database: Arc<Mutex<Connection>>,
    service: ThumbnailService,
    directory_id: i64,
) -> Result<(), String> {
    let media_ids = {
        let connection = database.lock().map_err(|_| "Local database is unavailable".to_string())?;
        let mut statement = connection
            .prepare(
                "SELECT f.id FROM media_files f
                 JOIN media_directories d ON d.id = f.directory_id AND d.is_active = 1
                 LEFT JOIN thumbnail_cache t ON t.media_file_id = f.id
                 WHERE f.directory_id = ?1 AND f.is_available = 1 AND COALESCE(t.status, 'pending') = 'pending'
                 ORDER BY f.id",
            )
            .map_err(|_| "Could not queue thumbnails".to_string())?;
        let rows = statement
            .query_map([directory_id], |row| row.get::<_, i64>(0))
            .map_err(|_| "Could not queue thumbnails".to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "Could not queue thumbnails".to_string())?;
        rows
    };
    if media_ids.is_empty() {
        return Ok(());
    }
    let should_start = {
        let mut queue = service.queue.lock().map_err(|_| "Could not queue thumbnails".to_string())?;
        for media_file_id in media_ids {
            if queue.queued.insert(media_file_id) {
                queue.pending.push_back(media_file_id);
            } else if queue.active == Some(media_file_id) {
                queue.rerun.insert(media_file_id);
            }
        }
        if !queue.running && !queue.pending.is_empty() {
            queue.running = true;
            true
        } else {
            false
        }
    };
    if should_start {
        tauri::async_runtime::spawn_blocking(move || run_thumbnail_worker(app, database, service));
    }
    Ok(())
}

fn run_thumbnail_worker(app: tauri::AppHandle, database: Arc<Mutex<Connection>>, service: ThumbnailService) {
    loop {
        let next_id = match service.queue.lock() {
            Ok(mut queue) => match queue.pending.pop_front() {
                Some(media_file_id) => {
                    queue.active = Some(media_file_id);
                    Some(media_file_id)
                }
                None => {
                    queue.active = None;
                    queue.running = false;
                    None
                }
            },
            Err(_) => None,
        };
        let Some(media_file_id) = next_id else { return };
        let result = generate_thumbnail_for_file(&database, &service.cache_dir, media_file_id);
        let (status, error) = match result {
            Ok(()) => ("ready".to_string(), None),
            Err(error) => {
                mark_thumbnail_failed(&database, media_file_id, &error);
                ("failed".to_string(), Some(error))
            }
        };
        let _ = app.emit(
            "thumbnail-generation-progress",
            ThumbnailProgress {
                media_file_id,
                status,
                error,
            },
        );
        if let Ok(mut queue) = service.queue.lock() {
            queue.active = None;
            if queue.rerun.remove(&media_file_id) {
                queue.pending.push_back(media_file_id);
            } else {
                queue.queued.remove(&media_file_id);
            }
        }
    }
}

fn generate_thumbnail_for_file(
    database: &Arc<Mutex<Connection>>,
    cache_dir: &Path,
    media_file_id: i64,
) -> Result<(), String> {
    let (path, source_size, source_modified_at, settings) = {
        let connection = database.lock().map_err(|_| "Local database is unavailable".to_string())?;
        connection
            .execute(
                "INSERT OR IGNORE INTO thumbnail_cache(media_file_id, source_size, source_modified_at, status)
                 SELECT id, size_bytes, modified_at, 'pending' FROM media_files WHERE id = ?1",
                [media_file_id],
            )
            .map_err(|_| "Could not initialize thumbnail metadata".to_string())?;
        let (path, size, modified) = connection
            .query_row(
                "SELECT f.path, f.size_bytes, f.modified_at FROM media_files f
                 JOIN media_directories d ON d.id = f.directory_id
                 WHERE f.id = ?1 AND f.is_available = 1 AND d.is_active = 1",
                [media_file_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)),
            )
            .optional()
            .map_err(|_| "Could not load video metadata".to_string())?
            .ok_or_else(|| "Video is unavailable in the library".to_string())?;
        (path, size, modified, read_thumbnail_settings(&connection)?)
    };
    let path = PathBuf::from(path);
    let metadata = fs::metadata(&path).map_err(|_| "Video is unavailable. Rescan its folder to update the library.".to_string())?;
    if i64::try_from(metadata.len()).ok() != Some(source_size) || file_modified_ns(&metadata) != source_modified_at {
        return Err("Video changed after its last scan. Rescan the folder before generating a thumbnail.".to_string());
    }
    let jpeg = thumbnail::generate_jpeg(
        &path,
        thumbnail::ThumbnailOptions {
            width: settings.width,
            height: settings.height,
            quality: settings.quality,
        },
    )?;
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let file_name = format!(
        "video-{media_file_id}-{source_size}-{}-{}x{}-q{}-{nonce}.jpg",
        source_modified_at, settings.width, settings.height, settings.quality
    );
    let cache_path = thumbnail::write_cache_file(cache_dir, &file_name, &jpeg)?;
    let current_metadata = fs::metadata(&path).map_err(|_| "Video became unavailable during thumbnail extraction".to_string())?;
    if i64::try_from(current_metadata.len()).ok() != Some(source_size) || file_modified_ns(&current_metadata) != source_modified_at {
        let _ = fs::remove_file(cache_path);
        return Err("Video changed during thumbnail extraction. Rescan the folder and try again.".to_string());
    }
    let cache_path = cache_path.to_string_lossy().into_owned();
    let connection = database.lock().map_err(|_| "Local database is unavailable".to_string())?;
    connection
        .execute(
            "UPDATE thumbnail_cache SET source_size = ?1, source_modified_at = ?2, cache_path = ?3,
             width = ?4, height = ?5, quality = ?6, status = 'ready', error_message = NULL,
             updated_at = CURRENT_TIMESTAMP WHERE media_file_id = ?7",
            params![source_size, source_modified_at, cache_path, settings.width, settings.height, settings.quality, media_file_id],
        )
        .map_err(|_| "Could not save thumbnail metadata".to_string())?;
    Ok(())
}

fn file_modified_ns(metadata: &fs::Metadata) -> String {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().to_string())
        .unwrap_or_else(|| "0".to_string())
}

fn mark_thumbnail_failed(database: &Arc<Mutex<Connection>>, media_file_id: i64, error: &str) {
    if let Ok(connection) = database.lock() {
        let _ = connection.execute(
            "UPDATE thumbnail_cache SET status = 'failed', error_message = ?1, updated_at = CURRENT_TIMESTAMP WHERE media_file_id = ?2",
            params![error, media_file_id],
        );
    }
}

#[tauri::command]
async fn get_media_server_status(server: State<'_, MediaServerManager>) -> Result<MediaServerStatus, String> {
    Ok(server.status_snapshot().await)
}

#[tauri::command]
fn get_media_pairing_token(server: State<'_, MediaServerManager>) -> String {
    server.pairing_token()
}

#[tauri::command]
async fn start_media_server(server: State<'_, MediaServerManager>) -> Result<MediaServerStatus, String> {
    server.start().await
}

#[tauri::command]
async fn stop_media_server(server: State<'_, MediaServerManager>) -> Result<MediaServerStatus, String> {
    Ok(server.stop().await)
}

#[tauri::command]
async fn restart_media_server(server: State<'_, MediaServerManager>) -> Result<MediaServerStatus, String> {
    server.restart().await
}

#[tauri::command]
async fn set_media_server_port(
    server: State<'_, MediaServerManager>,
    port: u16,
) -> Result<MediaServerStatus, String> {
    server.set_port(port).await
}

#[tauri::command]
async fn get_media_network_diagnostics() -> Result<NetworkDiagnostics, String> {
    tokio::task::spawn_blocking(network::detect_network)
        .await
        .map_err(|_| "Could not inspect network interfaces".to_string())
}

#[tauri::command]
async fn test_media_server_connection(server: State<'_, MediaServerManager>) -> Result<ConnectionTestResult, String> {
    Ok(server.test_connection().await)
}

#[tauri::command]
fn get_media_stream_diagnostics(server: State<'_, MediaServerManager>) -> StreamDiagnosticsSnapshot {
    server.stream_diagnostics()
}

#[tauri::command]
fn clear_media_stream_diagnostics(server: State<'_, MediaServerManager>) {
    server.clear_stream_diagnostics();
}

/// Real, on-demand mDNS discovery for the in-app Connect panel. This does not
/// talk to the sender page and never returns credentials or streaming URLs.
#[tauri::command]
async fn discover_cast_devices() -> Result<Vec<CastDevice>, String> {
    tokio::task::spawn_blocking(|| cast_discovery::discover(Duration::from_secs(4)))
        .await
        .map_err(|_| "Cast discovery could not start.".to_string())?
}

#[tauri::command]
fn get_cast_sender_status(cast: State<'_, CastSenderManager>) -> CastSenderStatus {
    cast.status()
}

#[tauri::command]
async fn launch_cast_sender(
    window: tauri::Window,
    cast: State<'_, CastSenderManager>,
) -> Result<CastSenderStatus, String> {
    // Only an explicit user action may undo an intentional hide. Automatic
    // playback calls launch() directly and must keep the panel hidden.
    cast.clear_relaunch_suppression();
    let status = cast.launch().await?;
    // Every launch path (player bar, wizard, Cast panel) asks the app shell to
    // show the in-window sender panel so the Chrome window is never left alone.
    let _ = window.emit("cast-sender-launch-requested", ());
    Ok(status)
}

#[cfg(windows)]
fn app_window_handle(window: &tauri::Window) -> Result<isize, String> {
    window.hwnd().map(|handle| handle.0 as isize).map_err(|error| error.to_string())
}

#[cfg(not(windows))]
fn app_window_handle(_window: &tauri::Window) -> Result<isize, String> {
    Err("Embedding the Cast sender is available on Windows only.".to_string())
}

/// Re-parents the managed Chrome sender window into the app's sender panel.
/// Looking up and resizing Chrome's window can take seconds, so it must run on
/// a blocking thread: running it on the UI thread made the app "Not Responding".
#[tauri::command]
async fn embed_cast_sender(
    window: tauri::Window,
    cast: State<'_, CastSenderManager>,
    rect: sender_embed::HostRect,
) -> Result<bool, String> {
    let handle = app_window_handle(&window)?;
    let pid = cast.sender_pid();
    cast.clear_relaunch_suppression();
    tokio::task::spawn_blocking(move || {
        sender_embed::embed(handle, pid, rect)?;
        Ok(sender_embed::embedded_window_is_valid())
    })
    .await
    .map_err(|error| format!("Could not embed the Cast sender ({error})."))?
}

#[tauri::command]
async fn show_cast_sender(window: tauri::Window, rect: sender_embed::HostRect) -> Result<bool, String> {
    let handle = app_window_handle(&window)?;
    tokio::task::spawn_blocking(move || {
        sender_embed::show_embedded(handle, rect)?;
        Ok(sender_embed::embedded_window_is_valid())
    })
    .await
    .map_err(|error| format!("Could not show the Cast sender ({error})."))?
}

#[tauri::command]
async fn move_cast_sender(window: tauri::Window, rect: sender_embed::HostRect) -> Result<(), String> {
    let handle = app_window_handle(&window)?;
    tokio::task::spawn_blocking(move || sender_embed::move_embedded(handle, rect))
        .await
        .map_err(|error| format!("Could not reposition the Cast sender ({error})."))?
}

#[tauri::command]
async fn hide_cast_sender(cast: State<'_, CastSenderManager>) -> Result<(), String> {
    // The panel was hidden on purpose (receiver chosen, or user clicked X).
    // Suppress crash-relaunch so a floating Chrome window never reappears.
    cast.suppress_relaunch();
    tokio::task::spawn_blocking(sender_embed::hide_embedded)
        .await
        .map_err(|error| format!("Could not hide the Cast sender ({error})."))?
}

#[tauri::command]
async fn stop_cast_sender(cast: State<'_, CastSenderManager>) -> Result<CastSenderStatus, String> {
    // Ask the receiver to end the session before the bridge and Chrome die.
    // Best-effort: a disconnected sender must not block shutdown.
    let _ = cast.stop_playback();
    cast.shutdown().await;
    Ok(cast.status())
}

#[tauri::command]
async fn get_cast_diagnostics(
    server: State<'_, MediaServerManager>,
    cast: State<'_, CastSenderManager>,
) -> Result<CastDiagnosticsSnapshot, String> {
    let active_grants = server.cast_grants().active_count();
    Ok(cast.diagnostics(active_grants))
}

#[tauri::command]
fn get_cast_playback_state(
    database: State<'_, AppDatabase>,
    cast: State<'_, CastSenderManager>,
) -> Result<CastPlaybackSnapshot, String> {
    let mut snapshot = cast.snapshot();
    if snapshot.media_file_id.is_none() {
        let connection = database.0.lock().map_err(|_| "Local database is unavailable".to_string())?;
        if let Ok(Some((media_file_id, filename))) = cast_sender::read_cast_current_media(&connection) {
            snapshot.media_file_id = Some(media_file_id);
            snapshot.filename = Some(filename);
        }
    }
    Ok(snapshot)
}

/// Casts a library video to the connected receiver: makes sure the media
/// server and the Chrome sender are running, then issues a short-lived,
/// media-scoped streaming URL and hands it to the sender page. The receiver
/// fetches the file directly from this PC over LAN.
#[tauri::command]
async fn cast_load_media(
    server: State<'_, MediaServerManager>,
    cast: State<'_, CastSenderManager>,
    media_file_id: i64,
) -> Result<CastLoadOutcome, String> {
    if media_file_id <= 0 {
        return Err("Choose a video from your library to cast".to_string());
    }
    let snapshot = server.status_snapshot().await;
    if snapshot.state != "running" {
        let started = server.start().await?;
        if started.state != "running" {
            return Err(
                "The media server could not start for Cast playback. Check the network settings and try again.".to_string(),
            );
        }
    }
    cast.ensure_for_playback().await?;
    let scoped = server.create_cast_stream_url(media_file_id).await?;
    let probe = server.verify_cast_stream_url(media_file_id).await;
    let probe_ok = probe.reachable;
    let probe_message = probe.message.clone();
    cast.set_media_url_probe(probe);
    if !probe_ok {
        return Err(format!("The receiver would not be able to reach this video: {probe_message}"));
    }
    cast.set_pending_load(media_file_id, scoped.filename.clone());
    cast.send_command(serde_json::json!({
        "type": "load_media",
        "url": scoped.url,
        "mime_type": scoped.mime_type,
        "title": scoped.filename,
        "position_seconds": 0,
    }))?;
    Ok(CastLoadOutcome {
        media_file_id,
        filename: scoped.filename,
        mime_type: scoped.mime_type,
        expires_in_seconds: scoped.expires_in_seconds,
    })
}

fn sanitize_cast_field(value: String, max_chars: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_string()
}

#[tauri::command]
async fn cast_send_control(
    server: State<'_, MediaServerManager>,
    cast: State<'_, CastSenderManager>,
    action: String,
    position_seconds: Option<f64>,
    volume: Option<f64>,
    muted: Option<bool>,
    device_id: Option<String>,
    device_name: Option<String>,
) -> Result<(), String> {
    if !matches!(
        action.as_str(),
        "play" | "pause" | "toggle_play" | "stop" | "seek" | "set_volume" | "set_mute"
            | "disconnect" | "request_session" | "close_sender"
    ) {
        return Err("Unknown Cast control".to_string());
    }
    let command = serde_json::json!({
        "type": "control",
        "action": action,
        "position_seconds": position_seconds,
        "volume": volume,
        "muted": muted,
        "device_id": device_id.map(|value| sanitize_cast_field(value, 120)),
        "device_name": device_name.map(|value| sanitize_cast_field(value, 120)),
    });
    let result = cast.send_command(command);
    if action == "stop" {
        // Stopping ends playback: drop the streaming grant so the receiver
        // cannot keep fetching the file.
        let current = cast.snapshot().media_file_id;
        if let Some(media_file_id) = current {
            server.cast_grants().revoke_media(media_file_id);
        }
    }
    result
}

/// Probes the exact URL a receiver would fetch for this media id (with a
/// dedicated, immediately revoked grant) and stores the result in the Cast
/// diagnostics. The endpoint is reported without its credential.
#[tauri::command]
async fn verify_cast_media_url(
    server: State<'_, MediaServerManager>,
    cast: State<'_, CastSenderManager>,
    media_file_id: i64,
) -> Result<ConnectionTestResult, String> {
    if media_file_id <= 0 {
        return Err("Choose a video from your library to verify".to_string());
    }
    let result = server.verify_cast_stream_url(media_file_id).await;
    cast.set_media_url_probe(result.clone());
    Ok(result)
}

// ---------------------------------------------------------------------------
// Phase 5 — playlist / queue / state machine commands.
// Every command routes through the Rust orchestrator; the frontend never
// mutates playback state directly.
// ---------------------------------------------------------------------------

fn phase5_add_mode(value: &str) -> AddMode {
    match value {
        "play_next" => AddMode::PlayNext,
        "play_now" => AddMode::PlayNow,
        _ => AddMode::Append,
    }
}

async fn phase5_finish(
    orchestrator: &PlaybackOrchestrator,
    effects: Vec<EngineEffect>,
) -> playlist::PlaybackSnapshot {
    orchestrator.emit_snapshot();
    orchestrator.execute_effects(effects).await;
    let snapshot = orchestrator.snapshot();
    orchestrator.emit_snapshot();
    snapshot
}

#[tauri::command]
fn phase5_get_state(orchestrator: State<'_, PlaybackOrchestrator>) -> playlist::PlaybackSnapshot {
    orchestrator.snapshot()
}

/// Clears watch history only. Queue, playlists and indexed media are untouched.
#[tauri::command]
fn phase5_clear_watch_history(
    orchestrator: State<'_, PlaybackOrchestrator>,
) -> Result<playlist::PlaybackSnapshot, String> {
    orchestrator.clear_watch_history()?;
    orchestrator.emit_snapshot();
    Ok(orchestrator.snapshot())
}

// Playlists ---------------------------------------------------------------

#[tauri::command]
fn phase5_list_playlists(
    orchestrator: State<'_, PlaybackOrchestrator>,
) -> Result<Vec<PlaylistSummary>, String> {
    orchestrator.engine().list_playlists()
}

#[tauri::command]
fn phase5_create_playlist(
    orchestrator: State<'_, PlaybackOrchestrator>,
    name: String,
) -> Result<i64, String> {
    orchestrator.engine().create_playlist(name)
}

#[tauri::command]
fn phase5_rename_playlist(
    orchestrator: State<'_, PlaybackOrchestrator>,
    playlist_id: i64,
    name: String,
) -> Result<(), String> {
    orchestrator.engine().rename_playlist(playlist_id, name)
}

#[tauri::command]
fn phase5_delete_playlist(
    orchestrator: State<'_, PlaybackOrchestrator>,
    playlist_id: i64,
) -> Result<(), String> {
    orchestrator.engine().delete_playlist(playlist_id)
}

#[tauri::command]
fn phase5_select_playlist(
    orchestrator: State<'_, PlaybackOrchestrator>,
    playlist_id: i64,
) -> Result<(), String> {
    orchestrator.engine().select_playlist(playlist_id)
}

#[tauri::command]
fn phase5_list_playlist_items(
    orchestrator: State<'_, PlaybackOrchestrator>,
    playlist_id: i64,
) -> Result<Vec<PlaylistItem>, String> {
    orchestrator.engine().list_playlist_items(playlist_id)
}

#[tauri::command]
fn phase5_add_playlist_items(
    orchestrator: State<'_, PlaybackOrchestrator>,
    playlist_id: i64,
    media_file_ids: Vec<i64>,
) -> Result<usize, String> {
    orchestrator.engine().add_playlist_items(playlist_id, &media_file_ids)
}

#[tauri::command]
fn phase5_remove_playlist_item(
    orchestrator: State<'_, PlaybackOrchestrator>,
    item_id: i64,
) -> Result<(), String> {
    orchestrator.engine().remove_playlist_item(item_id)
}

#[tauri::command]
fn phase5_reorder_playlist_item(
    orchestrator: State<'_, PlaybackOrchestrator>,
    item_id: i64,
    target_position: i32,
) -> Result<(), String> {
    orchestrator.engine().reorder_playlist_item(item_id, target_position)
}

// Queue -------------------------------------------------------------------

#[tauri::command]
async fn phase5_queue_add(
    orchestrator: State<'_, PlaybackOrchestrator>,
    media_file_ids: Vec<i64>,
    mode: String,
) -> Result<playlist::PlaybackSnapshot, String> {
    let effects = orchestrator.engine().queue_add(media_file_ids, phase5_add_mode(&mode))?;
    Ok(phase5_finish(&orchestrator, effects).await)
}

#[tauri::command]
async fn phase5_queue_add_playlist(
    orchestrator: State<'_, PlaybackOrchestrator>,
    playlist_id: i64,
    mode: String,
) -> Result<playlist::PlaybackSnapshot, String> {
    let effects = orchestrator.engine().queue_add_playlist(playlist_id, phase5_add_mode(&mode))?;
    Ok(phase5_finish(&orchestrator, effects).await)
}

/// Starts a saved playlist as one continuous Cast session. This replaces the
/// queue, enables automatic advancement and loads the first available item.
/// Advancing to the next item still requires a real receiver completion event
/// correlated to the current playback token — no durations or timers are used.
#[tauri::command]
async fn phase5_play_playlist(
    orchestrator: State<'_, PlaybackOrchestrator>,
    playlist_id: i64,
) -> Result<playlist::PlaybackSnapshot, String> {
    let mut effects = orchestrator.engine().queue_clear()?;
    effects.extend(orchestrator.engine().set_mode(PlaybackMode::Auto)?);
    orchestrator.engine().select_playlist(playlist_id)?;
    effects.extend(orchestrator.engine().queue_add_playlist(playlist_id, AddMode::PlayNow)?);
    Ok(phase5_finish(&orchestrator, effects).await)
}

#[tauri::command]
async fn phase5_queue_remove(
    orchestrator: State<'_, PlaybackOrchestrator>,
    entry_id: String,
) -> Result<playlist::PlaybackSnapshot, String> {
    let effects = orchestrator.engine().queue_remove(entry_id)?;
    Ok(phase5_finish(&orchestrator, effects).await)
}

#[tauri::command]
async fn phase5_queue_reorder(
    orchestrator: State<'_, PlaybackOrchestrator>,
    entry_id: String,
    target_position: usize,
) -> Result<playlist::PlaybackSnapshot, String> {
    let effects = orchestrator.engine().queue_reorder(entry_id, target_position)?;
    Ok(phase5_finish(&orchestrator, effects).await)
}

#[tauri::command]
async fn phase5_queue_clear(
    orchestrator: State<'_, PlaybackOrchestrator>,
) -> Result<playlist::PlaybackSnapshot, String> {
    let effects = orchestrator.engine().queue_clear()?;
    Ok(phase5_finish(&orchestrator, effects).await)
}

// Playback settings & navigation ------------------------------------------

#[tauri::command]
async fn phase5_set_mode(
    orchestrator: State<'_, PlaybackOrchestrator>,
    mode: String,
) -> Result<playlist::PlaybackSnapshot, String> {
    let effects = orchestrator.engine().set_mode(PlaybackMode::parse(&mode))?;
    Ok(phase5_finish(&orchestrator, effects).await)
}

#[tauri::command]
async fn phase5_set_shuffle(
    orchestrator: State<'_, PlaybackOrchestrator>,
    enabled: bool,
) -> Result<playlist::PlaybackSnapshot, String> {
    let effects = orchestrator.engine().set_shuffle(enabled)?;
    Ok(phase5_finish(&orchestrator, effects).await)
}

#[tauri::command]
async fn phase5_set_repeat(
    orchestrator: State<'_, PlaybackOrchestrator>,
    mode: String,
) -> Result<playlist::PlaybackSnapshot, String> {
    let effects = orchestrator.engine().set_repeat(RepeatMode::parse(&mode))?;
    Ok(phase5_finish(&orchestrator, effects).await)
}

#[tauri::command]
async fn phase5_play_now(
    orchestrator: State<'_, PlaybackOrchestrator>,
    entry_id: String,
) -> Result<playlist::PlaybackSnapshot, String> {
    let effects = orchestrator.engine().play_now(entry_id)?;
    Ok(phase5_finish(&orchestrator, effects).await)
}

#[tauri::command]
async fn phase5_play_media_now(
    orchestrator: State<'_, PlaybackOrchestrator>,
    media_file_id: i64,
) -> Result<playlist::PlaybackSnapshot, String> {
    let effects = orchestrator.engine().play_media_now(media_file_id)?;
    Ok(phase5_finish(&orchestrator, effects).await)
}

#[tauri::command]
async fn phase5_next(
    orchestrator: State<'_, PlaybackOrchestrator>,
) -> Result<playlist::PlaybackSnapshot, String> {
    let effects = orchestrator.engine().next()?;
    Ok(phase5_finish(&orchestrator, effects).await)
}

#[tauri::command]
async fn phase5_previous(
    orchestrator: State<'_, PlaybackOrchestrator>,
) -> Result<playlist::PlaybackSnapshot, String> {
    let effects = orchestrator.engine().previous()?;
    Ok(phase5_finish(&orchestrator, effects).await)
}

#[tauri::command]
async fn phase5_recover(
    orchestrator: State<'_, PlaybackOrchestrator>,
) -> Result<playlist::PlaybackSnapshot, String> {
    let effects = orchestrator.engine().recover()?;
    Ok(phase5_finish(&orchestrator, effects).await)
}

pub(crate) fn open_database(app_data_dir: &Path) -> Result<Connection, Box<dyn std::error::Error>> {
    fs::create_dir_all(app_data_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(app_data_dir, fs::Permissions::from_mode(0o700))?;
    }
    let database_path = app_data_dir.join("windows-media-station.sqlite3");
    let connection = Connection::open(&database_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&database_path, fs::Permissions::from_mode(0o600))?;
    }
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.execute_batch(
        "BEGIN IMMEDIATE;
         CREATE TABLE IF NOT EXISTS schema_migrations (
             version INTEGER PRIMARY KEY,
             applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
         );
         CREATE TABLE IF NOT EXISTS media_directories (
             id INTEGER PRIMARY KEY,
             path TEXT NOT NULL UNIQUE,
             added_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
             last_scanned_at TEXT,
             is_active INTEGER NOT NULL DEFAULT 1 CHECK(is_active IN (0, 1)),
             availability_state TEXT NOT NULL DEFAULT 'available'
         );
         CREATE TABLE IF NOT EXISTS media_files (
             id INTEGER PRIMARY KEY,
             directory_id INTEGER NOT NULL REFERENCES media_directories(id) ON DELETE CASCADE,
             path TEXT NOT NULL UNIQUE,
             filename TEXT NOT NULL,
             extension TEXT NOT NULL,
             size_bytes INTEGER NOT NULL CHECK(size_bytes >= 0),
             modified_at TEXT NOT NULL,
             indexed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
             is_available INTEGER NOT NULL DEFAULT 1 CHECK(is_available IN (0, 1)),
             availability_state TEXT NOT NULL DEFAULT 'available'
         );
         CREATE INDEX IF NOT EXISTS idx_media_files_filename ON media_files(filename COLLATE NOCASE);
         CREATE INDEX IF NOT EXISTS idx_media_files_directory ON media_files(directory_id);
         CREATE TABLE IF NOT EXISTS thumbnail_cache (
             media_file_id INTEGER PRIMARY KEY REFERENCES media_files(id) ON DELETE CASCADE,
             source_size INTEGER NOT NULL,
             source_modified_at TEXT NOT NULL,
             cache_path TEXT,
             width INTEGER,
             height INTEGER,
             quality INTEGER,
             status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending', 'ready', 'failed')),
             error_message TEXT,
             created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
             updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
         );
         CREATE INDEX IF NOT EXISTS idx_thumbnail_cache_status ON thumbnail_cache(status);
         CREATE TABLE IF NOT EXISTS playlists (
             id INTEGER PRIMARY KEY,
             name TEXT NOT NULL,
             created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
             updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
         );
         CREATE TABLE IF NOT EXISTS playlist_items (
             id INTEGER PRIMARY KEY,
             playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
             media_file_id INTEGER NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
             position INTEGER NOT NULL CHECK(position >= 0),
             UNIQUE(playlist_id, position)
         );
         CREATE INDEX IF NOT EXISTS idx_playlist_items_playlist ON playlist_items(playlist_id, position);
         CREATE TABLE IF NOT EXISTS playback_history (
             id INTEGER PRIMARY KEY,
             media_file_id INTEGER NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
             started_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
             watched_seconds INTEGER NOT NULL DEFAULT 0 CHECK(watched_seconds >= 0)
         );
         CREATE TABLE IF NOT EXISTS application_settings (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL,
             updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
         );
         CREATE TABLE IF NOT EXISTS playback_state (
             id INTEGER PRIMARY KEY CHECK(id = 1),
             current_media_file_id INTEGER REFERENCES media_files(id) ON DELETE SET NULL,
             playlist_id INTEGER REFERENCES playlists(id) ON DELETE SET NULL,
             queue_position INTEGER,
             playback_mode TEXT NOT NULL DEFAULT 'manual' CHECK(playback_mode IN ('manual', 'auto')),
             shuffle_enabled INTEGER NOT NULL DEFAULT 0 CHECK(shuffle_enabled IN (0, 1)),
             repeat_mode TEXT NOT NULL DEFAULT 'off' CHECK(repeat_mode IN ('off', 'one', 'all')),
             updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
         );
         INSERT OR IGNORE INTO playback_state(id) VALUES (1);
         INSERT OR IGNORE INTO schema_migrations(version) VALUES (1);
         COMMIT;",
    )?;
    ensure_column(
        &connection,
        "media_directories",
        "is_active",
        "INTEGER NOT NULL DEFAULT 1 CHECK(is_active IN (0, 1))",
    )?;
    ensure_column(
        &connection,
        "media_files",
        "is_available",
        "INTEGER NOT NULL DEFAULT 1 CHECK(is_available IN (0, 1))",
    )?;
    connection.execute(
        "INSERT OR IGNORE INTO schema_migrations(version) VALUES (2)",
        [],
    )?;
    ensure_column(
        &connection,
        "media_directories",
        "availability_state",
        "TEXT NOT NULL DEFAULT 'available'",
    )?;
    ensure_column(
        &connection,
        "media_files",
        "availability_state",
        "TEXT NOT NULL DEFAULT 'available'",
    )?;
    connection.execute(
        "INSERT OR IGNORE INTO schema_migrations(version) VALUES (3)",
        [],
    )?;
    // Phase 5: persistent playlists, queue state machine and reliable duration.
    ensure_column(
        &connection,
        "media_files",
        "duration_seconds",
        "REAL",
    )?;
    ensure_column(
        &connection,
        "playback_history",
        "entry_id",
        "TEXT",
    )?;
    ensure_column(
        &connection,
        "playback_history",
        "session_id",
        "TEXT",
    )?;
    ensure_column(
        &connection,
        "playback_history",
        "position_seconds",
        "REAL NOT NULL DEFAULT 0",
    )?;
    ensure_column(
        &connection,
        "playback_history",
        "completion_state",
        "TEXT",
    )?;
    for (column, declaration) in [
        ("last_position_seconds", "REAL NOT NULL DEFAULT 0"),
        ("playback_session_id", "TEXT"),
        ("cast_session_id", "TEXT"),
        ("cast_media_session_id", "INTEGER"),
        ("last_completion_token", "TEXT"),
        ("last_selected_playlist_id", "INTEGER"),
        ("playback_engine_state", "TEXT NOT NULL DEFAULT 'idle'"),
        ("current_entry_id", "TEXT"),
        ("shuffle_order", "TEXT NOT NULL DEFAULT '[]'"),
        ("previous_restart_threshold_seconds", "REAL NOT NULL DEFAULT 30"),
    ] {
        ensure_column(&connection, "playback_state", column, declaration)?;
    }
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS playback_queue (
              entry_id TEXT PRIMARY KEY,
              media_file_id INTEGER NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
              position INTEGER NOT NULL CHECK(position >= 0),
              status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending', 'playing', 'completed', 'unavailable'))
          );
          CREATE INDEX IF NOT EXISTS idx_playback_queue_position ON playback_queue(position);",
    )?;
    connection.execute(
        "INSERT OR IGNORE INTO schema_migrations(version) VALUES (4)",
        [],
    )?;
    // M3U/M3U8/WPL imports. `status = 'deleted'` is a tombstone that prevents
    // a user-deleted imported playlist from being recreated by a later scan.
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS playlist_file_imports (
             path TEXT PRIMARY KEY,
             directory_id INTEGER REFERENCES media_directories(id) ON DELETE SET NULL,
             playlist_id INTEGER REFERENCES playlists(id) ON DELETE SET NULL,
             filename TEXT NOT NULL,
             extension TEXT NOT NULL,
             size_bytes INTEGER NOT NULL CHECK(size_bytes >= 0),
             modified_at TEXT NOT NULL,
             status TEXT NOT NULL DEFAULT 'imported'
               CHECK(status IN ('imported', 'error', 'deleted')),
             error_message TEXT,
             created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
             updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
         );
         CREATE INDEX IF NOT EXISTS idx_playlist_file_imports_playlist
           ON playlist_file_imports(playlist_id);",
    )?;
    connection.execute(
        "INSERT OR IGNORE INTO schema_migrations(version) VALUES (5)",
        [],
    )?;
    Ok(connection)
}

fn ensure_column(
    connection: &Connection,
    table: &str,
    column: &str,
    declaration: &str,
) -> Result<(), rusqlite::Error> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
    for existing_column in columns {
        if existing_column? == column {
            return Ok(());
        }
    }
    connection.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {declaration}"))?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_data_dir = app.path().app_local_data_dir()?;
            let cache_dir = app_data_dir.join("thumbnails");
            fs::create_dir_all(&cache_dir)?;
            let connection = open_database(&app_data_dir)?;
            let token = secret::load_or_create_token(&connection)
                .map_err(|_| std::io::Error::from(std::io::ErrorKind::Other))?;
            let database = Arc::new(Mutex::new(connection));
            app.manage(AppDatabase(Arc::clone(&database)));
            app.manage(ScanRegistry(Arc::new(Mutex::new(HashMap::new()))));
            app.manage(ThumbnailService {
                cache_dir,
                queue: Arc::new(Mutex::new(ThumbnailQueue::default())),
            });
            let server = MediaServerManager::new(app.handle().clone(), Arc::clone(&database), token);
            app.manage(server.clone());
            server.start_lifecycle_tasks();
            let cast = CastSenderManager::new(app.handle().clone(), Arc::clone(&database));
            app.manage(cast.clone());
            let playback = PlaybackEngine::new(Arc::clone(&database))
                .map_err(std::io::Error::other)?;
            app.manage(playback.clone());
            let orchestrator = PlaybackOrchestrator::new(
                app.handle().clone(),
                playback,
                server.clone(),
                cast.clone(),
            );
            app.manage(orchestrator.clone());
            // The authoritative queue engine listens to real Cast SDK events
            // inside Rust; the UI is only a renderer of its snapshots.
            let event_orchestrator = orchestrator.clone();
            app.listen("cast-playback-event", move |event| {
                if let Ok(report) = serde_json::from_str::<CastEventReport>(&event.payload()) {
                    event_orchestrator.handle_cast_report(report);
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let closing_window = window.clone();
                let server = closing_window.state::<MediaServerManager>().inner().clone();
                let cast = closing_window.state::<CastSenderManager>().inner().clone();
                if server.shutdown_is_finished() {
                    return;
                }
                api.prevent_close();
                if !server.begin_shutdown() {
                    return;
                }
                tauri::async_runtime::spawn(async move {
                    // The X button must stop receiver playback, not merely kill
                    // the sender process and leave the session alive. Ask the
                    // receiver to leave before the media server goes away.
                    let _ = cast.stop_playback();
                    server.shutdown().await;
                    sender_embed::detach_embedded();
                    cast.shutdown().await;
                    let _ = closing_window.destroy();
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_local_status,
            wizard_diag_log,
            add_media_directory,
            list_media_directories,
            remove_media_directory,
            list_media_files,
            scan_media_directory,
            cancel_media_scan,
            import_playlist_files,
            export_playlist_file,
            get_thumbnail_settings,
            set_thumbnail_settings,
            refresh_thumbnail,
            prepare_thumbnail_cache,
            cleanup_thumbnail_cache,
            get_media_server_status,
            get_media_pairing_token,
            start_media_server,
            stop_media_server,
            restart_media_server,
            set_media_server_port,
            get_media_network_diagnostics,
            test_media_server_connection,
            get_media_stream_diagnostics,
            clear_media_stream_diagnostics,
            discover_cast_devices,
            get_cast_sender_status,
            launch_cast_sender,
            embed_cast_sender,
            show_cast_sender,
            move_cast_sender,
            hide_cast_sender,
            stop_cast_sender,
            get_cast_diagnostics,
            get_cast_playback_state,
            cast_load_media,
            cast_send_control,
            verify_cast_media_url,
            phase5_get_state,
            phase5_clear_watch_history,
            phase5_list_playlists,
            phase5_create_playlist,
            phase5_rename_playlist,
            phase5_delete_playlist,
            phase5_select_playlist,
            phase5_list_playlist_items,
            phase5_add_playlist_items,
            phase5_remove_playlist_item,
            phase5_reorder_playlist_item,
            phase5_queue_add,
            phase5_queue_add_playlist,
            phase5_play_playlist,
            phase5_queue_remove,
            phase5_queue_reorder,
            phase5_queue_clear,
            phase5_set_mode,
            phase5_set_shuffle,
            phase5_set_repeat,
            phase5_play_now,
            phase5_play_media_now,
            phase5_next,
            phase5_previous,
            phase5_recover
        ])
        .run(tauri::generate_context!())
        .expect("failed to run RollaPlay");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn fixture() -> (TempDir, Arc<Mutex<Connection>>, i64, PathBuf) {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("media");
        fs::create_dir_all(&root).unwrap();
        let database = Arc::new(Mutex::new(open_database(&temporary.path().join("data")).unwrap()));
        let path = root.to_string_lossy().into_owned();
        let directory_id = database
            .lock()
            .unwrap()
            .query_row(
                "INSERT INTO media_directories(path, is_active, availability_state) VALUES (?1, 1, 'available') RETURNING id",
                [&path],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        (temporary, database, directory_id, root)
    }

    fn scan(database: &Arc<Mutex<Connection>>, directory_id: i64, root: PathBuf) -> Result<ScanResult, String> {
        scan_directory(database, directory_id, root, &AtomicBool::new(false), |_| {})
    }

    fn indexed_count(database: &Arc<Mutex<Connection>>) -> i64 {
        database.lock().unwrap().query_row("SELECT COUNT(*) FROM media_files", [], |row| row.get(0)).unwrap()
    }

    fn playlist_count(database: &Arc<Mutex<Connection>>) -> i64 {
        database.lock().unwrap().query_row("SELECT COUNT(*) FROM playlists", [], |row| row.get(0)).unwrap()
    }

    fn playlist_filenames(database: &Arc<Mutex<Connection>>, playlist_id: i64) -> Vec<String> {
        let connection = database.lock().unwrap();
        let mut statement = connection
            .prepare(
                "SELECT f.filename FROM playlist_items i JOIN media_files f ON f.id = i.media_file_id
                 WHERE i.playlist_id = ?1 ORDER BY i.position, i.id",
            )
            .unwrap();
        statement
            .query_map([playlist_id], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    #[test]
    fn repeated_scans_do_not_duplicate_files() {
        let (_temporary, database, directory_id, root) = fixture();
        fs::write(root.join("movie.mp4"), b"sample video bytes").unwrap();
        scan(&database, directory_id, root.clone()).unwrap();
        scan(&database, directory_id, root).unwrap();
        assert_eq!(indexed_count(&database), 1);
    }

    #[test]
    fn modified_file_invalidates_thumbnail_cache_and_keeps_identity() {
        let (_temporary, database, directory_id, root) = fixture();
        let path = root.join("movie.mp4");
        fs::write(&path, b"first version").unwrap();
        scan(&database, directory_id, root.clone()).unwrap();
        let (media_file_id, first_size) = database
            .lock()
            .unwrap()
            .query_row("SELECT id, size_bytes FROM media_files WHERE path = ?1", [path.to_string_lossy().as_ref()], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
            .unwrap();
        database.lock().unwrap().execute(
            "UPDATE thumbnail_cache SET cache_path = 'obsolete.jpg', status = 'ready', width = 320, height = 180, quality = 82 WHERE media_file_id = ?1",
            [media_file_id],
        ).unwrap();
        fs::write(&path, b"a longer, modified video placeholder").unwrap();
        scan(&database, directory_id, root).unwrap();
        let (current_id, current_size, thumbnail_status, cache_path) = database
            .lock()
            .unwrap()
            .query_row(
                "SELECT f.id, f.size_bytes, t.status, t.cache_path FROM media_files f JOIN thumbnail_cache t ON t.media_file_id = f.id WHERE f.path = ?1",
                [path.to_string_lossy().as_ref()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?)),
            )
            .unwrap();
        assert_eq!(current_id, media_file_id);
        assert_ne!(current_size, first_size);
        assert_eq!(thumbnail_status, "pending");
        assert!(cache_path.is_none());
        assert_eq!(indexed_count(&database), 1);
    }

    #[test]
    fn renamed_file_keeps_playlist_reference_when_signature_is_unique() {
        let (_temporary, database, directory_id, root) = fixture();
        let old_path = root.join("old-name.mp4");
        fs::write(&old_path, b"stable file contents").unwrap();
        scan(&database, directory_id, root.clone()).unwrap();
        let media_file_id = database.lock().unwrap().query_row("SELECT id FROM media_files", [], |row| row.get::<_, i64>(0)).unwrap();
        database.lock().unwrap().execute("INSERT INTO playlists(name) VALUES ('Saved')", []).unwrap();
        let playlist_id = database.lock().unwrap().last_insert_rowid();
        database.lock().unwrap().execute("INSERT INTO playlist_items(playlist_id, media_file_id, position) VALUES (?1, ?2, 0)", params![playlist_id, media_file_id]).unwrap();
        let new_path = root.join("new-name.mp4");
        fs::rename(&old_path, &new_path).unwrap();
        let result = scan(&database, directory_id, root).unwrap();
        assert_eq!(result.renamed_files, 1);
        assert_eq!(indexed_count(&database), 1);
        assert_eq!(database.lock().unwrap().query_row("SELECT media_file_id FROM playlist_items WHERE playlist_id = ?1", [playlist_id], |row| row.get::<_, i64>(0)).unwrap(), media_file_id);
        assert_eq!(database.lock().unwrap().query_row("SELECT path FROM media_files WHERE id = ?1", [media_file_id], |row| row.get::<_, String>(0)).unwrap(), new_path.to_string_lossy().into_owned());
    }

    #[test]
    fn cancelled_scan_discards_partial_index_changes() {
        let (_temporary, database, directory_id, root) = fixture();
        fs::write(root.join("movie.mp4"), b"sample video bytes").unwrap();
        let cancel = AtomicBool::new(true);
        let result = scan_directory(&database, directory_id, root, &cancel, |_| {}).unwrap();
        assert!(result.cancelled);
        assert_eq!(indexed_count(&database), 0);
    }

    #[test]
    fn inaccessible_root_is_temporary_and_preserves_playlist_items() {
        let (_temporary, database, directory_id, root) = fixture();
        let file = root.join("movie.mp4");
        fs::write(&file, b"sample video bytes").unwrap();
        scan(&database, directory_id, root.clone()).unwrap();
        let media_file_id = database.lock().unwrap().query_row("SELECT id FROM media_files", [], |row| row.get::<_, i64>(0)).unwrap();
        database.lock().unwrap().execute("INSERT INTO playlists(name) VALUES ('Saved')", []).unwrap();
        let playlist_id = database.lock().unwrap().last_insert_rowid();
        database.lock().unwrap().execute("INSERT INTO playlist_items(playlist_id, media_file_id, position) VALUES (?1, ?2, 0)", params![playlist_id, media_file_id]).unwrap();
        fs::remove_dir_all(root).unwrap();
        assert!(scan(&database, directory_id, file.parent().unwrap().to_path_buf()).is_err());
        let state = database.lock().unwrap().query_row("SELECT availability_state FROM media_files WHERE id = ?1", [media_file_id], |row| row.get::<_, String>(0)).unwrap();
        let references = database.lock().unwrap().query_row("SELECT COUNT(*) FROM playlist_items WHERE playlist_id = ?1", [playlist_id], |row| row.get::<_, i64>(0)).unwrap();
        assert_eq!(state, "temporarily_unavailable");
        assert_eq!(references, 1);
    }

    #[test]
    fn confirmed_deletion_is_not_confused_with_an_offline_drive() {
        let (_temporary, database, directory_id, root) = fixture();
        let file = root.join("movie.mp4");
        fs::write(&file, b"sample video bytes").unwrap();
        scan(&database, directory_id, root.clone()).unwrap();
        fs::remove_file(file).unwrap();
        let result = scan(&database, directory_id, root).unwrap();
        assert_eq!(result.files_removed, 1);
        assert_eq!(database.lock().unwrap().query_row("SELECT availability_state FROM media_files", [], |row| row.get::<_, String>(0)).unwrap(), "permanently_missing");
    }

    #[test]
    fn duplicate_active_scans_are_rejected_and_cancel_flags_are_shared() {
        let registry = ScanRegistry(Arc::new(Mutex::new(HashMap::new())));
        let cancel = register_scan(&registry, 42).unwrap();
        assert!(register_scan(&registry, 42).is_err());
        cancel.store(true, Ordering::Relaxed);
        assert!(registry.0.lock().unwrap().get(&42).unwrap().load(Ordering::Relaxed));
        drop(ScanGuard { directory_id: 42, registry: Arc::clone(&registry.0) });
        assert!(register_scan(&registry, 42).is_ok());
    }

    #[test]
    fn incomplete_walks_classify_absent_entries_as_temporary() {
        assert_eq!(missing_file_availability_state(0), "permanently_missing");
        assert_eq!(missing_file_availability_state(1), "temporarily_unavailable");
    }

    #[test]
    fn partial_scan_preserves_inaccessible_file_and_playlist_reference() {
        let (_temporary, database, directory_id, root) = fixture();
        fs::write(root.join("private.mp4"), b"sample video bytes").unwrap();
        scan(&database, directory_id, root).unwrap();
        let media_file_id = database.lock().unwrap().query_row("SELECT id FROM media_files", [], |row| row.get::<_, i64>(0)).unwrap();
        database.lock().unwrap().execute("INSERT INTO playlists(name) VALUES ('Saved')", []).unwrap();
        let playlist_id = database.lock().unwrap().last_insert_rowid();
        database.lock().unwrap().execute("INSERT INTO playlist_items(playlist_id, media_file_id, position) VALUES (?1, ?2, 0)", params![playlist_id, media_file_id]).unwrap();
        let connection = database.lock().unwrap();
        let transaction = connection.unchecked_transaction().unwrap();
        let result = apply_missing_file_states(&transaction, directory_id, &HashSet::new(), 1).unwrap();
        transaction.commit().unwrap();
        assert_eq!(result, (0, 1, 0));
        assert_eq!(connection.query_row("SELECT availability_state FROM media_files WHERE id = ?1", [media_file_id], |row| row.get::<_, String>(0)).unwrap(), "temporarily_unavailable");
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM playlist_items WHERE playlist_id = ?1", [playlist_id], |row| row.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn removing_folder_keeps_playlist_rows_and_marks_entries_removed() {
        let (_temporary, database, directory_id, root) = fixture();
        fs::write(root.join("movie.mp4"), b"sample video bytes").unwrap();
        scan(&database, directory_id, root).unwrap();
        let media_file_id = database.lock().unwrap().query_row("SELECT id FROM media_files", [], |row| row.get::<_, i64>(0)).unwrap();
        database.lock().unwrap().execute("INSERT INTO playlists(name) VALUES ('Saved')", []).unwrap();
        let playlist_id = database.lock().unwrap().last_insert_rowid();
        database.lock().unwrap().execute("INSERT INTO playlist_items(playlist_id, media_file_id, position) VALUES (?1, ?2, 0)", params![playlist_id, media_file_id]).unwrap();
        remove_directory_from_database(&mut database.lock().unwrap(), directory_id).unwrap();
        let state = database.lock().unwrap().query_row("SELECT availability_state FROM media_files WHERE id = ?1", [media_file_id], |row| row.get::<_, String>(0)).unwrap();
        let references = database.lock().unwrap().query_row("SELECT COUNT(*) FROM playlist_items WHERE playlist_id = ?1", [playlist_id], |row| row.get::<_, i64>(0)).unwrap();
        assert_eq!(state, "removed_from_library");
        assert_eq!(references, 1);
    }

    #[test]
    fn m3u_scan_imports_only_indexed_files_and_is_idempotent() {
        let (_temporary, database, directory_id, root) = fixture();
        fs::write(root.join("one.mp4"), b"one").unwrap();
        fs::write(root.join("two.mp4"), b"two").unwrap();
        let playlist_path = root.join("Road Trip.m3u");
        fs::write(
            &playlist_path,
            "#EXTM3U\r\n#PLAYLIST:Road Trip\r\n#EXTINF:99,One\r\none.mp4\r\n\
             http://example.com/stream.m3u8\r\nmissing.mp4\r\n",
        )
        .unwrap();

        let first = scan(&database, directory_id, root.clone()).unwrap();
        assert_eq!(first.playlists_imported, 1);
        assert_eq!(playlist_count(&database), 1);
        let (playlist_id, name) = database
            .lock()
            .unwrap()
            .query_row("SELECT id, name FROM playlists", [], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap();
        assert_eq!(name, "Road Trip");
        assert_eq!(playlist_filenames(&database, playlist_id), vec!["one.mp4"]);

        let second = scan(&database, directory_id, root.clone()).unwrap();
        assert_eq!(second.playlists_imported, 0);
        assert_eq!(playlist_count(&database), 1);
        assert_eq!(playlist_filenames(&database, playlist_id), vec!["one.mp4"]);

        fs::write(
            &playlist_path,
            "#EXTM3U\r\n#PLAYLIST:Road Trip Remix\r\ntwo.mp4\r\none.mp4\r\n",
        )
        .unwrap();
        let third = scan(&database, directory_id, root).unwrap();
        assert_eq!(third.playlists_imported, 1);
        assert_eq!(playlist_count(&database), 1);
        assert_eq!(database.lock().unwrap().query_row("SELECT name FROM playlists WHERE id = ?1", [playlist_id], |row| row.get::<_, String>(0)).unwrap(), "Road Trip Remix");
        assert_eq!(playlist_filenames(&database, playlist_id), vec!["two.mp4", "one.mp4"]);
    }

    #[test]
    fn wpl_scan_decodes_utf16_and_xml_entities() {
        let (_temporary, database, directory_id, root) = fixture();
        fs::write(root.join("a & b.mp4"), b"video").unwrap();
        let source = "<?wpl version=\"1.0\"?><smil><head><title>Rock &amp; Roll</title></head>\
                      <body><seq><media src=\"a &amp; b.mp4\"/></seq></body></smil>";
        let units: Vec<u16> = source.encode_utf16().collect();
        let mut bytes = vec![0xff, 0xfe];
        for unit in units {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        fs::write(root.join("Roadie.wpl"), bytes).unwrap();

        let result = scan(&database, directory_id, root).unwrap();
        assert_eq!(result.playlists_imported, 1);
        let (playlist_id, name) = database
            .lock()
            .unwrap()
            .query_row("SELECT id, name FROM playlists", [], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap();
        assert_eq!(name, "Rock & Roll");
        assert_eq!(playlist_filenames(&database, playlist_id), vec!["a & b.mp4"]);
    }

    #[test]
    fn deleting_an_imported_playlist_keeps_a_rescan_tombstone() {
        let (_temporary, database, directory_id, root) = fixture();
        fs::write(root.join("one.mp4"), b"one").unwrap();
        fs::write(root.join("Saved.m3u"), "#EXTM3U\none.mp4\n").unwrap();
        scan(&database, directory_id, root.clone()).unwrap();
        let playlist_id = database
            .lock()
            .unwrap()
            .query_row("SELECT id FROM playlists", [], |row| row.get::<_, i64>(0))
            .unwrap();

        let engine = PlaybackEngine::new(Arc::clone(&database)).unwrap();
        engine.delete_playlist(playlist_id).unwrap();
        assert_eq!(playlist_count(&database), 0);
        assert_eq!(
            database.lock().unwrap().query_row(
                "SELECT status FROM playlist_file_imports WHERE playlist_id IS NULL",
                [],
                |row| row.get::<_, String>(0),
            ).unwrap(),
            "deleted",
        );

        let rescan = scan(&database, directory_id, root).unwrap();
        assert_eq!(rescan.playlists_imported, 0);
        assert_eq!(playlist_count(&database), 0);
    }

    #[test]
    fn cancelled_scan_does_not_import_playlist_files() {
        let (_temporary, database, directory_id, root) = fixture();
        fs::write(root.join("one.mp4"), b"one").unwrap();
        fs::write(root.join("Saved.m3u"), "#EXTM3U\none.mp4\n").unwrap();
        let cancel = AtomicBool::new(true);
        let result = scan_directory(&database, directory_id, root, &cancel, |_| {}).unwrap();
        assert!(result.cancelled);
        assert_eq!(result.playlists_imported, 0);
        assert_eq!(playlist_count(&database), 0);
    }

    #[test]
    fn legacy_database_migrates_thumbnail_and_availability_metadata() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("legacy.sqlite3");
        let legacy = Connection::open(&path).unwrap();
        legacy.execute_batch(
            "CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, applied_at TEXT DEFAULT CURRENT_TIMESTAMP);
             CREATE TABLE media_directories(id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE, added_at TEXT DEFAULT CURRENT_TIMESTAMP, last_scanned_at TEXT);
             CREATE TABLE media_files(id INTEGER PRIMARY KEY, directory_id INTEGER NOT NULL, path TEXT NOT NULL UNIQUE, filename TEXT NOT NULL, extension TEXT NOT NULL, size_bytes INTEGER NOT NULL, modified_at TEXT NOT NULL, indexed_at TEXT DEFAULT CURRENT_TIMESTAMP);
             INSERT INTO schema_migrations(version) VALUES (1);",
        ).unwrap();
        drop(legacy);
        let migrated = open_database(temporary.path()).unwrap();
        let columns = migrated.prepare("PRAGMA table_info(media_files)").unwrap().query_map([], |row| row.get::<_, String>(1)).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        assert!(columns.contains(&"availability_state".to_string()));
        assert!(columns.contains(&"is_available".to_string()));
        assert!(migrated.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'thumbnail_cache'", [], |row| row.get::<_, i64>(0)).unwrap() == 1);
    }
}
