//! YouTube downloader for Cast-friendly media.
//!
//! The Downloader fetches a progressive H.264/AAC MP4 by default. HD quality
//! merges separate AVC/AAC streams with FFmpeg. The opt-in TV-compatible mode
//! checks the real streams and re-encodes only unsupported files to H.264/AAC
//! MP4, replacing the original after the result is verified. Tools are never
//! bundled: they
//! are installed into app-local data from their official GitHub releases or,
//! when ticked in the UI, with WinGet. Discovery always prefers that app-local
//! copy so a stale WinGet folder cannot shadow a known-good tool. Whole
//! playlists are downloaded into the folder chosen in the UI. The child process
//! is launched without a shell and no URL or filesystem path is ever written
//! to diagnostics.

use serde::Serialize;
use std::{
    ffi::OsString,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager, State};
use url::Url;

const PROGRESS_EVENT: &str = "download-progress";
const INSTALL_EVENT: &str = "ffmpeg-install-progress";
const SD_FORMAT: &str = "best[ext=mp4][acodec!=none][vcodec!=none]/18";
const HD_FORMAT: &str = "bv*[vcodec^=avc]+ba[acodec^=mp4a]/b[ext=mp4]/b";
const POLL_INTERVAL: Duration = Duration::from_millis(120);
const STALL_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const CONVERSION_STALL_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const TOOL_PROBE_TIMEOUT: Duration = Duration::from_secs(8);
const INSTALL_SCRIPT: &str = include_str!("../../scripts/install-downloader-tools.ps1");
const WINGET_SCRIPT: &str = include_str!("../../scripts/install-downloader-tools-winget.ps1");

#[derive(Clone, Serialize)]
pub struct DownloaderTools {
    pub yt_dlp_found: bool,
    /// True only when the executable exists and answers `--version`.
    pub yt_dlp_ready: bool,
    pub ffmpeg_found: bool,
    /// True only when the executable exists and answers `-version`.
    pub ffmpeg_ready: bool,
    pub hd_ready: bool,
    pub hint: String,
    /// Internal absolute paths; skipped so they never reach the frontend or diagnostics.
    #[serde(skip)]
    pub yt_dlp_path: Option<PathBuf>,
    #[serde(skip)]
    pub ffmpeg_path: Option<PathBuf>,
}

#[derive(Clone, Serialize)]
pub struct DownloadProgress {
    pub index: usize,
    pub total: usize,
    pub phase: &'static str,
    pub percent: Option<u8>,
    pub detail: String,
}

#[derive(Clone, Serialize)]
pub struct ToolInstallProgress {
    pub phase: &'static str,
    pub detail: String,
}

#[derive(Clone, Copy)]
enum FfmpegFailure {
    Cancelled,
    Stalled,
    Message(&'static str),
}

#[derive(Clone, Copy, Default)]
struct StreamProbe {
    video_h264: bool,
    audio_aac: bool,
    has_audio: bool,
}

impl StreamProbe {
    fn tv_ready(&self) -> bool {
        self.video_h264 && (!self.has_audio || self.audio_aac)
    }
}

#[derive(Default)]
pub struct DownloadState(Arc<Mutex<Option<Arc<AtomicBool>>>>);

impl DownloadState {
    fn try_start(&self) -> Result<Arc<AtomicBool>, String> {
        let mut active = self.0.lock().map_err(|_| "Download state is unavailable".to_string())?;
        if active.is_some() {
            return Err("A download is already running.".to_string());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *active = Some(Arc::clone(&cancel));
        Ok(cancel)
    }

    pub(crate) fn cancel(&self) -> bool {
        self.0
            .lock()
            .ok()
            .and_then(|active| active.as_ref().map(|cancel| {
                cancel.store(true, Ordering::Relaxed);
                true
            }))
            .unwrap_or(false)
    }
}

struct DownloadGuard(Arc<Mutex<Option<Arc<AtomicBool>>>>);

impl Drop for DownloadGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.0.lock() {
            *active = None;
        }
    }
}

fn tools_directory(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app.path().app_local_data_dir().map_err(|error| error.to_string())?.join("bin");
    std::fs::create_dir_all(&directory).map_err(|_| "Could not prepare the downloader tools".to_string())?;
    Ok(directory)
}

fn search_directory(directory: &Path, executable: &str, depth: usize) -> Option<PathBuf> {
    if depth > 4 {
        return None;
    }
    let mut subdirectories = Vec::new();
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    let mut consider = |path: PathBuf| {
        let modified = std::fs::metadata(&path)
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .unwrap_or(UNIX_EPOCH);
        if newest.as_ref().map_or(true, |(current, _)| modified > *current) {
            newest = Some((modified, path));
        }
    };
    for entry in std::fs::read_dir(directory).ok()?.flatten() {
        let path = entry.path();
        if path.file_name().and_then(|name| name.to_str()) == Some(executable) && path.is_file() {
            consider(path);
        } else if path.is_dir() {
            subdirectories.push(path);
        }
    }
    for subdirectory in subdirectories {
        if let Some(found) = search_directory(&subdirectory, executable, depth + 1) {
            consider(found);
        }
    }
    newest.map(|(_, path)| path)
}

