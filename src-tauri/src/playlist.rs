//! Phase 5 — playlist, playback queue and playback state machine.
//!
//! This module is the **authoritative owner of queue state**. SQLite is the
//! durable source of truth and this module is the in-memory decision maker;
//! the React UI only renders snapshots and sends user commands. Cast
//! communication stays in `cast_sender`, streaming stays in `media_server`.
//!
//! Responsibilities:
//! - playlists: create/rename/delete/add/remove/reorder items (files are never
//!   deleted by playlist operations),
//! - queue: unique entry identities, ordering, play-next insertion, clearing,
//!   playlist ingest, persistence and recovery,
//! - state machine: Idle/Connecting/Loading/Playing/Paused/Buffering/Completed/
//!   WaitingForManualSelection/Recovering/Disconnected/Error with explicit
//!   transitions driven only by Cast media status events,
//! - progression: Automatic vs Manual, Shuffle, Repeat Off/One/All, Previous
//!   history.
//!
//! The engine never calls Cast directly. Every method returns `EngineEffect`s
//! which the Tauri orchestrator executes against the existing Cast service.
//! Completion is accepted only when the `playback_token` generated for the
//! current load matches, and (when available) the Cast session id and media
//! session id also match. This makes duplicate and stale completion events
//! harmless.
//!
//! Previous behaviour with Shuffle enabled: playback history keeps the real
//! order in which entries were played, so Previous always returns to the
//! previously played entry and never to a random neighbour. If the current
//! entry has played longer than `previous_restart_threshold_seconds`
//! (default 30s), Previous restarts it instead of navigating backward.

use rand::seq::SliceRandom;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use uuid::Uuid;

pub const PREVIOUS_RESTART_THRESHOLD_SECONDS: f64 = 30.0;
const HISTORY_DISPLAY_LIMIT: usize = 30;

// ---------------------------------------------------------------------------
// Public data model
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackMode {
    Manual,
    Auto,
}

impl PlaybackMode {
    pub fn parse(value: &str) -> Self {
        if value.eq_ignore_ascii_case("auto") { Self::Auto } else { Self::Manual }
    }
    pub fn as_str(self) -> &'static str {
        match self { Self::Auto => "auto", Self::Manual => "manual" }
    }
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RepeatMode {
    Off,
    One,
    All,
}

impl RepeatMode {
    pub fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "one" => Self::One,
            "all" => Self::All,
            _ => Self::Off,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self { Self::Off => "off", Self::One => "one", Self::All => "all" }
    }
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum EngineState {
    #[default]
    Idle,
    Connecting,
    Loading,
    Playing,
    Paused,
    Buffering,
    Completed,
    WaitingForManualSelection,
    Recovering,
    Disconnected,
    Error,
}

impl EngineState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Connecting => "connecting",
            Self::Loading => "loading",
            Self::Playing => "playing",
            Self::Paused => "paused",
            Self::Buffering => "buffering",
            Self::Completed => "completed",
            Self::WaitingForManualSelection => "waiting_manual",
            Self::Recovering => "recovering",
            Self::Disconnected => "disconnected",
            Self::Error => "error",
        }
    }
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntryStatus {
    Pending,
    Playing,
    Completed,
    Unavailable,
}

