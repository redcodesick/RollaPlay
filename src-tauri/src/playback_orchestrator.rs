//! Phase 5 orchestrator.
//!
//! Owns the boundary between the authoritative queue engine and the existing
//! Cast/media-server services. The engine returns declarative effects; this
//! module executes them. It also translates real Cast SDK events into engine
//! inputs. React never owns playback state and never drives progression.

use crate::{
    cast_sender::{CastEventReport, CastSenderManager},
    dlna::{DlnaManager, DlnaPlaybackRequest},
    media_server::MediaServerManager,
    playlist::{CastStatusEvent, EngineEffect, PlaybackEngine, PlaybackSnapshot},
};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

#[derive(Clone)]
pub struct PlaybackOrchestrator {
    app: AppHandle,
    engine: PlaybackEngine,
    server: MediaServerManager,
    cast: CastSenderManager,
    dlna: DlnaManager,
}

impl PlaybackOrchestrator {
    pub fn new(
        app: AppHandle,
        engine: PlaybackEngine,
        server: MediaServerManager,
        cast: CastSenderManager,
        dlna: DlnaManager,
    ) -> Self {
        Self { app, engine, server, cast, dlna }
    }

    pub fn engine(&self) -> &PlaybackEngine {
        &self.engine
    }

    pub fn snapshot(&self) -> PlaybackSnapshot {
        self.engine.snapshot()
    }

    pub fn clear_watch_history(&self) -> Result<(), String> {
        self.engine.clear_watch_history()
    }

    pub fn emit_snapshot(&self) {
        let _ = self.app.emit("phase5-snapshot", self.snapshot());
    }

    /// Called for every `cast-playback-event` emitted by the real Cast bridge.
    /// This is the only path by which receiver state reaches the engine.
    pub fn handle_cast_report(&self, report: CastEventReport) {
        // A real DLNA completion retires the media-scoped grant that the TV
        // used. The correlated DLNA session id prevents stale Cast reports
        // from revoking a live renderer's URL.
        if report.kind.as_str() == "media_completed"
            && self.dlna.is_active()
            && report
                .cast_session_id
                .as_deref()
                .is_some_and(|id| id.starts_with("dlna-"))
        {
            if let Some(media_file_id) = self
                .dlna
                .take_media_id_for_session(report.cast_session_id.as_deref().unwrap_or(""))
            {
                self.server.cast_grants().revoke_media(media_file_id);
            }
        }
        let event = CastStatusEvent {
            kind: report.kind,
            playback_token: report.playback_token,
            cast_session_id: report.cast_session_id,
            media_session_id: report.media_session_id,
            player_state: report.player_state,
            position_seconds: report.position_seconds,
            duration_seconds: report.duration_seconds,
            idle_reason: report.idle_reason,
            message: report.message,
        };
        let effects = self.engine.on_cast_event(event);
        self.emit_snapshot();
        if effects.is_empty() {
            return;
        }
        let orchestrator = self.clone();
        tauri::async_runtime::spawn(async move {
            orchestrator.execute_effects(effects).await;
        });
    }

    /// Executes engine effects through the existing services.
    pub async fn execute_effects(&self, effects: Vec<EngineEffect>) {
        for effect in effects {
            match effect {
                EngineEffect::Load {
                    media_file_id,
                    entry_id: _,
                    playback_token,
                    position_seconds,
                    filename,
                } => {
                    let token_for_failure = playback_token.clone();
                    if let Err(message) = self
                        .load_for_engine(media_file_id, playback_token, position_seconds, filename)
                        .await
                    {
                        // A failed load must not leave its scoped URL usable.
                        let _ = self.server.cast_grants().revoke_media(media_file_id);
                        // Feed the failure back with the exact token so only
                        // the load that failed is affected.
                        self.engine.on_cast_event(CastStatusEvent {
                            kind: "load_failed".to_string(),
                            playback_token: Some(token_for_failure),
                            message: Some(message.clone()),
                            ..Default::default()
                        });
                        let _ = self.app.emit(
                            "phase5-notification",
                            serde_json::json!({ "level": "error", "message": message }),
                        );
                        self.emit_snapshot();
                    }
                }
                EngineEffect::Stop => {
                    if self.dlna.is_active() {
                        let media_before = self.dlna.snapshot().media_file_id;
                        let _ = self.dlna.control("stop", None, None, None);
                        if let Some(media_file_id) = media_before {
                            self.server.cast_grants().revoke_media(media_file_id);
                        }
                    } else {
                        let _ = self
                            .cast
                            .send_command(serde_json::json!({ "type": "control", "action": "stop" }));
                    }
                }
                EngineEffect::Notify { level, message } => {
                    let _ = self
                        .app
                        .emit("phase5-notification", serde_json::json!({ "level": level, "message": message }));
                }
            }
        }
    }

    async fn load_for_engine(
        &self,
        media_file_id: i64,
        playback_token: String,
        position_seconds: f64,
        filename: String,
    ) -> Result<(), String> {
        let status = self.server.status_snapshot().await;
        if status.state != "running" {
            let started = self.server.start().await?;
            if started.state != "running" {
                return Err(
                    "The media server could not start for Cast playback. Check the network settings and try again.".to_string(),
                );
            }
        }
        let scoped = self.server.create_cast_stream_url(media_file_id).await?;
        let probe = self.server.verify_cast_stream_url(media_file_id).await;
        let probe_ok = probe.reachable;
        let probe_message = probe.message.clone();
        self.cast.set_media_url_probe(probe);
        if !probe_ok {
            return Err(format!("The receiver would not be able to reach this video: {probe_message}"));
        }
        if scoped.av1_webm {
            let _ = self.app.emit(
                "phase5-notification",
                serde_json::json!({
                    "level": "warning",
                    "message": "This WebM uses AV1. Some receivers play it but report Idle; H.264/AAC MP4 or VP9 WebM is more reliable.",
                }),
            );
        }
        if self.dlna.is_active() {
            let device = self
                .dlna
                .selected()
                .ok_or_else(|| "Choose a DLNA renderer before playing.".to_string())?;
            let manager = self.dlna.clone();
            let dlna = tokio::task::spawn_blocking(move || {
                manager.start_playback(DlnaPlaybackRequest {
                    device,
                    url: scoped.url,
                    media_id: media_file_id,
                    filename,
                    mime_type: scoped.mime_type,
                    position_seconds,
                    playback_token,
                    cast_session_id: format!("dlna-{}", Uuid::new_v4()),
                })
            })
            .await;
            return match dlna {
                Ok(result) => result,
                Err(_) => Err("DLNA playback could not start.".to_string()),
            };
        }
        self.cast.ensure_for_playback().await?;
        self.cast.set_pending_load(media_file_id, filename.clone());
        self.cast.send_command(serde_json::json!({
            "type": "load_media",
            "url": scoped.url,
            "mime_type": scoped.mime_type,
            "title": filename,
            "position_seconds": position_seconds,
            "playback_token": playback_token,
            "request_id": Uuid::new_v4().to_string(),
        }))
    }
}