/// Candidate order is fixed: app-local `bin` first, then WinGet Links, then
/// WinGet's versioned packages and finally PATH. A WinGet package can no longer
/// silently shadow a tool RollaPlay installed itself.
fn candidate_paths(
    app_local: Option<PathBuf>,
    local_app_data: Option<OsString>,
    path_env: Option<OsString>,
    name: &str,
) -> Vec<PathBuf> {
    let executable = format!("{name}.exe");
    let mut candidates = Vec::new();
    if let Some(directory) = app_local {
        candidates.push(directory.join("bin").join(&executable));
    }
    if let Some(local) = local_app_data {
        let local_root = PathBuf::from(local);
        candidates.push(local_root.join(r"Microsoft\WinGet\Links").join(&executable));
        // WinGet installs portable tools in versioned folders that are not on
        // PATH until the shell restarts. Scan them as a lower-priority source
        // so a fresh install still works without restarting RollaPlay.
        let packages = local_root.join(r"Microsoft\WinGet\Packages");
        if let Some(found) = search_directory(&packages, &executable, 0) {
            candidates.push(found);
        }
    }
    if let Some(paths) = path_env {
        candidates.extend(std::env::split_paths(&paths).map(|directory| directory.join(&executable)));
    }
    candidates
}

fn executable_candidates(app: &AppHandle, name: &str) -> Vec<PathBuf> {
    candidate_paths(
        app.path().app_local_data_dir().ok(),
        std::env::var_os("LOCALAPPDATA"),
        std::env::var_os("PATH"),
        name,
    )
}

fn find_executable(app: &AppHandle, name: &str) -> Option<PathBuf> {
    executable_candidates(app, name).into_iter().find(|path| path.is_file())
}

/// Probe candidates in priority order and keep the first executable that
/// actually answers. A broken app-local shim no longer blocks a working WinGet
/// or PATH copy, while `found` still distinguishes "not responding" from
/// "not installed".
fn probe_tool_candidates(app: &AppHandle, name: &str, version_arg: &str) -> (bool, Option<PathBuf>) {
    let mut found = None;
    for path in executable_candidates(app, name) {
        if !path.is_file() {
            continue;
        }
        if found.is_none() {
            found = Some(path.clone());
        }
        if probe_tool(&path, version_arg) {
            return (true, Some(path));
        }
    }
    (false, found)
}

fn locate_tools(app: &AppHandle) -> DownloaderTools {
    let yt_dlp = find_executable(app, "yt-dlp");
    let ffmpeg = find_executable(app, "ffmpeg");
    DownloaderTools {
        yt_dlp_found: yt_dlp.is_some(),
        yt_dlp_ready: false,
        ffmpeg_found: ffmpeg.is_some(),
        ffmpeg_ready: false,
        hd_ready: false,
        hint: String::new(),
        yt_dlp_path: yt_dlp,
        ffmpeg_path: ffmpeg,
    }
}

/// A file on disk is not enough: WinGet can leave an old or broken shim that
/// never answers. Run the tool's version command so "ready" means usable.
fn probe_tool(executable: &Path, version_arg: &str) -> bool {
    let mut command = Command::new(executable);
    command
        .arg(version_arg)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null());
    configure_command(&mut command);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => return false,
    };
    let reader = child.stdout.take().map(|stdout| {
        std::thread::spawn(move || {
            let mut output = String::new();
            let _ = BufReader::new(stdout).read_to_string(&mut output);
            output
        })
    });
    let deadline = Instant::now() + TOOL_PROBE_TIMEOUT;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            let output = reader.and_then(|handle| handle.join().ok()).unwrap_or_default();
            return status.success() && !output.trim().is_empty();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            if let Some(handle) = reader {
                let _ = handle.join();
            }
            return false;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn tool_hint(tools: &DownloaderTools) -> String {
    if !tools.yt_dlp_ready {
        if tools.yt_dlp_found {
            "yt-dlp was found but did not respond to a version check. Reinstall it with WinGet or refresh it with Prepare from GitHub.".to_string()
        } else {
            "Install yt-dlp with WinGet or fetch it from its official GitHub release.".to_string()
        }
    } else if !tools.ffmpeg_ready {
        if tools.ffmpeg_found {
            "FFmpeg was found but did not respond to a version check. Reinstall it with WinGet, refresh it from GitHub, or use standard quality.".to_string()
        } else {
            "HD quality needs a working FFmpeg. Tick HD and install it with WinGet, or use standard quality.".to_string()
        }
    } else {
        String::new()
    }
}

fn checked_tools(app: &AppHandle) -> DownloaderTools {
    let (yt_dlp_ready, yt_dlp_path) = probe_tool_candidates(app, "yt-dlp", "--version");
    let (ffmpeg_ready, ffmpeg_path) = probe_tool_candidates(app, "ffmpeg", "-version");
    let mut tools = DownloaderTools {
        yt_dlp_found: yt_dlp_path.is_some(),
        yt_dlp_ready,
        ffmpeg_found: ffmpeg_path.is_some(),
        ffmpeg_ready,
        hd_ready: ffmpeg_ready,
        hint: String::new(),
        yt_dlp_path,
        ffmpeg_path,
    };
    tools.hint = tool_hint(&tools);
    tools
}

async fn current_tools(app: &AppHandle) -> DownloaderTools {
    let probe_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || checked_tools(&probe_app))
        .await
        .unwrap_or_else(|_| locate_tools(app))
}

/// Only real YouTube links are accepted. Rejecting other schemes and hosts
/// keeps the generated command free of unexpected flags or local files.
fn validate_youtube_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let parsed = Url::parse(trimmed).map_err(|_| "Only YouTube video or playlist links are supported.".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Only YouTube video or playlist links are supported.".to_string());
    }
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    if host != "youtube.com" && host != "www.youtube.com" && host != "m.youtube.com" && host != "youtu.be" {
        return Err("Only YouTube video or playlist links are supported.".to_string());
    }
    Ok(trimmed.to_string())
}

fn percent_in(line: &str) -> Option<u8> {
    let bytes = line.as_bytes();
    for position in 1..bytes.len() {
        if bytes[position] != b'%' {
            continue;
        }
        let mut start = position - 1;
        while start > 0 && (bytes[start - 1].is_ascii_digit() || bytes[start - 1] == b'.') {
            start -= 1;
        }
        let text = &line[start..position];
        if text.is_empty() || text.len() > 6 || text.starts_with('.') || text.ends_with('.') {
            continue;
        }
        if let Ok(value) = text.parse::<f64>() {
            if value <= 100.0 {
                return Some(value.round() as u8);
            }
        }
    }
    None
}

