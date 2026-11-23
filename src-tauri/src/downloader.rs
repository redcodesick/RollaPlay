//! YouTube downloader for Cast-friendly media.
//!
//! `yt-dlp.exe` ships with the app. By default the Downloader fetches a
//! progressive H.264/AAC MP4 that needs no extra tools. When the user enables
//! HD quality, a bundled WinGet script installs FFmpeg and yt-dlp merges the
//! separate 1080p AVC and AAC streams. Whole playlists are downloaded into the
//! folder chosen in the UI. The child process is launched without a shell and
//! no URL or filesystem path is ever written to diagnostics.

use serde::Serialize;
use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, State};
use url::Url;

const PROGRESS_EVENT: &str = "download-progress";
const INSTALL_EVENT: &str = "ffmpeg-install-progress";
const SD_FORMAT: &str = "best[ext=mp4][acodec!=none][vcodec!=none]/18";
const HD_FORMAT: &str = "bv*[vcodec^=avc]+ba[acodec^=mp4a]/b[ext=mp4]/b";
const POLL_INTERVAL: Duration = Duration::from_millis(120);
const INSTALL_SCRIPT: &str = include_str!("../../scripts/install-downloader-tools.ps1");

#[derive(Clone, Serialize)]
pub struct DownloaderTools {
    pub yt_dlp_found: bool,
    pub ffmpeg_found: bool,
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

fn find_executable(app: &AppHandle, name: &str) -> Option<PathBuf> {
    let executable = format!("{name}.exe");
    let mut candidates = Vec::new();
    if let Ok(data) = app.path().app_local_data_dir() {
        candidates.push(data.join("bin").join(&executable));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local).join(r"Microsoft\WinGet\Links").join(&executable));
    }
    if let Some(found) = candidates.into_iter().find(|path| path.is_file()) {
        return Some(found);
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|directory| directory.join(&executable))
            .find(|path| path.is_file())
    })
}