impl EntryStatus {
    fn parse(value: &str) -> Self {
        match value {
            "playing" => Self::Playing,
            "completed" => Self::Completed,
            "unavailable" => Self::Unavailable,
            _ => Self::Pending,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Playing => "playing",
            Self::Completed => "completed",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct QueueEntry {
    pub entry_id: String,
    pub media_file_id: i64,
    pub filename: String,
    pub availability_state: String,
    pub duration_seconds: Option<f64>,
    pub status: EntryStatus,
    pub position: i32,
}

#[derive(Serialize, Clone, Debug)]
pub struct PlaylistSummary {
    pub id: i64,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub item_count: i64,
    pub duration_seconds: Option<f64>,
}

#[derive(Serialize, Clone, Debug)]
pub struct PlaylistItem {
    pub id: i64,
    pub media_file_id: i64,
    pub filename: String,
    pub availability_state: String,
    pub duration_seconds: Option<f64>,
    pub position: i32,
}

#[derive(Serialize, Clone, Debug)]
pub struct HistoryEntry {
    pub id: i64,
    pub entry_id: Option<String>,
    pub media_file_id: i64,
    pub filename: String,
    pub started_at: String,
    pub watched_seconds: i64,
    pub completion_state: Option<String>,
    pub position_seconds: f64,
}

#[derive(Serialize, Clone, Debug)]
pub struct PlaybackSnapshot {
    pub state: EngineState,
    pub mode: PlaybackMode,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub queue: Vec<QueueEntry>,
    pub current_entry_id: Option<String>,
    pub next_entry_id: Option<String>,
    pub history: Vec<HistoryEntry>,
    pub current_media_file_id: Option<i64>,
    pub position_seconds: f64,
    pub duration_seconds: Option<f64>,
    pub playback_token: Option<String>,
    pub cast_session_id: Option<String>,
    pub media_session_id: Option<i64>,
    pub last_completion_token: Option<String>,
    pub last_selected_playlist_id: Option<i64>,
    pub previous_restart_threshold_seconds: f64,
    pub message: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub enum EngineEffect {
    /// Ask the orchestrator to issue a fresh, media-scoped Cast load.
    Load {
        media_file_id: i64,
        entry_id: String,
        playback_token: String,
        position_seconds: f64,
        filename: String,
    },
    /// Stop Cast playback without touching the queue.
    Stop,
    /// Unobtrusive notification for the UI.
    Notify { level: String, message: String },
}

#[derive(Clone, Debug, Default)]
pub struct CastStatusEvent {
    pub kind: String,
    pub playback_token: Option<String>,
    pub cast_session_id: Option<String>,
    pub media_session_id: Option<i64>,
    pub player_state: Option<String>,
    pub position_seconds: Option<f64>,
    pub duration_seconds: Option<f64>,
    pub idle_reason: Option<String>,
    pub message: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub enum AddMode {
    Append,
    PlayNext,
    PlayNow,
}

// ---------------------------------------------------------------------------
// Engine state
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct EngineStateData {
    queue: Vec<QueueEntry>,
    current_entry_id: Option<String>,
    current_token: Option<String>,
    cast_session_id: Option<String>,
    media_session_id: Option<i64>,
    position_seconds: f64,
    duration_seconds: Option<f64>,
    mode: PlaybackMode,
    shuffle: bool,
    repeat: RepeatMode,
    state: EngineState,
    shuffle_order: Vec<String>,
    history_stack: Vec<String>,
    last_completion_token: Option<String>,
    last_selected_playlist_id: Option<i64>,
    restart_threshold: f64,
    message: Option<String>,
    last_error: Option<String>,
}

impl Default for EngineStateData {
    fn default() -> Self {
        Self {
            queue: Vec::new(),
            current_entry_id: None,
            current_token: None,
            cast_session_id: None,
            media_session_id: None,
            position_seconds: 0.0,
            duration_seconds: None,
            mode: PlaybackMode::Manual,
            shuffle: false,
            repeat: RepeatMode::Off,
            state: EngineState::Idle,
            shuffle_order: Vec::new(),
            history_stack: Vec::new(),
            last_completion_token: None,
            last_selected_playlist_id: None,
            restart_threshold: PREVIOUS_RESTART_THRESHOLD_SECONDS,
            message: None,
            last_error: None,
        }
    }
}

#[derive(Clone)]
pub struct PlaybackEngine {
    database: Arc<Mutex<Connection>>,
    inner: Arc<Mutex<EngineStateData>>,
}

impl PlaybackEngine {
    pub fn new(database: Arc<Mutex<Connection>>) -> Result<Self, String> {
        let engine = Self { database, inner: Arc::new(Mutex::new(EngineStateData::default())) };
        engine.load()?;
        Ok(engine)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, EngineStateData> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.database.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn snapshot(&self) -> PlaybackSnapshot {
        let data = self.lock();
        let next_entry_id = self.next_candidate(&data);
        let current_media_file_id = data.current_entry_id.as_ref().and_then(|id| {
            data.queue.iter().find(|entry| &entry.entry_id == id).map(|entry| entry.media_file_id)
        });
        let history = self.recent_history();
        PlaybackSnapshot {
            state: data.state,
            mode: data.mode,
            shuffle: data.shuffle,
            repeat: data.repeat,
            queue: data.queue.clone(),
            current_entry_id: data.current_entry_id.clone(),
            next_entry_id,
            history,
            current_media_file_id,
            position_seconds: data.position_seconds,
            duration_seconds: data.duration_seconds,
            playback_token: data.current_token.clone(),
            cast_session_id: data.cast_session_id.clone(),
            media_session_id: data.media_session_id,
            last_completion_token: data.last_completion_token.clone(),
            last_selected_playlist_id: data.last_selected_playlist_id,
            previous_restart_threshold_seconds: data.restart_threshold,
            message: data.message.clone(),
            last_error: data.last_error.clone(),
        }
    }

    // -----------------------------------------------------------------------
    // Persistence / loading
    // -----------------------------------------------------------------------

    fn load(&self) -> Result<(), String> {
        let connection = self.db();
        let row = connection
            .query_row(
                "SELECT current_media_file_id, playlist_id, queue_position, playback_mode,
                        shuffle_enabled, repeat_mode, last_position_seconds, playback_session_id,
                        cast_session_id, cast_media_session_id, last_completion_token,
                        last_selected_playlist_id, playback_engine_state, current_entry_id,
                        shuffle_order, previous_restart_threshold_seconds
                 FROM playback_state WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, Option<i64>>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)? != 0,
                        row.get::<_, String>(5)?,
                        row.get::<_, f64>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                        row.get::<_, Option<i64>>(9)?,
                        row.get::<_, Option<String>>(10)?,
                        row.get::<_, Option<i64>>(11)?,
                        row.get::<_, String>(12)?,
                        row.get::<_, Option<String>>(13)?,
                        row.get::<_, Option<String>>(14)?,
                        row.get::<_, f64>(15)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("Could not restore playback state: {error}"))?;
        let Some(row) = row else { return Ok(()) };
        let mut statement = connection
            .prepare(
                "SELECT q.entry_id, q.media_file_id, COALESCE(f.filename, '(missing file)'),
                        COALESCE(f.availability_state, 'permanently_missing'), f.duration_seconds, q.status, q.position
                 FROM playback_queue q
                 LEFT JOIN media_files f ON f.id = q.media_file_id
                 ORDER BY q.position",
            )
            .map_err(|error| format!("Could not read the playback queue: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok(QueueEntry {
                    entry_id: row.get(0)?,
                    media_file_id: row.get(1)?,
                    filename: row.get(2)?,
                    availability_state: row.get(3)?,
                    duration_seconds: row.get(4)?,
                    status: EntryStatus::parse(&row.get::<_, String>(5)?),
                    position: row.get(6)?,
                })
            })
            .map_err(|error| format!("Could not read queue rows: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Could not read queue rows: {error}"))?;
        drop(statement);
        drop(connection);

        let shuffle_order = row
            .14
            .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
            .unwrap_or_default();
        let mut data = self.lock();
        data.queue = rows;
        data.current_entry_id = row.13.filter(|id| data.queue.iter().any(|entry| &entry.entry_id == id));
        // A restored token is deliberately discarded: after a restart the old
        // receiver/media session is no longer trusted and playback never
        // resumes without an explicit user action.
        data.current_token = None;
        data.cast_session_id = None;
        data.media_session_id = None;
        data.position_seconds = row.6.max(0.0);
        data.duration_seconds = data
            .current_entry_id
            .as_ref()
            .and_then(|id| data.queue.iter().find(|entry| &entry.entry_id == id))
            .and_then(|entry| entry.duration_seconds);
        data.mode = PlaybackMode::parse(&row.3);
        data.shuffle = row.4;
        data.repeat = RepeatMode::parse(&row.5);
        data.last_completion_token = row.10;
        data.last_selected_playlist_id = row.11;
        data.shuffle_order = shuffle_order.into_iter().filter(|id| data.queue.iter().any(|entry| &entry.entry_id == id)).collect();
        data.restart_threshold = if row.15 > 0.0 { row.15 } else { PREVIOUS_RESTART_THRESHOLD_SECONDS };
        // Never auto-start after a restart. Keep the item and position so the
        // user can explicitly recover.
        data.state = if data.current_entry_id.is_some() { EngineState::Recovering } else { EngineState::Idle };
        data.message = if data.current_entry_id.is_some() {
            Some("Previous playback restored. Choose Reconnect & resume to continue.".to_string())
        } else {
            None
        };
        drop(data);
        Ok(())
    }

    fn persist(&self, data: &EngineStateData) {
        let connection = self.db();
        let position = data.current_entry_id.as_ref().and_then(|id| {
            data.queue.iter().position(|entry| &entry.entry_id == id).map(|index| index as i64)
        });
        let current_media_id = data.current_entry_id.as_ref().and_then(|id| {
            data.queue.iter().find(|entry| &entry.entry_id == id).map(|entry| entry.media_file_id)
        });
        let shuffle_order = serde_json::to_string(&data.shuffle_order).unwrap_or_else(|_| "[]".to_string());
        let result = connection.execute(
            "UPDATE playback_state SET current_media_file_id=?1, playlist_id=?2, queue_position=?3,
                playback_mode=?4, shuffle_enabled=?5, repeat_mode=?6, last_position_seconds=?7,
                playback_session_id=?8, cast_session_id=?9, cast_media_session_id=?10,
                last_completion_token=?11, last_selected_playlist_id=?12,
                playback_engine_state=?13, current_entry_id=?14, shuffle_order=?15,
                previous_restart_threshold_seconds=?16, updated_at=CURRENT_TIMESTAMP
             WHERE id=1",
            params![
                current_media_id,
                data.last_selected_playlist_id,
                position,
                data.mode.as_str(),
                i64::from(data.shuffle),
                data.repeat.as_str(),
                data.position_seconds,
                data.current_token,
                data.cast_session_id,
                data.media_session_id,
                data.last_completion_token,
                data.last_selected_playlist_id,
                data.state.as_str(),
                data.current_entry_id,
                shuffle_order,
                data.restart_threshold,
            ],
        );
        if result.is_err() {
            // Persistence failure is surfaced through the snapshot on the next
            // command; in-memory state remains usable for this session.
        }
        let transaction = connection.unchecked_transaction();
        if let Ok(transaction) = transaction {
            let _ = transaction.execute("DELETE FROM playback_queue", []);
            for entry in &data.queue {
                let _ = transaction.execute(
                    "INSERT INTO playback_queue(entry_id, media_file_id, position, status) VALUES (?1, ?2, ?3, ?4)",
                    params![entry.entry_id, entry.media_file_id, entry.position, entry.status.as_str()],
                );
            }
            let _ = transaction.commit();
        }
    }

    fn recent_history(&self) -> Vec<HistoryEntry> {
        let connection = self.db();
        let mut statement = connection
            .prepare(
                "SELECT h.id, h.entry_id, h.media_file_id, COALESCE(f.filename, '(missing file)'),
                        h.started_at, h.watched_seconds, h.completion_state, h.position_seconds
                 FROM playback_history h LEFT JOIN media_files f ON f.id = h.media_file_id
                 ORDER BY h.id DESC LIMIT ?1",
            )
            .unwrap();
        statement
            .query_map([HISTORY_DISPLAY_LIMIT as i64], |row| {
                Ok(HistoryEntry {
                    id: row.get(0)?,
                    entry_id: row.get(1)?,
                    media_file_id: row.get(2)?,
                    filename: row.get(3)?,
                    started_at: row.get(4)?,
                    watched_seconds: row.get(5)?,
                    completion_state: row.get(6)?,
                    position_seconds: row.get(7)?,
                })
            })
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .collect()
    }

    fn media_details(&self, media_file_id: i64) -> Option<(String, String, Option<f64>)> {
        let connection = self.db();
        connection
            .query_row(
                "SELECT filename, availability_state, duration_seconds FROM media_files WHERE id = ?1",
                [media_file_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .ok()
            .flatten()
    }

    fn renumber(&self, data: &mut EngineStateData) {
        for (index, entry) in data.queue.iter_mut().enumerate() {
            entry.position = index as i32;
        }
    }

    fn rebuild_shuffle_order(&self, data: &mut EngineStateData) {
        let mut ids: Vec<String> = data.queue.iter().map(|entry| entry.entry_id.clone()).collect();
        if data.shuffle {
            ids.shuffle(&mut rand::thread_rng());
            if let Some(current) = data.current_entry_id.clone() {
                if let Some(index) = ids.iter().position(|id| id == &current) {
                    let id = ids.remove(index);
                    ids.insert(0, id);
                }
            }
        } else {
            ids.sort_by_key(|id| data.queue.iter().find(|entry| &entry.entry_id == id).map(|entry| entry.position).unwrap_or(i32::MAX));
        }
        data.shuffle_order = ids;
    }

    fn next_candidate(&self, data: &EngineStateData) -> Option<String> {
        let current_index = data.current_entry_id.as_ref().and_then(|id| data.queue.iter().position(|entry| &entry.entry_id == id))?;
        let order: Vec<&String> = if data.shuffle && !data.shuffle_order.is_empty() {
            data.shuffle_order.iter().collect()
        } else {
            data.queue.iter().map(|entry| &entry.entry_id).collect()
        };
        let position_in_order = order.iter().position(|id| *id == data.current_entry_id.as_ref().unwrap())?;
        order
            .into_iter()
            .skip(position_in_order + 1)
            .find(|id| {
                data.queue
                    .iter()
                    .find(|entry| &entry.entry_id == *id)
                    .map(|entry| entry.status != EntryStatus::Unavailable && entry.availability_state == "available")
                    .unwrap_or(false)
            })
            .cloned()
            .or_else(|| {
                if current_index + 1 < data.queue.len() {
                    None
                } else {
                    None
                }
            })
    }

    fn valid_index_after(&self, data: &EngineStateData, from: usize) -> Option<usize> {
        let order: Vec<String> = if data.shuffle && !data.shuffle_order.is_empty() {
            data.shuffle_order.clone()
        } else {
            data.queue.iter().map(|entry| entry.entry_id.clone()).collect()
        };
        let current_id = data.queue.get(from).map(|entry| entry.entry_id.clone())?;
        let index = order.iter().position(|id| id == &current_id)?;
        order
            .iter()
            .skip(index + 1)
            .filter_map(|id| data.queue.iter().position(|entry| &entry.entry_id == id))
            .find(|index| {
                let entry = &data.queue[*index];
                entry.status != EntryStatus::Unavailable && entry.availability_state == "available"
            })
    }

    fn wrap_first_index(&self, data: &EngineStateData) -> Option<usize> {
        let order: Vec<String> = if data.shuffle && !data.shuffle_order.is_empty() {
            data.shuffle_order.clone()
        } else {
            data.queue.iter().map(|entry| entry.entry_id.clone()).collect()
        };
        order
            .iter()
            .filter_map(|id| data.queue.iter().position(|entry| &entry.entry_id == id))
            .find(|index| {
                let entry = &data.queue[*index];
                entry.status != EntryStatus::Unavailable && entry.availability_state == "available"
            })
    }

    fn record_history_start(&self, data: &EngineStateData, entry: &QueueEntry) -> Option<i64> {
        let connection = self.db();
        let token = data.current_token.as_deref().unwrap_or("");
        connection
            .execute(
                "INSERT INTO playback_history(media_file_id, entry_id, session_id, position_seconds, completion_state)
                 VALUES (?1, ?2, ?3, ?4, 'started')",
                params![entry.media_file_id, entry.entry_id, token, data.position_seconds],
            )
            .ok()?;
        Some(connection.last_insert_rowid())
    }

    fn begin_entry(&self, entry_index: usize, position_seconds: f64, effects: &mut Vec<EngineEffect>) {
        let mut data = self.lock();
        let Some(entry) = data.queue.get(entry_index).cloned() else { return };
        if entry.availability_state != "available" {
            data.queue[entry_index].status = EntryStatus::Unavailable;
            data.last_error = Some(format!("{} is unavailable and was skipped.", entry.filename));
            effects.push(EngineEffect::Notify { level: "warning".to_string(), message: data.last_error.clone().unwrap() });
            let next_playable = data
                .queue
                .iter()
                .enumerate()
                .skip(entry_index + 1)
                .find(|(_, item)| item.availability_state == "available" && item.status != EntryStatus::Unavailable)
                .map(|(index, _)| index);
            self.persist(&data);
            drop(data);
            if let Some(next_playable) = next_playable {
                self.begin_entry(next_playable, position_seconds, effects);
            } else {
                let mut data = self.lock();
                data.state = EngineState::Error;
                data.message = Some("No playable videos remain in the queue.".to_string());
                self.persist(&data);
            }
            return;
        }
        for item in data.queue.iter_mut() {
            if item.status == EntryStatus::Playing { item.status = EntryStatus::Pending; }
        }
        let token = Uuid::new_v4().to_string();
        data.current_entry_id = Some(entry.entry_id.clone());
        data.current_token = Some(token.clone());
        data.position_seconds = position_seconds.max(0.0);
        data.duration_seconds = entry.duration_seconds;
        data.state = EngineState::Loading;
        data.message = Some(format!("Loading {}", entry.filename));
        data.last_error = None;
        data.queue[entry_index].status = EntryStatus::Playing;
        data.history_stack.retain(|id| id != &entry.entry_id);
        data.history_stack.push(entry.entry_id.clone());
        self.record_history_start(&data, &entry);
        effects.push(EngineEffect::Load {
            media_file_id: entry.media_file_id,
            entry_id: entry.entry_id,
            playback_token: token,
            position_seconds: data.position_seconds,
            filename: entry.filename,
        });
        self.persist(&data);
    }

    fn finish_current(&self, completion: bool, _effects: &mut Vec<EngineEffect>) {
        let data = self.lock();
        let Some(entry_id) = data.current_entry_id.clone() else { return };
        let token = data.current_token.clone().unwrap_or_default();
        let connection = self.db();
        let _ = connection.execute(
            "UPDATE playback_history SET watched_seconds = MAX(watched_seconds, ?1), position_seconds = ?2,
                completion_state = ?3 WHERE session_id = ?4 AND session_id <> ''",
            params![data.position_seconds.floor() as i64, data.position_seconds, if completion { "completed" } else { "stopped" }, token],
        );
        drop(connection);
        drop(data);
        let mut data = self.lock();
        if let Some(entry) = data.queue.iter_mut().find(|entry| entry.entry_id == entry_id) {
            if entry.status == EntryStatus::Playing { entry.status = if completion { EntryStatus::Completed } else { EntryStatus::Pending }; }
        }
        data.current_token = None;
        self.persist(&data);
    }

    // -----------------------------------------------------------------------
    // Playlist operations
    // -----------------------------------------------------------------------

    pub fn list_playlists(&self) -> Result<Vec<PlaylistSummary>, String> {
        let connection = self.db();
        let mut statement = connection
            .prepare(
                "SELECT p.id, p.name, p.created_at, p.updated_at, COUNT(i.id), SUM(f.duration_seconds)
                 FROM playlists p LEFT JOIN playlist_items i ON i.playlist_id = p.id
                 LEFT JOIN media_files f ON f.id = i.media_file_id
                 GROUP BY p.id ORDER BY p.name COLLATE NOCASE",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(PlaylistSummary {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    created_at: row.get(2)?,
                    updated_at: row.get(3)?,
                    item_count: row.get(4)?,
                    duration_seconds: row.get(5)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())
    }

    pub fn create_playlist(&self, name: String) -> Result<i64, String> {
        let name = name.trim();
        if name.is_empty() { return Err("Enter a playlist name.".to_string()); }
        if name.chars().count() > 120 { return Err("Playlist names must be 120 characters or fewer.".to_string()); }
        let connection = self.db();
        connection
            .execute("INSERT INTO playlists(name) VALUES (?1)", [name])
            .map_err(|error| format!("Could not create the playlist: {error}"))?;
        Ok(connection.last_insert_rowid())
    }

    pub fn rename_playlist(&self, playlist_id: i64, name: String) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() { return Err("Enter a playlist name.".to_string()); }
        if name.chars().count() > 120 { return Err("Playlist names must be 120 characters or fewer.".to_string()); }
        let connection = self.db();
        let changed = connection
            .execute("UPDATE playlists SET name=?1, updated_at=CURRENT_TIMESTAMP WHERE id=?2", params![name, playlist_id])
            .map_err(|error| format!("Could not rename the playlist: {error}"))?;
        if changed == 0 { return Err("That playlist no longer exists.".to_string()); }
        Ok(())
    }

    pub fn delete_playlist(&self, playlist_id: i64) -> Result<(), String> {
        // The database guard must be released before persist() re-locks it.
        let changed = {
            let connection = self.db();
            // playlist_items are ON DELETE CASCADE; media files are never touched.
            connection
                .execute("DELETE FROM playlists WHERE id=?1", [playlist_id])
                .map_err(|error| format!("Could not delete the playlist: {error}"))?
        };
        if changed == 0 { return Err("That playlist no longer exists.".to_string()); }
        let mut data = self.lock();
        if data.last_selected_playlist_id == Some(playlist_id) {
            data.last_selected_playlist_id = None;
            self.persist(&data);
        }
        Ok(())
    }

    pub fn select_playlist(&self, playlist_id: i64) -> Result<(), String> {
        // The database guard must be released before persist() re-locks it.
        let exists = {
            let connection = self.db();
            connection
                .query_row("SELECT 1 FROM playlists WHERE id=?1", [playlist_id], |_| Ok(()))
                .optional()
                .map_err(|error| error.to_string())?
        };
        if exists.is_none() { return Err("That playlist no longer exists.".to_string()); }
        let mut data = self.lock();
        data.last_selected_playlist_id = Some(playlist_id);
        self.persist(&data);
        Ok(())
    }

    pub fn list_playlist_items(&self, playlist_id: i64) -> Result<Vec<PlaylistItem>, String> {
        let connection = self.db();
        let mut statement = connection
            .prepare(
                "SELECT i.id, i.media_file_id, COALESCE(f.filename, '(missing file)'),
                        COALESCE(f.availability_state, 'permanently_missing'), f.duration_seconds, i.position
                 FROM playlist_items i LEFT JOIN media_files f ON f.id = i.media_file_id
                 WHERE i.playlist_id = ?1 ORDER BY i.position, i.id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([playlist_id], |row| {
                Ok(PlaylistItem {
                    id: row.get(0)?,
                    media_file_id: row.get(1)?,
                    filename: row.get(2)?,
                    availability_state: row.get(3)?,
                    duration_seconds: row.get(4)?,
                    position: row.get(5)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())
    }

    pub fn add_playlist_items(&self, playlist_id: i64, media_file_ids: &[i64]) -> Result<usize, String> {
        if media_file_ids.is_empty() { return Ok(0); }
        let connection = self.db();
        let exists = connection.query_row("SELECT 1 FROM playlists WHERE id=?1", [playlist_id], |_| Ok(())).optional().map_err(|e| e.to_string())?;
        if exists.is_none() { return Err("That playlist no longer exists.".to_string()); }
        let transaction = connection.unchecked_transaction().map_err(|error| error.to_string())?;
        let mut added = 0usize;
        for media_file_id in media_file_ids {
            let valid: Option<i64> = transaction.query_row("SELECT id FROM media_files WHERE id=?1", [*media_file_id], |row| row.get(0)).optional().map_err(|e| e.to_string())?;
            if valid.is_none() { continue; }
            let duplicate: Option<i64> = transaction.query_row("SELECT id FROM playlist_items WHERE playlist_id=?1 AND media_file_id=?2", params![playlist_id, media_file_id], |row| row.get(0)).optional().map_err(|e| e.to_string())?;
            if duplicate.is_some() { continue; }
            let next: i64 = transaction.query_row("SELECT COALESCE(MAX(position), -1) + 1 FROM playlist_items WHERE playlist_id=?1", [playlist_id], |row| row.get(0)).map_err(|e| e.to_string())?;
            transaction.execute("INSERT INTO playlist_items(playlist_id, media_file_id, position) VALUES (?1, ?2, ?3)", params![playlist_id, media_file_id, next]).map_err(|e| e.to_string())?;
            added += 1;
        }
        transaction.execute("UPDATE playlists SET updated_at=CURRENT_TIMESTAMP WHERE id=?1", [playlist_id]).map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(added)
    }

    pub fn remove_playlist_item(&self, item_id: i64) -> Result<(), String> {
        let connection = self.db();
        let playlist_id: Option<i64> = connection.query_row("SELECT playlist_id FROM playlist_items WHERE id=?1", [item_id], |row| row.get(0)).optional().map_err(|e| e.to_string())?;
        let Some(playlist_id) = playlist_id else { return Err("That playlist item no longer exists.".to_string()); };
        let transaction = connection.unchecked_transaction().map_err(|e| e.to_string())?;
        transaction.execute("DELETE FROM playlist_items WHERE id=?1", [item_id]).map_err(|e| e.to_string())?;
        let items: Vec<(i64, i32)> = transaction.prepare("SELECT id, position FROM playlist_items WHERE playlist_id=?1 ORDER BY position").map_err(|e| e.to_string())?
            .query_map([playlist_id], |row| Ok((row.get(0)?, row.get(1)?))).map_err(|e| e.to_string())?.filter_map(Result::ok).collect();
        for (index, (id, _)) in items.iter().enumerate() {
            transaction.execute("UPDATE playlist_items SET position=?1 WHERE id=?2", params![index as i32, id]).map_err(|e| e.to_string())?;
        }
        transaction.execute("UPDATE playlists SET updated_at=CURRENT_TIMESTAMP WHERE id=?1", [playlist_id]).map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn reorder_playlist_item(&self, item_id: i64, target_position: i32) -> Result<(), String> {
        if target_position < 0 { return Err("Invalid playlist position.".to_string()); }
        let connection = self.db();
        let playlist_id: Option<i64> = connection.query_row("SELECT playlist_id FROM playlist_items WHERE id=?1", [item_id], |row| row.get(0)).optional().map_err(|e| e.to_string())?;
        let Some(playlist_id) = playlist_id else { return Err("That playlist item no longer exists.".to_string()); };
        let transaction = connection.unchecked_transaction().map_err(|e| e.to_string())?;
        let mut items: Vec<i64> = transaction.prepare("SELECT id FROM playlist_items WHERE playlist_id=?1 ORDER BY position, id").map_err(|e| e.to_string())?
            .query_map([playlist_id], |row| row.get(0)).map_err(|e| e.to_string())?.filter_map(Result::ok).collect();
        let Some(from) = items.iter().position(|id| id == &item_id) else { return Err("That playlist item no longer exists.".to_string()); };
        let id = items.remove(from);
        let target = (target_position as usize).min(items.len());
        items.insert(target, id);
        // Move every row above the current position range first so the
        // UNIQUE(playlist_id, position) constraint is never hit mid-reorder.
        let offset = items.len() as i32 + 10;
        for (index, id) in items.iter().enumerate() {
            transaction.execute("UPDATE playlist_items SET position=?1 WHERE id=?2", params![offset + index as i32, id]).map_err(|e| e.to_string())?;
        }
        for (index, id) in items.iter().enumerate() {
            transaction.execute("UPDATE playlist_items SET position=?1 WHERE id=?2", params![index as i32, id]).map_err(|e| e.to_string())?;
        }
        transaction.execute("UPDATE playlists SET updated_at=CURRENT_TIMESTAMP WHERE id=?1", [playlist_id]).map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Queue operations
    // -----------------------------------------------------------------------

    fn append_media_ids(&self, media_file_ids: &[i64], mode: AddMode, effects: &mut Vec<EngineEffect>) -> Result<(), String> {
        if media_file_ids.is_empty() { return Ok(()); }
        let mut data = self.lock();
        let mut entries = Vec::new();
        for media_file_id in media_file_ids {
            let Some((filename, availability_state, duration_seconds)) = self.media_details(*media_file_id) else {
                effects.push(EngineEffect::Notify { level: "warning".to_string(), message: format!("Media #{media_file_id} is no longer in the library and was not queued.") });
                continue;
            };
            let status = if availability_state == "available" { EntryStatus::Pending } else { EntryStatus::Unavailable };
            if status == EntryStatus::Unavailable {
                effects.push(EngineEffect::Notify {
                    level: "warning".to_string(),
                    message: format!("{filename} is unavailable and will be skipped during playback."),
                });
            }
            entries.push(QueueEntry {
                entry_id: Uuid::new_v4().to_string(),
                media_file_id: *media_file_id,
                filename,
                availability_state,
                duration_seconds,
                status,
                position: 0,
            });
        }
        let insert_at = match mode {
            AddMode::PlayNext => data.current_entry_id.as_ref()
                .and_then(|id| data.queue.iter().position(|entry| &entry.entry_id == id))
                .map(|index| index + 1)
                .unwrap_or(0),
            _ => data.queue.len(),
        };
        for (offset, entry) in entries.into_iter().enumerate() {
            data.queue.insert(insert_at + offset, entry);
        }
        self.renumber(&mut data);
        self.rebuild_shuffle_order(&mut data);
        self.persist(&data);
        let first_id = match mode {
            AddMode::PlayNext => data.queue.get(insert_at).map(|entry| entry.entry_id.clone()),
            AddMode::PlayNow => data.queue.get(insert_at).map(|entry| entry.entry_id.clone()),
            AddMode::Append => None,
        };
        drop(data);
        if let (AddMode::PlayNow, Some(id)) = (mode, first_id) {
            let index = self.lock().queue.iter().position(|entry| entry.entry_id == id);
            if let Some(index) = index { self.begin_entry(index, 0.0, effects); }
        }
        Ok(())
    }

    pub fn queue_add(&self, media_file_ids: Vec<i64>, mode: AddMode) -> Result<Vec<EngineEffect>, String> {
        let mut effects = Vec::new();
        self.append_media_ids(&media_file_ids, mode, &mut effects)?;
        Ok(effects)
    }

    pub fn queue_add_playlist(&self, playlist_id: i64, mode: AddMode) -> Result<Vec<EngineEffect>, String> {
        let ids = self.list_playlist_items(playlist_id)?.into_iter().map(|item| item.media_file_id).collect::<Vec<_>>();
        let mut effects = self.queue_add(ids, mode)?;
        effects.push(EngineEffect::Notify { level: "info".to_string(), message: "Playlist added to the queue.".to_string() });
        Ok(effects)
    }

    pub fn queue_remove(&self, entry_id: String) -> Result<Vec<EngineEffect>, String> {
        let mut data = self.lock();
        let index = data.queue.iter().position(|entry| entry.entry_id == entry_id).ok_or("That queue item no longer exists.")?;
        let was_current = data.current_entry_id.as_ref() == Some(&entry_id);
        data.queue.remove(index);
        self.renumber(&mut data);
        self.rebuild_shuffle_order(&mut data);
        if was_current {
            data.current_entry_id = None;
            data.current_token = None;
            data.position_seconds = 0.0;
            data.duration_seconds = None;
            data.state = EngineState::Idle;
        }
        self.persist(&data);
        let mut effects = vec![EngineEffect::Notify { level: "info".to_string(), message: "Removed from the queue.".to_string() }];
        if was_current { effects.push(EngineEffect::Stop); }
        Ok(effects)
    }

    pub fn queue_reorder(&self, entry_id: String, target_position: usize) -> Result<Vec<EngineEffect>, String> {
        let mut data = self.lock();
        let from = data.queue.iter().position(|entry| entry.entry_id == entry_id).ok_or("That queue item no longer exists.")?;
        let entry = data.queue.remove(from);
        let target = target_position.min(data.queue.len());
        data.queue.insert(target, entry);
        self.renumber(&mut data);
        self.rebuild_shuffle_order(&mut data);
        self.persist(&data);
        Ok(vec![])
    }

    pub fn queue_clear(&self) -> Result<Vec<EngineEffect>, String> {
        let mut data = self.lock();
        data.queue.clear();
        data.current_entry_id = None;
        data.current_token = None;
        data.position_seconds = 0.0;
        data.duration_seconds = None;
        data.shuffle_order.clear();
        data.history_stack.clear();
        data.state = EngineState::Idle;
        data.message = None;
        data.last_error = None;
        self.persist(&data);
        Ok(vec![
            EngineEffect::Stop,
            EngineEffect::Notify { level: "info".to_string(), message: "Queue cleared.".to_string() },
        ])
    }

    pub fn set_mode(&self, mode: PlaybackMode) -> Result<Vec<EngineEffect>, String> {
        let mut data = self.lock();
        data.mode = mode;
        if data.state == EngineState::WaitingForManualSelection && mode == PlaybackMode::Auto {
            data.state = EngineState::Completed;
        }
        self.persist(&data);
        Ok(vec![EngineEffect::Notify { level: "info".to_string(), message: format!("Playback mode: {}.", if mode == PlaybackMode::Auto { "Automatic" } else { "Manual" }) }])
    }

    pub fn set_shuffle(&self, shuffle: bool) -> Result<Vec<EngineEffect>, String> {
        let mut data = self.lock();
        data.shuffle = shuffle;
        self.rebuild_shuffle_order(&mut data);
        self.persist(&data);
        Ok(vec![EngineEffect::Notify { level: "info".to_string(), message: if shuffle { "Shuffle on. The current video stays in place." } else { "Shuffle off. Queue order restored." }.to_string() }])
    }

    pub fn set_repeat(&self, repeat: RepeatMode) -> Result<Vec<EngineEffect>, String> {
        let mut data = self.lock();
        data.repeat = repeat;
        self.persist(&data);
        Ok(vec![EngineEffect::Notify { level: "info".to_string(), message: match repeat {
            RepeatMode::Off => "Repeat off.".to_string(),
            RepeatMode::One => "Repeat current video.".to_string(),
            RepeatMode::All => "Repeat entire queue.".to_string(),
        } }])
    }

    pub fn play_now(&self, entry_id: String) -> Result<Vec<EngineEffect>, String> {
        let index = self.lock().queue.iter().position(|entry| entry.entry_id == entry_id).ok_or("That queue item no longer exists.")?;
        let mut effects = Vec::new();
        self.begin_entry(index, 0.0, &mut effects);
        Ok(effects)
    }

    pub fn play_media_now(&self, media_file_id: i64) -> Result<Vec<EngineEffect>, String> {
        let effects = self.queue_add(vec![media_file_id], AddMode::PlayNow)?;
        Ok(effects)
    }

    pub fn next(&self) -> Result<Vec<EngineEffect>, String> {
        let mut effects = Vec::new();
        let (index, repeat, mode) = {
            let data = self.lock();
            let index = data.current_entry_id.as_ref().and_then(|id| data.queue.iter().position(|entry| &entry.entry_id == id));
            (index, data.repeat, data.mode)
        };
        let Some(index) = index else {
            if let Some(second) = { let data = self.lock(); self.wrap_first_index(&data) } {
                self.begin_entry(second, 0.0, &mut effects);
            }
            return Ok(effects);
        };
        let next = { let data = self.lock(); self.valid_index_after(&data, index) };
        if let Some(next) = next {
            self.begin_entry(next, 0.0, &mut effects);
        } else if repeat == RepeatMode::All {
            let first = { let data = self.lock(); self.wrap_first_index(&data) };
            if let Some(first) = first { self.begin_entry(first, 0.0, &mut effects); }
        } else {
            let mut data = self.lock();
            data.state = if mode == PlaybackMode::Manual { EngineState::WaitingForManualSelection } else { EngineState::Completed };
            data.message = Some("End of queue.".to_string());
            effects.push(EngineEffect::Notify { level: "info".to_string(), message: "End of queue.".to_string() });
            self.persist(&data);
        }
        Ok(effects)
    }

    pub fn previous(&self) -> Result<Vec<EngineEffect>, String> {
        let mut effects = Vec::new();
        let (current_id, position, threshold) = {
            let data = self.lock();
            (data.current_entry_id.clone(), data.position_seconds, data.restart_threshold)
        };
        if position >= threshold {
            if let Some(id) = current_id {
                let index = { self.lock().queue.iter().position(|entry| entry.entry_id == id) };
                if let Some(index) = index {
                    self.begin_entry(index, 0.0, &mut effects);
                    effects.push(EngineEffect::Notify { level: "info".to_string(), message: "Restarted the current video.".to_string() });
                    return Ok(effects);
                }
            }
        }
        let previous = {
            let data = self.lock();
            data.history_stack.iter().rev().nth(1).and_then(|id| data.queue.iter().position(|entry| &entry.entry_id == id))
        };
        if let Some(index) = previous {
            self.begin_entry(index, 0.0, &mut effects);
        } else {
            effects.push(EngineEffect::Notify { level: "info".to_string(), message: "No earlier item in playback history.".to_string() });
        }
        Ok(effects)
    }

    /// Explicit user recovery: reload the current item at its last position.
    pub fn recover(&self) -> Result<Vec<EngineEffect>, String> {
        let index = {
            let data = self.lock();
            data.current_entry_id.as_ref().and_then(|id| data.queue.iter().position(|entry| &entry.entry_id == id))
        };
        let Some(index) = index else {
            return Err("There is no previous item to recover. Choose a video first.".to_string());
        };
        let mut effects = Vec::new();
        let position = self.lock().position_seconds;
        self.begin_entry(index, position, &mut effects);
        effects.push(EngineEffect::Notify { level: "info".to_string(), message: "Reconnecting and resuming playback.".to_string() });
        Ok(effects)
    }

    // -----------------------------------------------------------------------
    // Cast event processing (the only progression trigger)
    // -----------------------------------------------------------------------

    pub fn on_cast_event(&self, event: CastStatusEvent) -> Vec<EngineEffect> {
        let mut effects = Vec::new();
        let kind = event.kind.as_str();
        if kind == "session_started" || kind == "session_resumed" {
            let mut data = self.lock();
            data.cast_session_id = event.cast_session_id.clone();
            let was_recovering = data.state == EngineState::Recovering || data.state == EngineState::Disconnected;
            if data.current_entry_id.is_some() && was_recovering {
                data.state = EngineState::Loading;
            }
            self.persist(&data);
            return effects;
        }
        if kind == "session_ended" {
            let mut data = self.lock();
            if data.current_entry_id.is_some() && !matches!(data.state, EngineState::Completed | EngineState::WaitingForManualSelection | EngineState::Idle) {
                data.state = EngineState::Disconnected;
                data.message = Some("Cast receiver disconnected. The queue is preserved.".to_string());
                effects.push(EngineEffect::Notify { level: "warning".to_string(), message: "Cast disconnected. Queue preserved — use Reconnect & resume when ready.".to_string() });
            }
            data.cast_session_id = None;
            data.media_session_id = None;
            data.current_token = None;
            self.persist(&data);
            return effects;
        }
        if kind == "load_failed" {
            let token_matches = event.playback_token.as_deref()
                .map(|token| self.lock().current_token.as_deref() == Some(token))
                .unwrap_or(false);
            if token_matches {
                let mut data = self.lock();
                let current_entry_id = data.current_entry_id.clone();
                if let Some(entry) = data.queue.iter_mut().find(|entry| Some(&entry.entry_id) == current_entry_id.as_ref()) {
                    entry.status = EntryStatus::Unavailable;
                }
                data.state = EngineState::Error;
                data.last_error = Some(event.message.clone().unwrap_or_else(|| "The receiver could not load this video.".to_string()));
                effects.push(EngineEffect::Notify { level: "error".to_string(), message: data.last_error.clone().unwrap() });
                let mode = data.mode;
                self.persist(&data);
                if mode == PlaybackMode::Auto { self.next_inside_completion(&mut effects); }
            }
            return effects;
        }
        if kind == "media_completed" {
            let mut data = self.lock();
            let Some(token) = data.current_token.clone() else { return effects };
            if event.playback_token.as_deref() != Some(token.as_str()) { return effects; }
            // A completion is trusted only when BOTH sides agree on the Cast
            // session and the media session. The token prevents stale loads,
            // while these ids prevent a different receiver/session event from
            // completing the current queue entry.
            if data.cast_session_id.as_deref() != event.cast_session_id.as_deref() { return effects; }
            if data.media_session_id != event.media_session_id { return effects; }
            if data.last_completion_token.as_deref() == Some(token.as_str()) { return effects; }
            // Position from the receiver is authoritative for this item.
            if let Some(position) = event.position_seconds.filter(|value| value.is_finite() && *value >= 0.0) {
                data.position_seconds = position;
            }
            let repeat = data.repeat;
            let mode = data.mode;
            let current_entry_id = data.current_entry_id.clone();
            drop(data);
            {
                let mut data = self.lock();
                data.last_completion_token = Some(token);
                data.state = EngineState::Completed;
                data.message = Some("Video completed.".to_string());
            }
            self.finish_current(true, &mut effects);
            effects.push(EngineEffect::Notify { level: "success".to_string(), message: "Video completed.".to_string() });
            match repeat {
                RepeatMode::One => {
                    if let Some(id) = current_entry_id {
                        let index = { self.lock().queue.iter().position(|entry| entry.entry_id == id) };
                        if let Some(index) = index {
                            self.begin_entry(index, 0.0, &mut effects);
                        }
                    }
                }
                _ if mode == PlaybackMode::Auto => self.next_inside_completion(&mut effects),
                _ => {
                    let mut data = self.lock();
                    data.state = EngineState::WaitingForManualSelection;
                    data.message = Some("Waiting for manual selection.".to_string());
                    effects.push(EngineEffect::Notify { level: "info".to_string(), message: "Waiting for manual selection.".to_string() });
                    self.persist(&data);
                }
            }
            return effects;
        }
        if kind == "media_status" {
            // Status without the current token belongs to a previous load and
            // is ignored entirely.
            let token_matches = event.playback_token.as_deref()
                .map(|token| self.lock().current_token.as_deref() == Some(token))
                .unwrap_or(false);
            if !token_matches { return effects; }
            let mut data = self.lock();
            if let Some(position) = event.position_seconds.filter(|value| value.is_finite() && *value >= 0.0) {
                data.position_seconds = position;
            }
            if let Some(duration) = event.duration_seconds.filter(|value| value.is_finite() && *value > 0.0) {
                data.duration_seconds = Some(duration);
                if let Some(id) = data.current_media_file_id() {
                    let connection = self.db();
                    let _ = connection.execute("UPDATE media_files SET duration_seconds=?1 WHERE id=?2", params![duration, id]);
                }
            }
            if let Some(session_id) = event.cast_session_id { data.cast_session_id = Some(session_id); }
            if let Some(media_id) = event.media_session_id { data.media_session_id = Some(media_id); }
            data.state = match event.player_state.as_deref() {
                Some("PLAYING") => EngineState::Playing,
                Some("PAUSED") => EngineState::Paused,
                Some("BUFFERING") => EngineState::Buffering,
                Some("IDLE") | Some("STOPPED") => EngineState::Completed,
                _ => data.state,
            };
            self.persist(&data);
        }
        effects
    }

    fn next_inside_completion(&self, effects: &mut Vec<EngineEffect>) {
        let (index, repeat) = {
            let data = self.lock();
            let index = data.current_entry_id.as_ref().and_then(|id| data.queue.iter().position(|entry| &entry.entry_id == id));
            (index, data.repeat)
        };
        let Some(index) = index else { return };
        if let Some(next) = { let data = self.lock(); self.valid_index_after(&data, index) } {
            self.begin_entry(next, 0.0, effects);
            effects.push(EngineEffect::Notify { level: "info".to_string(), message: "Playing next video.".to_string() });
            return;
        }
        if repeat == RepeatMode::All {
            if let Some(first) = { let data = self.lock(); self.wrap_first_index(&data) } {
                self.begin_entry(first, 0.0, effects);
                effects.push(EngineEffect::Notify { level: "info".to_string(), message: "Playing next video.".to_string() });
                return;
            }
        }
        let mut data = self.lock();
        data.state = if data.mode == PlaybackMode::Manual { EngineState::WaitingForManualSelection } else { EngineState::Completed };
        data.message = Some("Queue completed.".to_string());
        effects.push(EngineEffect::Notify { level: "info".to_string(), message: "Queue completed.".to_string() });
        self.persist(&data);
    }
}

impl EngineStateData {
    fn current_media_file_id(&self) -> Option<i64> {
        self.current_entry_id.as_ref().and_then(|id| self.queue.iter().find(|entry| &entry.entry_id == id).map(|entry| entry.media_file_id))
    }
}

/// Convenience for implementing an effect sink without an async trait.
pub type EffectSender = mpsc::UnboundedSender<EngineEffect>;
pub type EffectReceiver = mpsc::UnboundedReceiver<EngineEffect>;
pub fn effect_channel() -> (EffectSender, EffectReceiver) {
    mpsc::unbounded_channel()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::open_database;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(1);

    struct Fixture {
        _temp: tempfile::TempDir,
        engine: PlaybackEngine,
        database: Arc<Mutex<Connection>>,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let database = Arc::new(Mutex::new(open_database(temp.path()).unwrap()));
            database
                .lock()
                .unwrap()
                .execute(
                    "INSERT INTO media_directories(id, path, is_active, availability_state) VALUES (1, 'C:\\media', 1, 'available')",
                    [],
                )
                .unwrap();
            let engine = PlaybackEngine::new(Arc::clone(&database)).unwrap();
            Self { _temp: temp, engine, database }
        }

        fn add_media(&self, name: &str, duration: Option<f64>, availability: &str) -> i64 {
            let id = NEXT_ID.fetch_add(1, Ordering::SeqCst) as i64 + 10_000;
            self.database
                .lock()
                .unwrap()
                .execute(
                    "INSERT INTO media_files(id, directory_id, path, filename, extension, size_bytes, modified_at, indexed_at, availability_state, duration_seconds)
                     VALUES (?1, 1, ?2, ?3, 'mp4', 100, '2026-01-01 00:00:00', '2026-01-01 00:00:00', ?4, ?5)",
                    params![id, format!("C:\\media\\{name}.mp4"), name, availability, duration],
                )
                .unwrap();
            id
        }

        fn media_ids(&self, count: usize) -> Vec<i64> {
            (0..count)
                .map(|index| self.add_media(&format!("video{index}"), Some(120.0 + index as f64), "available"))
                .collect()
        }

        fn load_effect(effects: &[EngineEffect]) -> (i64, String, String) {
            effects
                .iter()
                .find_map(|effect| match effect {
                    EngineEffect::Load { media_file_id, entry_id, playback_token, .. } => {
                        Some((*media_file_id, entry_id.clone(), playback_token.clone()))
                    }
                    _ => None,
                })
                .expect("expected a Load effect")
        }

        fn completes(token: &str) -> CastStatusEvent {
            CastStatusEvent {
                kind: "media_completed".to_string(),
                playback_token: Some(token.to_string()),
                cast_session_id: Some("receiver-1".to_string()),
                media_session_id: Some(7),
                position_seconds: Some(120.0),
                duration_seconds: Some(120.0),
                idle_reason: Some("FINISHED".to_string()),
                player_state: Some("IDLE".to_string()),
                message: None,
            }
        }

        fn status(token: &str, state: &str, position: f64) -> CastStatusEvent {
            CastStatusEvent {
                kind: "media_status".to_string(),
                playback_token: Some(token.to_string()),
                cast_session_id: Some("receiver-1".to_string()),
                media_session_id: Some(7),
                player_state: Some(state.to_string()),
                position_seconds: Some(position),
                duration_seconds: Some(120.0),
                idle_reason: None,
                message: None,
            }
        }
    }

    #[test]
    fn playlist_crud_preserves_media_files() {
        let fixture = Fixture::new();
        let ids = fixture.media_ids(3);
        let playlist_id = fixture.engine.create_playlist("Favorites".to_string()).unwrap();
        assert_eq!(fixture.engine.add_playlist_items(playlist_id, &ids).unwrap(), 3);
        // Duplicates are ignored.
        assert_eq!(fixture.engine.add_playlist_items(playlist_id, &ids[..1]).unwrap(), 0);
        let items = fixture.engine.list_playlist_items(playlist_id).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].position, 0);
        fixture.engine.reorder_playlist_item(items[2].id, 0).unwrap();
        assert_eq!(fixture.engine.list_playlist_items(playlist_id).unwrap()[0].media_file_id, ids[2]);
        fixture.engine.rename_playlist(playlist_id, "Renamed".to_string()).unwrap();
        assert_eq!(fixture.engine.list_playlists().unwrap()[0].name, "Renamed");
        assert!(fixture.engine.list_playlists().unwrap()[0].duration_seconds.is_some());
        fixture.engine.delete_playlist(playlist_id).unwrap();
        assert!(fixture.engine.list_playlists().unwrap().is_empty());
        let remaining: i64 = fixture
            .database
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM media_files WHERE id IN (?1, ?2, ?3)", params![ids[0], ids[1], ids[2]], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 3, "deleting a playlist must never delete media files");
    }

    #[test]
    fn selecting_and_deleting_the_selected_playlist_do_not_deadlock() {
        let fixture = Fixture::new();
        let ids = fixture.media_ids(2);
        let first = fixture.engine.create_playlist("First".to_string()).unwrap();
        let second = fixture.engine.create_playlist("Second".to_string()).unwrap();
        fixture.engine.add_playlist_items(first, &ids).unwrap();

        fixture.engine.select_playlist(first).unwrap();
        assert_eq!(fixture.engine.snapshot().last_selected_playlist_id, Some(first));
        fixture.engine.select_playlist(second).unwrap();
        assert_eq!(fixture.engine.snapshot().last_selected_playlist_id, Some(second));
        fixture.engine.select_playlist(first).unwrap();
        fixture.engine.delete_playlist(first).unwrap();
        assert_eq!(fixture.engine.snapshot().last_selected_playlist_id, None);
        assert!(fixture.engine.select_playlist(first).is_err());
        assert_eq!(fixture.engine.list_playlists().unwrap().len(), 1);
        let remaining: i64 = fixture
            .database
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM media_files WHERE id IN (?1, ?2)", params![ids[0], ids[1]], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 2, "playlist operations must never delete media files");
    }

    #[test]
    fn queue_add_play_now_play_next_and_reorder() {
        let fixture = Fixture::new();
        let ids = fixture.media_ids(4);
        fixture.engine.queue_add(ids.clone(), AddMode::Append).unwrap();
        let snapshot = fixture.engine.snapshot();
        assert_eq!(snapshot.queue.len(), 4);
        let unique: std::collections::HashSet<_> = snapshot.queue.iter().map(|entry| &entry.entry_id).collect();
        assert_eq!(unique.len(), 4);
        fixture.engine.queue_add(vec![ids[0]], AddMode::PlayNext).unwrap();
        assert_eq!(fixture.engine.snapshot().queue.len(), 5);
        let effects = fixture.engine.play_now(snapshot.queue[2].entry_id.clone()).unwrap();
        let (media_id, entry_id, _) = Fixture::load_effect(&effects);
        assert_eq!(media_id, ids[2]);
        assert_eq!(fixture.engine.snapshot().state, EngineState::Loading);
        let before_next = fixture.engine.snapshot();
        fixture.engine.queue_add(vec![ids[3]], AddMode::PlayNext).unwrap();
        let after_next = fixture.engine.snapshot();
        let current_index = after_next.queue.iter().position(|entry| entry.entry_id == entry_id).unwrap();
        assert_eq!(after_next.queue[current_index + 1].media_file_id, ids[3]);
        assert_eq!(before_next.queue.len() + 1, after_next.queue.len());
        let target = after_next.queue.last().unwrap().entry_id.clone();
        fixture.engine.queue_reorder(target.clone(), 0).unwrap();
        assert_eq!(fixture.engine.snapshot().queue[0].entry_id, target);
        fixture.engine.queue_remove(target.clone()).unwrap();
        assert!(!fixture.engine.snapshot().queue.iter().any(|entry| entry.entry_id == target));
        fixture.engine.queue_clear().unwrap();
        assert!(fixture.engine.snapshot().queue.is_empty());
    }

    #[test]
    fn automatic_completion_advances_only_on_correlated_real_event() {
        let fixture = Fixture::new();
        let ids = fixture.media_ids(3);
        fixture.engine.queue_add(ids.clone(), AddMode::Append).unwrap();
        fixture.engine.set_mode(PlaybackMode::Auto).unwrap();
        let first = fixture.engine.snapshot().queue[0].entry_id.clone();
        let effects = fixture.engine.play_now(first.clone()).unwrap();
        let (_, _, token) = Fixture::load_effect(&effects);

        // Wrong token and mismatched session must never advance.
        fixture.engine.on_cast_event(Fixture::completes("stale-token"));
        let mut stale = Fixture::completes(&token);
        stale.cast_session_id = Some("different-receiver".to_string());
        fixture.engine.on_cast_event(stale);
        assert_eq!(fixture.engine.snapshot().current_entry_id, Some(first.clone()));

        // A repeated event for the same load must not advance twice.
        fixture.engine.on_cast_event(Fixture::status(&token, "PLAYING", 40.0));
        fixture.engine.on_cast_event(Fixture::completes(&token));
        let snapshot = fixture.engine.snapshot();
        assert_eq!(snapshot.current_entry_id.as_ref().unwrap(), &snapshot.queue[1].entry_id);
        let second_token = fixture.engine.snapshot().playback_token.clone().unwrap();
        let effects = fixture.engine.on_cast_event(Fixture::completes(&token));
        assert!(effects.is_empty());
        assert_eq!(fixture.engine.snapshot().playback_token, Some(second_token));
    }

    #[test]
    fn manual_mode_waits_and_can_switch_during_playback() {
        let fixture = Fixture::new();
        let ids = fixture.media_ids(2);
        fixture.engine.queue_add(ids, AddMode::Append).unwrap();
        let first = fixture.engine.snapshot().queue[0].entry_id.clone();
        let effects = fixture.engine.play_now(first).unwrap();
        let (_, _, token) = Fixture::load_effect(&effects);
        fixture.engine.on_cast_event(Fixture::status(&token, "PLAYING", 0.0));
        fixture.engine.on_cast_event(Fixture::completes(&token));
        assert_eq!(fixture.engine.snapshot().state, EngineState::WaitingForManualSelection);
        // Switching to Automatic at the wait point leaves the completed item
        // as the current entry; the user chooses the next one explicitly.
        fixture.engine.set_mode(PlaybackMode::Auto).unwrap();
        assert_eq!(fixture.engine.snapshot().mode, PlaybackMode::Auto);
        assert_eq!(fixture.engine.snapshot().state, EngineState::Completed);
    }

    #[test]
    fn repeat_one_reloads_and_repeat_all_wraps() {
        let fixture = Fixture::new();
        let ids = fixture.media_ids(2);
        fixture.engine.queue_add(ids.clone(), AddMode::Append).unwrap();
        let first = fixture.engine.snapshot().queue[0].entry_id.clone();
        let effects = fixture.engine.play_now(first.clone()).unwrap();
        let (_, _, token) = Fixture::load_effect(&effects);
        fixture.engine.on_cast_event(Fixture::status(&token, "PLAYING", 0.0));
        fixture.engine.set_repeat(RepeatMode::One).unwrap();
        let effects = fixture.engine.on_cast_event(Fixture::completes(&token));
        let (media_id, _, _) = Fixture::load_effect(&effects);
        assert_eq!(media_id, ids[0]);
        assert_eq!(fixture.engine.snapshot().current_entry_id, Some(first));

        let second_token = fixture.engine.snapshot().playback_token.clone().unwrap();
        fixture.engine.set_mode(PlaybackMode::Auto).unwrap();
        fixture.engine.set_repeat(RepeatMode::All).unwrap();
        let effects = fixture.engine.on_cast_event(Fixture::completes(&second_token));
        let (media_id, _, _) = Fixture::load_effect(&effects);
        assert_eq!(media_id, ids[1]);
    }

    #[test]
    fn shuffle_preserves_current_and_previous_follows_real_history() {
        let fixture = Fixture::new();
        let ids = fixture.media_ids(4);
        fixture.engine.queue_add(ids.clone(), AddMode::Append).unwrap();
        let first = fixture.engine.snapshot().queue[1].entry_id.clone();
        let effects = fixture.engine.play_now(first.clone()).unwrap();
        let (_, _, _token) = Fixture::load_effect(&effects);
        fixture.engine.set_shuffle(true).unwrap();
        assert_eq!(fixture.engine.snapshot().current_entry_id, Some(first.clone()));
        let next_effects = fixture.engine.next().unwrap();
        let (_, second_id, _) = Fixture::load_effect(&next_effects);
        assert_ne!(second_id, first);
        let before = fixture.engine.snapshot().position_seconds;
        assert!(before < PREVIOUS_RESTART_THRESHOLD_SECONDS);
        let previous = fixture.engine.previous().unwrap();
        let (_, restored, _) = Fixture::load_effect(&previous);
        assert_eq!(restored, first, "Previous must follow played history, not random order");
    }

    #[test]
    fn previous_restarts_after_significant_time() {
        let fixture = Fixture::new();
        let ids = fixture.media_ids(2);
        fixture.engine.queue_add(ids.clone(), AddMode::Append).unwrap();
        let first = fixture.engine.snapshot().queue[0].entry_id.clone();
        let effects = fixture.engine.play_now(first.clone()).unwrap();
        let (_, _, token) = Fixture::load_effect(&effects);
        fixture.engine.on_cast_event(Fixture::status(&token, "PLAYING", 45.0));
        let previous = fixture.engine.previous().unwrap();
        let (_, restarted, _) = Fixture::load_effect(&previous);
        assert_eq!(restarted, first);
        assert!(previous.iter().any(|effect| matches!(effect, EngineEffect::Notify { message, .. } if message.contains("Restarted"))));
    }

    #[test]
    fn disconnection_does_not_advance_and_preserves_queue() {
        let fixture = Fixture::new();
        let ids = fixture.media_ids(3);
        fixture.engine.queue_add(ids, AddMode::Append).unwrap();
        let first = fixture.engine.snapshot().queue[0].entry_id.clone();
        let effects = fixture.engine.play_now(first.clone()).unwrap();
        let (_, _, token) = Fixture::load_effect(&effects);
        fixture.engine.on_cast_event(CastStatusEvent {
            kind: "session_ended".to_string(),
            playback_token: Some(token.clone()),
            cast_session_id: Some("receiver-1".to_string()),
            ..Default::default()
        });
        let snapshot = fixture.engine.snapshot();
        assert_eq!(snapshot.state, EngineState::Disconnected);
        assert_eq!(snapshot.queue.len(), 3);
        assert_eq!(snapshot.current_entry_id, Some(first.clone()));
        // A completion that arrives after the disconnect is stale.
        let effects = fixture.engine.on_cast_event(Fixture::completes(&token));
        assert!(effects.is_empty());
        assert_eq!(fixture.engine.snapshot().current_entry_id, Some(first.clone()));
        // Explicit recovery reloads the same entry and position.
        let recover = fixture.engine.recover().unwrap();
        let (_, recovered, _) = Fixture::load_effect(&recover);
        assert_eq!(recovered, first);
    }

    #[test]
    fn unavailable_entries_are_skipped() {
        let fixture = Fixture::new();
        let good = fixture.add_media("good", Some(90.0), "available");
        let missing = fixture.add_media("missing", None, "permanently_missing");
        let effects = fixture.engine.queue_add(vec![missing, good], AddMode::Append).unwrap();
        assert!(effects.iter().any(|effect| matches!(effect, EngineEffect::Notify { level, .. } if level == "warning")));
        let first = fixture.engine.snapshot().queue[0].entry_id.clone();
        let effects = fixture.engine.play_now(first).unwrap();
        assert!(effects.iter().any(|effect| matches!(effect, EngineEffect::Notify { level, .. } if level == "warning")));
        let (media_id, _, _) = Fixture::load_effect(&effects);
        assert_eq!(media_id, good);
    }

    #[test]
    fn restore_keeps_queue_but_never_autoplays() {
        let fixture = Fixture::new();
        let ids = fixture.media_ids(2);
        fixture.engine.queue_add(ids.clone(), AddMode::Append).unwrap();
        let first = fixture.engine.snapshot().queue[0].entry_id.clone();
        let effects = fixture.engine.play_now(first.clone()).unwrap();
        let (_, _, token) = Fixture::load_effect(&effects);
        fixture.engine.on_cast_event(Fixture::status(&token, "PLAYING", 37.0));
        fixture.engine.set_shuffle(true).unwrap();
        fixture.engine.set_mode(PlaybackMode::Auto).unwrap();
        fixture.engine.set_repeat(RepeatMode::All).unwrap();

        let restored = PlaybackEngine::new(Arc::clone(&fixture.database)).unwrap().snapshot();
        assert_eq!(restored.queue.len(), 2);
        assert_eq!(restored.current_entry_id, Some(first));
        assert_eq!(restored.state, EngineState::Recovering);
        assert!(restored.playback_token.is_none(), "old tokens must not survive a restart");
        assert_eq!(restored.mode, PlaybackMode::Auto);
        assert!(restored.shuffle);
        assert_eq!(restored.repeat, RepeatMode::All);
        assert!((restored.position_seconds - 37.0).abs() < 0.01);
    }
}