fn progress_line(line: &str, index: usize, total: usize) -> Option<DownloadProgress> {
    if line.contains("[Merger]") || line.contains("Merging formats into") {
        return Some(DownloadProgress {
            index,
            total,
            phase: "merging",
            percent: Some(100),
            detail: "Combining HD video and audio into MP4".to_string(),
        });
    }
    // yt-dlp prints the absolute output path in Destination lines; never emit it.
    if !line.contains("[download]") || line.contains("Destination:") {
        return None;
    }
    let percent = percent_in(line)?;
    let tail = line.splitn(2, '%').nth(1).unwrap_or("").trim();
    let detail = if tail.is_empty() {
        format!("Downloading · {percent}%")
    } else {
        format!("{percent}% · {}", tail.chars().take(72).collect::<String>())
    };
    Some(DownloadProgress { index, total, phase: "downloading", percent: Some(percent), detail })
}

/// Maps raw tool output to a fixed, safe message. Nothing from the tool is
/// echoed back, so URLs, paths and tokens can never reach diagnostics.
fn failure_hint(line: &str) -> Option<&'static str> {
    let lower = line.to_ascii_lowercase();
    if lower.contains("private") {
        Some("This link is private or sign-in-only. Use a public video or playlist; private links need a YouTube sign-in.")
    } else if lower.contains("ffmpeg") && (lower.contains("error") || lower.contains("failed") || lower.contains("dll") || lower.contains("not found")) {
        Some("FFmpeg could not merge the HD formats. Reinstall it with WinGet, refresh it from GitHub, or use standard quality.")
    } else if lower.contains("video unavailable") || lower.contains("has been removed") || lower.contains("is not available") || lower.contains("not found") || lower.contains("does not exist") {
        Some("This video is unavailable or has been removed.")
    } else if lower.contains("sign in") || lower.contains("captcha") || lower.contains("bot") || lower.contains("human") || lower.contains("age-restricted") || lower.contains("members") {
        Some("YouTube asked for a sign-in or membership. Use a public link or try again later.")
    } else if lower.contains("unsupported url") || lower.contains("not a valid url") || lower.contains("no video formats") {
        Some("This link is not a supported YouTube video or playlist.")
    } else if lower.contains("please report this issue") || lower.contains("extractorerror") || lower.contains("unable to extract")
        || (lower.contains("yt-dlp") && (lower.contains("update") || lower.contains("outdated") || lower.contains("newer"))) {
        Some("yt-dlp could not read this page. Refresh it with Prepare from GitHub or WinGet, then try again.")
    } else if lower.contains("rate limit") || lower.contains("too many") {
        Some("YouTube is rate-limiting this PC. Wait a few minutes and try again.")
    } else if lower.contains("unable to download") || lower.contains("connection") || lower.contains("timed out") || lower.contains("timeout")
        || lower.contains("ssl") || lower.contains("tls") || lower.contains("network") || lower.contains("dns") {
        Some("A network error stopped the download. Check your connection and try again.")
    } else if lower.contains("http error 403") || lower.contains("forbidden") {
        Some("YouTube refused the download. Try again later or refresh yt-dlp with Prepare from GitHub.")
    } else if lower.contains("requested format") || lower.contains("format is not available") {
        Some("No Cast-friendly format was offered for this video. Try standard quality or another public link.")
    } else if lower.contains("http error") || lower.contains("http 5") {
        Some("YouTube returned an HTTP error. Try again later.")
    } else {
        None
    }
}

fn sanitize_error_line(line: &str) -> String {
    let cleaned = line
        .split_whitespace()
        .filter_map(|token| {
            let trimmed = token.trim_matches(|c: char| matches!(c, '"' | '\'' | ',' | ';' | ':' | ')' | '(' | '[' | ']'));
            if trimmed.is_empty() {
                return None;
            }
            let lower = trimmed.to_ascii_lowercase();
            if lower.starts_with("http") || lower.contains("://") {
                return Some("[url]".to_string());
            }
            if lower.contains("token=") || lower.contains("bearer") || lower.starts_with("authorization") {
                return Some("[secret]".to_string());
            }
            if lower.contains(":\\") || lower.contains("\\\\") || lower.starts_with('/') {
                return Some("[path]".to_string());
            }
            if trimmed.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') && trimmed.len() >= 10 {
                return Some("[id]".to_string());
            }
            if trimmed.len() > 28 {
                return Some("[text]".to_string());
            }
            Some(trimmed.to_string())
        })
        .take(24)
        .collect::<Vec<_>>()
        .join(" ");
    let mut result = cleaned;
    if result.len() > 180 {
        result.truncate(180);
        result.push_str("…");
    }
    result
}

fn log_unmapped_error(app: &AppHandle, line: &str) {
    let _ = crate::wizard_diag_log(
        app.clone(),
        "downloader".to_string(),
        "unmapped_tool_error".to_string(),
        Some(format!("bytes={};text={}", line.len(), sanitize_error_line(line))),
    );
}

/// Reads only the stream descriptions FFmpeg prints for an input file. No
/// filename, metadata value or raw FFmpeg line is retained.
fn parse_stream_probe(stderr: &str) -> StreamProbe {
    let lower = stderr.to_ascii_lowercase();
    StreamProbe {
        video_h264: lower.contains("video: h264") || lower.contains("video: avc1"),
        audio_aac: lower.contains("audio: aac") || lower.contains("audio: mp4a"),
        has_audio: lower.contains("audio:"),
    }
}