fn locate_tools(app: &AppHandle) -> DownloaderTools {
    let yt_dlp = find_executable(app, "yt-dlp");
    let ffmpeg = find_executable(app, "ffmpeg");
    let hint = if yt_dlp.is_none() {
        "yt-dlp will be downloaded from its official GitHub release on first use.".to_string()
    } else if ffmpeg.is_none() {
        "HD quality needs FFmpeg from its official GitHub release. Tick HD and choose Prepare tools, or use standard quality.".to_string()
    } else {
        String::new()
    };
    DownloaderTools {
        yt_dlp_found: yt_dlp.is_some(),
        ffmpeg_found: ffmpeg.is_some(),
        hd_ready: ffmpeg.is_some(),
        hint,
        yt_dlp_path: yt_dlp,
        ffmpeg_path: ffmpeg,
    }
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

fn emit(app: &AppHandle, index: usize, total: usize, phase: &'static str, detail: &str) {
    let _ = app.emit(PROGRESS_EVENT, DownloadProgress {
        index,
        total,
        phase,
        percent: match phase {
            "complete" => Some(100),
            "downloading" | "merging" => None,
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

fn build_command(tools: &DownloaderTools, directory: &Path, url: &str, hd: bool) -> Command {
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
        .arg(template)
        .arg(url);
    if hd {
        command
            .arg("--merge-output-format")
            .arg("mp4")
            .arg("--ffmpeg-location")
            .arg(tools.ffmpeg_path.clone().expect("FFmpeg checked before HD download"));
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    configure_command(&mut command);
    command
}

fn run_downloads(
    app: AppHandle,
    directory: PathBuf,
    urls: Vec<String>,
    tools: DownloaderTools,
    hd: bool,
    cancel: Arc<AtomicBool>,
    active: Arc<Mutex<Option<Arc<AtomicBool>>>>,
) {
    let _guard = DownloadGuard(active);
    let total = urls.len();

    for (index, url) in urls.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            emit(&app, index, total, "cancelled", "Download cancelled");
            return;
        }
        emit(&app, index, total, "starting", "Starting download");
        let mut child = match build_command(&tools, &directory, url, hd).spawn() {
            Ok(child) => child,
            Err(_) => {
                emit(&app, index, total, "failed", "Could not start yt-dlp");
                return;
            }
        };
        if let Some(stdout) = child.stdout.take() {
            let progress_app = app.clone();
            let progress_cancel = Arc::clone(&cancel);
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if progress_cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Some(update) = progress_line(&line, index, total) {
                        let _ = progress_app.emit(PROGRESS_EVENT, update);
                    }
                }
            });
        }
        // Drain stderr so a full pipe cannot block the download. Raw tool
        // output can contain URLs or paths and is deliberately discarded.
        if let Some(stderr) = child.stderr.take() {
            std::thread::spawn(move || {
                let _ = BufReader::new(stderr).lines().count();
            });
        }

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
            std::thread::sleep(POLL_INTERVAL);
        };
        if !status.success() {
            emit(&app, index, total, "failed", "Download failed. Check the link and update yt-dlp");
            return;
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
    locate_tools(&app)
}

#[tauri::command]
pub async fn start_download(
    app: AppHandle,
    state: State<'_, DownloadState>,
    directory: String,
    urls: Vec<String>,
    hd: bool,
) -> Result<(), String> {
    let tools = locate_tools(&app);
    if !tools.yt_dlp_found {
        return Err("Prepare downloader tools before starting.".to_string());
    }
    if hd && !tools.hd_ready {
        return Err("Prepare tools with HD enabled before starting.".to_string());
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
        run_downloads(app, directory, urls, tools, hd, cancel, active)
    });
    Ok(())
}

#[tauri::command]
pub fn cancel_download(state: State<'_, DownloadState>) -> bool {
    state.cancel()
}

/// Fetches the official binaries from their GitHub releases the first time
/// they are needed. Files live in LocalAppData, never in the install folder.
#[tauri::command]
pub async fn prepare_tools(app: AppHandle, include_ffmpeg: bool) -> Result<DownloaderTools, String> {
    let directory = tools_directory(&app)?;
    let shell = powershell().ok_or_else(|| "Windows PowerShell is unavailable".to_string())?;
    let script = directory.join("install-downloader-tools.ps1");
    std::fs::write(&script, INSTALL_SCRIPT)
        .map_err(|_| "Could not prepare the downloader setup script".to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let _ = app.emit(INSTALL_EVENT, ToolInstallProgress {
            phase: "starting",
            detail: if include_ffmpeg { "Preparing yt-dlp and FFmpeg" } else { "Preparing yt-dlp" }.to_string(),
        });
        let mut command = Command::new(shell);
        command
            .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&script)
            .arg("-TargetDirectory").arg(&directory);
        if include_ffmpeg {
            command.arg("-IncludeFFmpeg");
        }
        command.stdout(Stdio::piped()).stderr(Stdio::null());
        configure_command(&mut command);
        let mut child = command.spawn().map_err(|_| "Could not start the tool installer".to_string())?;
        if let Some(stdout) = child.stdout.take() {
            let install_app = app.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    let detail = line.trim();
                    // The script's output is generic; still drop anything that
                    // could contain a URL or a Windows path.
                    if detail.is_empty() || detail.len() > 120 || detail.contains("http") || detail.contains(":\\") {
                        continue;
                    }
                    let _ = install_app.emit(INSTALL_EVENT, ToolInstallProgress {
                        phase: "output",
                        detail: detail.to_string(),
                    });
                }
            });
        }
        let status = child.wait().map_err(|_| "The tool installer stopped unexpectedly".to_string())?;
        let tools = locate_tools(&app);
        let ready = tools.yt_dlp_found && (!include_ffmpeg || tools.hd_ready);
        if status.success() && ready {
            let _ = app.emit(INSTALL_EVENT, ToolInstallProgress {
                phase: "complete",
                detail: "Downloader tools are ready".to_string(),
            });
            Ok(tools)
        } else {
            let _ = app.emit(INSTALL_EVENT, ToolInstallProgress {
                phase: "failed",
                detail: "Tool installation failed".to_string(),
            });
            Err("Could not download the tools. Check your internet connection and try again.".to_string())
        }
    })
    .await
    .map_err(|_| "The tool installer stopped unexpectedly".to_string())?
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
    fn reads_progress_without_exposing_destination_paths() {
        let update = progress_line("[download]  45.0% of ~ 15.70MiB at 1.20MiB/s ETA 00:12", 0, 2).unwrap();
        assert_eq!(update.percent, Some(45));
        assert_eq!(update.phase, "downloading");
        assert!(progress_line("[download] Destination: F:\\Videos\\clip.mp4", 0, 2).is_none());
        let merging = progress_line("[Merger] Merging formats into \"clip.mp4\"", 0, 2).unwrap();
        assert_eq!(merging.phase, "merging");
    }
}