fn join_ffmpeg_threads(
    stdout: &mut Option<std::thread::JoinHandle<()>>,
    stderr: &mut Option<std::thread::JoinHandle<String>>,
) -> String {
    if let Some(thread) = stdout.take() {
        let _ = thread.join();
    }
    stderr
        .take()
        .and_then(|thread| thread.join().ok())
        .unwrap_or_default()
}

fn ffmpeg_command(tools: &DownloaderTools, args: &[OsString]) -> Result<Command, &'static str> {
    let executable = tools
        .ffmpeg_path
        .clone()
        .ok_or("FFmpeg is missing or not working. Install it with WinGet and try again.")?;
    let mut command = Command::new(executable);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    configure_command(&mut command);
    Ok(command)
}

/// Runs FFmpeg for probing or conversion. Progress lines are consumed only to
/// keep the watchdog alive; stderr is returned for fixed-code classification
/// and is never forwarded verbatim.
fn run_ffmpeg(
    tools: &DownloaderTools,
    args: Vec<OsString>,
    cancel: &AtomicBool,
    allowed_codes: &[i32],
) -> Result<String, FfmpegFailure> {
    let mut command = ffmpeg_command(tools, &args)
        .map_err(FfmpegFailure::Message)?;
    let mut child = command
        .spawn()
        .map_err(|_| FfmpegFailure::Message("Could not start FFmpeg. Reinstall it and try again."))?;
    let last_activity = Arc::new(AtomicU64::new(now_millis()));
    let mut stdout_thread = child.stdout.take().map(|stdout| {
        let activity = Arc::clone(&last_activity);
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                activity.store(now_millis(), Ordering::Relaxed);
                let _ = line;
            }
        })
    });
    let mut stderr_thread = child.stderr.take().map(|stderr| {
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = BufReader::new(stderr).read_to_string(&mut text);
            text
        })
    });
    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = join_ffmpeg_threads(&mut stdout_thread, &mut stderr_thread);
            return Err(FfmpegFailure::Cancelled);
        }
        if let Some(status) = child.try_wait().ok().flatten() {
            break status;
        }
        let last = last_activity.load(Ordering::Relaxed);
        if now_millis().saturating_sub(last) > CONVERSION_STALL_TIMEOUT.as_millis() as u64 {
            let _ = child.kill();
            let _ = child.wait();
            let _ = join_ffmpeg_threads(&mut stdout_thread, &mut stderr_thread);
            return Err(FfmpegFailure::Stalled);
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    let stderr = join_ffmpeg_threads(&mut stdout_thread, &mut stderr_thread);
    if !allowed_codes.contains(&status.code().unwrap_or(-1)) {
        return Err(FfmpegFailure::Message(
            "FFmpeg could not finish the TV-compatible conversion. The original file was kept.",
        ));
    }
    Ok(stderr)
}

fn probe_tv_streams(
    tools: &DownloaderTools,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<StreamProbe, FfmpegFailure> {
    let args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-i"),
        path.as_os_str().to_os_string(),
    ];
    // `ffmpeg -i` exits with 1 after printing stream details; that is expected.
    let stderr = run_ffmpeg(tools, args, cancel, &[0, 1])?;
    Ok(parse_stream_probe(&stderr))
}

fn tv_conversion_args(input: &Path, output: &Path) -> Vec<OsString> {
    vec![
        OsString::from("-y"),
        OsString::from("-hide_banner"),
        OsString::from("-nostdin"),
        OsString::from("-i"),
        input.as_os_str().to_os_string(),
        OsString::from("-map"),
        OsString::from("0:v:0"),
        OsString::from("-map"),
        OsString::from("0:a:0?"),
        OsString::from("-sn"),
        OsString::from("-dn"),
        OsString::from("-c:v"),
        OsString::from("libx264"),
        OsString::from("-preset"),
        OsString::from("veryfast"),
        OsString::from("-crf"),
        OsString::from("20"),
        OsString::from("-pix_fmt"),
        OsString::from("yuv420p"),
        OsString::from("-c:a"),
        OsString::from("aac"),
        OsString::from("-b:a"),
        OsString::from("128k"),
        OsString::from("-movflags"),
        OsString::from("+faststart"),
        OsString::from("-progress"),
        OsString::from("pipe:1"),
        OsString::from("-nostats"),
        output.as_os_str().to_os_string(),
    ]
}

fn conversion_paths(path: &Path, stamp: u64) -> (PathBuf, PathBuf) {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("video");
    (
        directory.join(format!("{stem}.rollaplay-tv-{stamp}.mp4")),
        directory.join(format!("{stem}.rollaplay-orig-{stamp}")),
    )
}

fn log_conversion_failure(app: &AppHandle, stderr_len: usize) {
    // Only the byte count is kept. Conversion stderr can contain local file
    // names, so no part of it is copied into diagnostics.
    let _ = crate::wizard_diag_log(
        app.clone(),
        "downloader".to_string(),
        "tv_conversion_failed".to_string(),
        Some(format!("bytes={stderr_len}")),
    );
}

/// Converts one downloaded file to H.264/AAC MP4 only when its real streams
/// or container are not already TV-friendly. The original is renamed aside
/// first and removed only after the verified replacement is in place.
fn convert_to_tv_format(
    app: &AppHandle,
    tools: &DownloaderTools,
    path: &Path,
    index: usize,
    total: usize,
    cancel: &AtomicBool,
) -> bool {
    emit(app, index, total, "converting", "Checking video format");
    let probe = match probe_tv_streams(tools, path, cancel) {
        Ok(probe) => probe,
        Err(FfmpegFailure::Cancelled) => {
            emit(app, index, total, "cancelled", "Download cancelled");
            return false;
        }
        Err(FfmpegFailure::Stalled) => {
            emit(app, index, total, "failed", "FFmpeg stopped responding during conversion. Reinstall it and try again.");
            return false;
        }
        Err(FfmpegFailure::Message(detail)) => {
            emit(app, index, total, "failed", detail);
            return false;
        }
    };
    let already_mp4 = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("mp4"))
        .unwrap_or(false);
    if probe.tv_ready() && already_mp4 {
        emit(app, index, total, "converting", "Already TV-compatible; no conversion needed");
        return true;
    }

    emit(app, index, total, "converting", "Converting to TV-friendly H.264/AAC MP4");
    let stamp = now_millis();
    let (temporary, backup) = conversion_paths(path, stamp);
    let stderr = match run_ffmpeg(tools, tv_conversion_args(path, &temporary), cancel, &[0]) {
        Ok(stderr) => stderr,
        Err(FfmpegFailure::Cancelled) => {
            let _ = std::fs::remove_file(&temporary);
            emit(app, index, total, "cancelled", "Download cancelled");
            return false;
        }
        Err(FfmpegFailure::Stalled) => {
            let _ = std::fs::remove_file(&temporary);
            emit(app, index, total, "failed", "Conversion stalled with no output for ten minutes. Try a shorter video or a faster computer.");
            return false;
        }
        Err(FfmpegFailure::Message(detail)) => {
            let _ = std::fs::remove_file(&temporary);
            log_conversion_failure(app, detail.len());
            emit(app, index, total, "failed", detail);
            return false;
        }
    };
    let usable = temporary
        .is_file()
        && std::fs::metadata(&temporary)
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false);
    if !usable {
        let _ = std::fs::remove_file(&temporary);
        log_conversion_failure(app, stderr.len());
        emit(app, index, total, "failed", "FFmpeg did not produce a playable MP4. The original file was kept.");
        return false;
    }
    let verified = match probe_tv_streams(tools, &temporary, cancel) {
        Ok(verified) => verified,
        Err(FfmpegFailure::Cancelled) => {
            let _ = std::fs::remove_file(&temporary);
            emit(app, index, total, "cancelled", "Download cancelled");
            return false;
        }
        Err(_) => {
            let _ = std::fs::remove_file(&temporary);
            log_conversion_failure(app, stderr.len());
            emit(app, index, total, "failed", "The converted file could not be verified. The original file was kept.");
            return false;
        }
    };
    if !verified.tv_ready() {
        let _ = std::fs::remove_file(&temporary);
        log_conversion_failure(app, stderr.len());
        emit(app, index, total, "failed", "FFmpeg produced a file this receiver may not play. The original file was kept.");
        return false;
    }
    if std::fs::rename(path, &backup).is_err() {
        let _ = std::fs::remove_file(&temporary);
        emit(app, index, total, "failed", "The converted MP4 could not replace the download. The original file was kept.");
        return false;
    }
    if std::fs::rename(&temporary, path).is_err() {
        let _ = std::fs::rename(&backup, path);
        let _ = std::fs::remove_file(&temporary);
        emit(app, index, total, "failed", "The converted MP4 could not replace the download. The original file was kept.");
        return false;
    }
    let _ = std::fs::remove_file(&backup);
    emit(app, index, total, "converting", "TV-friendly MP4 ready");
    true
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

fn emit(app: &AppHandle, index: usize, total: usize, phase: &'static str, detail: &str) {
    let _ = app.emit(PROGRESS_EVENT, DownloadProgress {
        index,
        total,
        phase,
        percent: match phase {
            "complete" => Some(100),
            "downloading" | "merging" | "converting" => None,
            _ => Some(0),
        },
        detail: detail.to_string(),
    });
}

fn configure_command(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

fn build_command(
    tools: &DownloaderTools,
    directory: &Path,
    url: &str,
    hd: bool,
    tv_compatible: bool,
) -> Command {
    let template = directory
        .join("%(title).150B [%(id)s].%(ext)s")
        .to_string_lossy()
        .replace('\\', "/");
    let mut command = Command::new(tools.yt_dlp_path.clone().expect("yt-dlp checked before download"));
    command
        .arg("--no-warnings")
        .arg("--newline")
        .arg("--yes-playlist")
        .arg("--format")
        .arg(if hd { HD_FORMAT } else { SD_FORMAT })
        .arg("--output")
        .arg(template);
    if tv_compatible {
        // Captured in Rust only. These absolute paths are never emitted; they
        // let the converter verify the exact files yt-dlp finished.
        command.arg("--print").arg("after_move:filepath");
    }
    command.arg(url);
    if hd {
        command
            .arg("--merge-output-format")
            .arg("mp4")
            .arg("--ffmpeg-location")
            .arg(tools.ffmpeg_path.clone().expect("FFmpeg checked before HD download"));
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
    configure_command(&mut command);
    command
}

fn run_downloads(
    app: AppHandle,
    directory: PathBuf,
    urls: Vec<String>,
    tools: DownloaderTools,
    hd: bool,
    tv_compatible: bool,
    cancel: Arc<AtomicBool>,
    active: Arc<Mutex<Option<Arc<AtomicBool>>>>,
) {
    let _guard = DownloadGuard(active);
    let total = urls.len();
    let failure: Arc<Mutex<Option<&'static str>>> = Arc::new(Mutex::new(None));

    for (index, url) in urls.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            emit(&app, index, total, "cancelled", "Download cancelled");
            return;
        }
        emit(&app, index, total, "starting", "Starting download");
        let started_millis = now_millis();
        let last_activity = Arc::new(AtomicU64::new(0));
        let output_paths: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
        let mut child = match build_command(&tools, &directory, url, hd, tv_compatible).spawn() {
            Ok(child) => child,
            Err(_) => {
                emit(&app, index, total, "failed", "Could not start yt-dlp");
                return;
            }
        };
        // yt-dlp reports some failures on stdout and others on stderr, so both
        // streams are classified. Raw output can contain URLs or paths; only
        // fixed hints are forwarded and unmapped lines are redacted in diagnostics.
        let stdout_thread = child.stdout.take().map(|stdout| {
            let progress_app = app.clone();
            let progress_cancel = Arc::clone(&cancel);
            let activity = Arc::clone(&last_activity);
            let failure_hint_ref = Arc::clone(&failure);
            let log_app = app.clone();
            let captured_paths = Arc::clone(&output_paths);
            std::thread::spawn(move || {
                let mut logged_unmapped = false;
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    activity.store(now_millis(), Ordering::Relaxed);
                    if progress_cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    if tv_compatible {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() && Path::new(trimmed).is_file() {
                            if let Ok(mut paths) = captured_paths.lock() {
                                paths.push(PathBuf::from(trimmed));
                            }
                        }
                    }
                    if let Some(hint) = failure_hint(&line) {
                        if let Ok(mut slot) = failure_hint_ref.lock() {
                            if slot.is_none() {
                                *slot = Some(hint);
                            }
                        }
                    } else if !logged_unmapped && line.to_ascii_lowercase().contains("error") {
                        logged_unmapped = true;
                        log_unmapped_error(&log_app, &line);
                    }
                    if let Some(update) = progress_line(&line, index, total) {
                        let _ = progress_app.emit(PROGRESS_EVENT, update);
                    }
                }
            })
        });
        let stderr_thread = child.stderr.take().map(|stderr| {
            let failure_hint_ref = Arc::clone(&failure);
            let log_app = app.clone();
            let activity = Arc::clone(&last_activity);
            std::thread::spawn(move || {
                let mut logged_unmapped = false;
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    activity.store(now_millis(), Ordering::Relaxed);
                    if let Some(hint) = failure_hint(&line) {
                        if let Ok(mut slot) = failure_hint_ref.lock() {
                            if slot.is_none() {
                                *slot = Some(hint);
                            }
                        }
                    } else if !logged_unmapped && line.to_ascii_lowercase().contains("error") {
                        logged_unmapped = true;
                        log_unmapped_error(&log_app, &line);
                    }
                }
            })
        });

        let status = loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                emit(&app, index, total, "cancelled", "Download cancelled");
                return;
            }
            if let Some(status) = child.try_wait().ok().flatten() {
                break status;
            }
            let last = last_activity.load(Ordering::Relaxed);
            let reference = if last == 0 { started_millis } else { last };
            if now_millis().saturating_sub(reference) > STALL_TIMEOUT.as_millis() as u64 {
                let _ = child.kill();
                let _ = child.wait();
                emit(&app, index, total, "failed", "Download stalled with no output for five minutes. Try a public link or standard quality.");
                return;
            }
            std::thread::sleep(POLL_INTERVAL);
        };
        // Join both readers before reading the hint slot: the final error line
        // can arrive just as the process exits.
        if let Some(thread) = stdout_thread {
            let _ = thread.join();
        }
        if let Some(thread) = stderr_thread {
            let _ = thread.join();
        }
        if !status.success() {
            let exit_code = status.code();
            let detail = failure
                .lock()
                .ok()
                .and_then(|slot| *slot)
                .map(str::to_string)
                .unwrap_or_else(|| format!(
                    "yt-dlp exited with code {}. Refresh it with Prepare from GitHub or WinGet, then retry.",
                    exit_code.unwrap_or(-1)
                ));
            emit(&app, index, total, "failed", &detail);
            return;
        }
        if tv_compatible {
            let downloaded = output_paths
                .lock()
                .map(|paths| paths.clone())
                .unwrap_or_default();
            if downloaded.is_empty() {
                emit(&app, index, total, "failed", "The download finished but its file could not be checked for TV compatibility. Try again.");
                return;
            }
            for path in downloaded {
                if !convert_to_tv_format(&app, &tools, &path, index, total, &cancel) {
                    return;
                }
            }
        }
        emit(&app, index, total, "complete", "Download finished");
    }
}

fn powershell() -> Option<PathBuf> {
    std::env::var_os("WINDIR").map(|root| {
        PathBuf::from(root).join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe")
    }).filter(|path| path.is_file())
}

#[tauri::command]
pub async fn downloader_tools(app: AppHandle) -> DownloaderTools {
    current_tools(&app).await
}

#[tauri::command]
pub async fn start_download(
    app: AppHandle,
    state: State<'_, DownloadState>,
    directory: String,
    urls: Vec<String>,
    hd: bool,
    tv_compatible: bool,
) -> Result<(), String> {
    let tools = current_tools(&app).await;
    if !tools.yt_dlp_ready {
        return Err("yt-dlp is missing or not working. Install it with WinGet and try again.".to_string());
    }
    if (hd || tv_compatible) && !tools.ffmpeg_ready {
        return Err("FFmpeg is missing or not working. Install it with WinGet and try again.".to_string());
    }
    let directory = crate::canonicalize_media_path(Path::new(&directory))
        .map_err(|_| "Choose a download folder that exists on this device".to_string())?;
    if !directory.is_dir() {
        return Err("The selected download path is not a folder".to_string());
    }
    let urls = urls
        .into_iter()
        .map(|url| validate_youtube_url(&url))
        .collect::<Result<Vec<_>, _>>()?;
    if urls.is_empty() {
        return Err("Paste at least one YouTube link".to_string());
    }
    let cancel = state.try_start()?;
    let active = Arc::clone(&state.0);
    tauri::async_runtime::spawn_blocking(move || {
        run_downloads(app, directory, urls, tools, hd, tv_compatible, cancel, active)
    });
    Ok(())
}

#[tauri::command]
pub fn cancel_download(state: State<'_, DownloadState>) -> bool {
    state.cancel()
}

async fn run_tool_installer(
    app: &AppHandle,
    script_name: &str,
    script: &str,
    args: Vec<OsString>,
    starting_detail: &str,
) -> Result<(), String> {
    let directory = tools_directory(app)?;
    let script_path = directory.join(script_name);
    std::fs::write(&script_path, script)
        .map_err(|_| "Could not prepare the downloader setup script".to_string())?;
    let shell = powershell().ok_or_else(|| "Windows PowerShell is unavailable".to_string())?;
    let installer_app = app.clone();
    let starting = starting_detail.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let _ = installer_app.emit(INSTALL_EVENT, ToolInstallProgress {
            phase: "starting",
            detail: starting,
        });
        let mut command = Command::new(shell);
        command
            .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&script_path);
        for arg in args {
            command.arg(arg);
        }
        command.stdout(Stdio::piped()).stderr(Stdio::null()).stdin(Stdio::null());
        configure_command(&mut command);
        let mut child = command.spawn().map_err(|_| "Could not start the tool installer".to_string())?;
        let stdout_thread = child.stdout.take().map(|stdout| {
            let emit_app = installer_app.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    let detail = line.trim();
                    // The scripts print generic lines; still drop anything
                    // that could contain a URL, a path or a credential.
                    if detail.is_empty() || detail.len() > 120 || detail.contains("http") || detail.contains(":\\") {
                        continue;
                    }
                    let _ = emit_app.emit(INSTALL_EVENT, ToolInstallProgress {
                        phase: "output",
                        detail: detail.to_string(),
                    });
                }
            })
        });
        let status = child.wait().map_err(|_| "The tool installer stopped unexpectedly".to_string())?;
        if let Some(thread) = stdout_thread {
            let _ = thread.join();
        }
        if status.success() {
            let _ = installer_app.emit(INSTALL_EVENT, ToolInstallProgress {
                phase: "complete",
                detail: "Downloader tools are ready".to_string(),
            });
            Ok(())
        } else {
            let _ = installer_app.emit(INSTALL_EVENT, ToolInstallProgress {
                phase: "failed",
                detail: "Tool installation failed".to_string(),
            });
            Err("Tool installation failed. Check your internet connection and try again.".to_string())
        }
    })
    .await
    .map_err(|_| "The tool installer stopped unexpectedly".to_string())?
}

/// Fetches the official binaries from their GitHub releases the first time
/// they are needed. Files live in LocalAppData, never in the install folder.
#[tauri::command]
pub async fn prepare_tools(app: AppHandle, include_ffmpeg: bool) -> Result<DownloaderTools, String> {
    let directory = tools_directory(&app)?;
    let mut args = vec![OsString::from("-TargetDirectory"), OsString::from(directory)];
    if include_ffmpeg {
        args.push(OsString::from("-IncludeFFmpeg"));
    }
    let starting = if include_ffmpeg { "Preparing yt-dlp and FFmpeg" } else { "Preparing yt-dlp" };
    run_tool_installer(&app, "install-downloader-tools.ps1", INSTALL_SCRIPT, args, starting).await?;
    let tools = current_tools(&app).await;
    if !tools.yt_dlp_ready || (include_ffmpeg && !tools.ffmpeg_ready) {
        return Err("The downloaded tools did not respond to a version check. Try WinGet instead.".to_string());
    }
    Ok(tools)
}

/// Installs exactly the tools the user ticked via WinGet's official packages.
#[tauri::command]
pub async fn install_tools_winget(
    app: AppHandle,
    install_yt_dlp: bool,
    install_ffmpeg: bool,
) -> Result<DownloaderTools, String> {
    let mut args = Vec::new();
    if install_yt_dlp {
        args.push(OsString::from("-InstallYtDlp"));
    }
    if install_ffmpeg {
        args.push(OsString::from("-InstallFfmpeg"));
    }
    if args.is_empty() {
        return Err("Select at least one tool to install.".to_string());
    }
    run_tool_installer(
        &app,
        "install-downloader-tools-winget.ps1",
        WINGET_SCRIPT,
        args,
        "Installing selected tools with WinGet",
    )
    .await?;
    let tools = current_tools(&app).await;
    if (install_yt_dlp && !tools.yt_dlp_ready) || (install_ffmpeg && !tools.ffmpeg_ready) {
        return Err("WinGet finished, but a selected tool still does not respond. Try Prepare from GitHub.".to_string());
    }
    Ok(tools)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_youtube_links_and_rejects_everything_else() {
        assert!(validate_youtube_url("https://www.youtube.com/watch?v=qORYO0atB6g").is_ok());
        assert!(validate_youtube_url("https://www.youtube.com/playlist?list=PL12345").is_ok());
        assert!(validate_youtube_url("https://youtu.be/qORYO0atB6g").is_ok());
        assert!(validate_youtube_url("file:///C:/secret.txt").is_err());
        assert!(validate_youtube_url("https://example.com/video").is_err());
        assert!(validate_youtube_url("not a link").is_err());
    }

    #[test]
    fn explains_private_and_unavailable_links_without_echoing_them() {
        let hint = failure_hint("ERROR: [youtube] VIDEO_ID: Private video. Sign in if you've been granted access").unwrap();
        assert!(hint.contains("private"));
        assert!(failure_hint("ERROR: [youtube] VIDEO_ID: Video unavailable").is_some());
        assert!(failure_hint("[download] Destination: F:\\secret\\clip.mp4").is_none());
    }

    #[test]
    fn reads_progress_without_exposing_destination_paths() {
        let update = progress_line("[download]  45.0% of ~ 15.70MiB at 1.20MiB/s ETA 00:12", 0, 2).unwrap();
        assert_eq!(update.percent, Some(45));
        assert_eq!(update.phase, "downloading");
        assert!(progress_line("[download] Destination: F:\\Videos\\clip.mp4", 0, 2).is_none());
        let merging = progress_line("[Merger] Merging formats into \"clip.mp4\"", 0, 2).unwrap();
        assert_eq!(merging.phase, "merging");
    }

    #[test]
    fn stream_probe_accepts_only_h264_with_aac_or_no_audio() {
        let ready = parse_stream_probe(
            "Stream #0:0: Video: h264 (High), yuv420p\nStream #0:1: Audio: aac (LC)",
        );
        assert!(ready.tv_ready());
        let silent = parse_stream_probe("Stream #0:0: Video: h264 (High), yuv420p");
        assert!(silent.tv_ready());
        let av1 = parse_stream_probe(
            "Stream #0:0: Video: av01 (Main)\nStream #0:1: Audio: opus",
        );
        assert!(!av1.tv_ready());
        let vp9 = parse_stream_probe(
            "Stream #0:0: Video: vp9\nStream #0:1: Audio: aac (LC)",
        );
        assert!(!vp9.tv_ready());
    }

    #[test]
    fn tv_conversion_forces_h264_aac_and_faststart() {
        let args = tv_conversion_args(Path::new("clip.webm"), Path::new("clip.mp4"))
            .into_iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(args.contains("-c:v libx264"));
        assert!(args.contains("-c:a aac"));
        assert!(args.contains("-pix_fmt yuv420p"));
        assert!(args.contains("-movflags +faststart"));
        assert!(args.contains("-progress pipe:1"));
    }

    #[test]
    fn tv_mode_captures_finished_paths_but_progress_never_emits_them() {
        let mut tools = fake_tools(true, true);
        tools.yt_dlp_path = Some(PathBuf::from("yt-dlp.exe"));
        tools.ffmpeg_path = Some(PathBuf::from("ffmpeg.exe"));
        let command = build_command(
            &tools,
            Path::new("F:\\Videos"),
            "https://youtu.be/video-id",
            false,
            true,
        );
        let args = command
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--print" && pair[1] == "after_move:filepath"));
        assert!(progress_line("[download] Destination: F:\\Videos\\clip.mp4", 0, 1).is_none());
    }

    fn fake_tools(found: bool, ready: bool) -> DownloaderTools {
        DownloaderTools {
            yt_dlp_found: found,
            yt_dlp_ready: ready,
            ffmpeg_found: found,
            ffmpeg_ready: ready,
            hd_ready: ready,
            hint: String::new(),
            yt_dlp_path: None,
            ffmpeg_path: None,
        }
    }

    #[test]
    fn tool_hint_separates_missing_from_installed_but_broken() {
        let missing = fake_tools(false, false);
        assert!(tool_hint(&missing).contains("Install yt-dlp"));

        let broken = fake_tools(true, false);
        assert!(tool_hint(&broken).contains("did not respond"));

        let ready = fake_tools(true, true);
        assert!(tool_hint(&ready).is_empty());
    }

    #[test]
    fn candidate_order_prefers_app_local_over_winget_and_path() {
        let unique = std::time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("rollaplay-candidates-{unique}"));
        let app_local = root.join("local");
        let winget = root.join("winget");
        let path_directory = root.join("path");
        let package = winget.join(r"Microsoft\WinGet\Packages\yt-dlp\current");
        for directory in [app_local.join("bin"), package.clone(), path_directory.clone()] {
            std::fs::create_dir_all(directory).unwrap();
        }
        let app_exe = app_local.join("bin").join("yt-dlp.exe");
        let winget_exe = package.join("yt-dlp.exe");
        let path_exe = path_directory.join("yt-dlp.exe");
        for executable in [&app_exe, &winget_exe, &path_exe] {
            std::fs::write(executable, b"stub").unwrap();
        }

        let candidates = candidate_paths(
            Some(app_local),
            Some(winget.into_os_string()),
            Some(path_directory.into_os_string()),
            "yt-dlp",
        );
        let existing = candidates.into_iter().filter(|path| path.is_file()).collect::<Vec<_>>();
        assert_eq!(existing, vec![app_exe, winget_exe, path_exe]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn classifies_network_ffmpeg_and_stale_tool_failures_without_echoing_output() {
        let network = failure_hint("ERROR: Unable to download webpage: <urlopen error [SSL: CERTIFICATE_VERIFY_FAILED]>").unwrap();
        assert!(network.contains("network error"));
        let ffmpeg = failure_hint("ERROR: [ffmpeg] ffmpeg.exe: dll load failed F:\\secret").unwrap();
        assert!(ffmpeg.contains("FFmpeg"));
        let stale = failure_hint("ERROR: Please report this issue on https://github.com/yt-dlp/yt-dlp").unwrap();
        assert!(stale.contains("Refresh it"));
        for hint in [network, ffmpeg, stale] {
            assert!(!hint.contains("http"));
            assert!(!hint.contains("F:"));
        }
        assert!(failure_hint("[download] Destination: F:\\secret\\clip.mp4").is_none());
    }

    #[test]
    fn sanitized_errors_never_keep_urls_paths_tokens_or_ids() {
        let raw = "ERROR: [youtube] dQw4w9WgXcQ: Sign in token=abc123 https://example.com/watch?v=x F:\\secret\\clip.mp4";
        let safe = sanitize_error_line(raw);
        assert!(!safe.contains("http"));
        assert!(!safe.contains("F:"));
        assert!(!safe.contains("dQw4w9WgXcQ"));
        assert!(!safe.contains("abc123"));
        assert!(!safe.contains("clip.mp4"));
        assert!(safe.contains("[url]"));
        assert!(safe.contains("[path]"));
        assert!(safe.contains("[id]"));
        assert!(safe.contains("[secret]"));
    }
}
