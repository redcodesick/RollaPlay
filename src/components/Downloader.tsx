import { useCallback, useEffect, useRef, useState } from 'react'
import { useSWRConfig } from 'swr'
import { open } from '@tauri-apps/plugin-dialog'
import { AlertCircle, Check, Download, FolderOpen, LoaderCircle, X } from 'lucide-react'
import { getMediaDirectories, isDesktopApp, type MediaDirectory, type ScanResult } from '../media'
import { debugLog, diagnosticError, friendlyError, invokeTracked } from '../diagnostics'
import {
  cancelDownload, getDownloaderTools, installToolsWinget, onDownloadProgress, onToolInstallProgress,
  prepareTools, startDownload, type DownloadProgress, type DownloaderTools,
} from '../downloader'

export default function Downloader() {
  const { mutate: globalMutate } = useSWRConfig()
  const isDesktop = isDesktopApp()
  const [folder, setFolder] = useState('')
  const [linksText, setLinksText] = useState('')
  const [hd, setHd] = useState(true)
  const [installYtDlp, setInstallYtDlp] = useState(true)
  const [installFfmpeg, setInstallFfmpeg] = useState(true)
  const [tools, setTools] = useState<DownloaderTools | null>(null)
  const [busy, setBusy] = useState(false)
  const [preparing, setPreparing] = useState(false)
  const [installDetail, setInstallDetail] = useState('')
  const [progress, setProgress] = useState<Record<number, DownloadProgress>>({})
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const completedRef = useRef<Set<number>>(new Set())
  const indexedRef = useRef(false)

  const addToLibrary = useCallback(async () => {
    if (indexedRef.current || !folder) return
    indexedRef.current = true
    try {
      const existing = await getMediaDirectories()
      let directory: MediaDirectory | undefined = existing.find((entry) => entry.path === folder)
      if (!directory) {
        directory = await invokeTracked<MediaDirectory>('add_media_directory', { path: folder }, 'media-folders')
      }
      const result = await invokeTracked<ScanResult>('scan_media_directory', { directoryId: directory.id }, 'media-folders')
      await Promise.all([
        globalMutate('media-directories'),
        globalMutate((key: unknown) => Array.isArray(key) && key[0] === 'media-files'),
      ])
      setNotice(`${result.files_indexed} video${result.files_indexed === 1 ? '' : 's'} indexed into your library.`)
    } catch (indexError) {
      diagnosticError('downloader', 'library_index_failed', indexError)
      setError('Downloads finished, but the folder could not be indexed. Scan it from Settings → Media folders.')
    }
  }, [folder, globalMutate])

  const addToLibraryRef = useRef(addToLibrary)
  useEffect(() => { addToLibraryRef.current = addToLibrary }, [addToLibrary])

  useEffect(() => {
    if (!isDesktop) return
    let disposed = false
    const stops: Array<() => void> = []
    void getDownloaderTools()
      .then((result) => { if (!disposed) setTools(result) })
      .catch((loadError) => diagnosticError('downloader', 'tools_load_failed', loadError))

    void onDownloadProgress((update) => {
      if (disposed) return
      debugLog('downloader', 'progress_event', { index: update.index, phase: update.phase, percent: update.percent })
      setProgress((current) => ({ ...current, [update.index]: update }))
      if (update.phase === 'complete') {
        completedRef.current.add(update.index)
        if (completedRef.current.size >= update.total) {
          setBusy(false)
          setNotice('Downloads finished. Adding them to your library…')
          void addToLibraryRef.current()
        }
      } else if (update.phase === 'failed') {
        setBusy(false)
        setError(update.detail || 'A download failed. Check the link, then try again.')
      } else if (update.phase === 'cancelled') {
        setBusy(false)
        setNotice('Download cancelled. Already downloaded files were kept.')
      }
    }).then((stop) => { if (disposed) stop(); else stops.push(stop) }).catch((listenError) => diagnosticError('downloader', 'progress_listener_failed', listenError))

    void onToolInstallProgress((update) => {
      if (disposed) return
      setInstallDetail(update.detail)
      if (update.phase === 'complete') void getDownloaderTools().then(setTools).catch(() => {})
    }).then((stop) => { if (disposed) stop(); else stops.push(stop) }).catch((listenError) => diagnosticError('downloader', 'install_listener_failed', listenError))

    return () => { disposed = true; stops.forEach((stop) => stop()) }
  }, [isDesktop])

  const pickFolder = useCallback(async () => {
    setError('')
    const selection = await open({ directory: true, multiple: false, title: 'Choose where downloads are saved' })
    if (typeof selection === 'string' && selection) setFolder(selection)
  }, [])

  const prepare = useCallback(async () => {
    setPreparing(true)
    setError('')
    setInstallDetail('')
    try {
      const result = await prepareTools(hd)
      setTools(result)
      setNotice(result.hd_ready && hd ? 'yt-dlp and FFmpeg are ready for HD downloads.' : 'yt-dlp is ready for standard downloads.')
    } catch (prepareError) {
      diagnosticError('downloader', 'tools_prepare_failed', prepareError, { hd })
      setError(friendlyError(prepareError))
    } finally {
      setPreparing(false)
    }
  }, [hd])

  const installWithWinget = useCallback(async () => {
    setPreparing(true)
    setError('')
    setNotice('')
    setInstallDetail('')
    try {
      const result = await installToolsWinget(installYtDlp, installFfmpeg)
      setTools(result)
      setNotice('Selected tools are installed with WinGet.')
    } catch (installError) {
      diagnosticError('downloader', 'winget_install_failed', installError, { installYtDlp, installFfmpeg })
      setError(friendlyError(installError))
    } finally {
      setPreparing(false)
    }
  }, [installYtDlp, installFfmpeg])

  const run = useCallback(async () => {
    setError('')
    setNotice('')
    const urls = linksText.split(/[\r\n]+/).map((line) => line.trim()).filter(Boolean)
    if (!folder) { setError('Choose a download folder first.'); return }
    if (!urls.length) { setError('Paste at least one YouTube video or playlist link.'); return }
    completedRef.current = new Set()
    indexedRef.current = false
    setProgress({})
    setBusy(true)
    try {
      if (!tools?.yt_dlp_found || (hd && !tools.hd_ready)) {
        const needYtDlp = !tools?.yt_dlp_found
        const needFfmpeg = hd && !tools?.hd_ready
        let prepared: DownloaderTools
        try {
          prepared = await installToolsWinget(needYtDlp, needFfmpeg)
        } catch (wingetError) {
          diagnosticError('downloader', 'winget_auto_install_failed', wingetError, { needYtDlp, needFfmpeg })
          setInstallDetail('WinGet was unavailable; fetching official releases instead.')
          prepared = await prepareTools(hd)
        }
        setTools(prepared)
      }
      await startDownload(folder, urls, hd)
      setNotice('Downloading… your library will update when the videos are ready.')
    } catch (downloadError) {
      diagnosticError('downloader', 'download_start_failed', downloadError, { hd, linkCount: urls.length })
      setBusy(false)
      setError(friendlyError(downloadError))
    }
  }, [folder, hd, linksText, tools])

  const cancel = useCallback(async () => {
    try {
      const stopped = await cancelDownload()
      setNotice(stopped ? 'Cancelling downloads…' : 'The download had already finished.')
    } catch (cancelError) {
      diagnosticError('downloader', 'download_cancel_failed', cancelError)
    }
  }, [])

  if (!isDesktop) {
    return <section className="section-page downloader-page"><h1>YouTube downloader</h1><p>This feature works in the installed Windows app.</p></section>
  }

  const needsTools = !tools?.yt_dlp_found || (hd && !tools.hd_ready)
  const orderedProgress = Object.values(progress).sort((a, b) => a.index - b.index)

  return (
    <section className="section-page downloader-page">
      <div className="section-eyebrow">YOUR COLLECTION</div>
      <h1>YouTube downloader</h1>
      <p>Paste a video or playlist link. RollaPlay saves Cast-friendly MP4 files in the folder you choose, then adds that folder to your library automatically.</p>

      <div className="downloader-grid">
        <div className="downloader-card">
          <div className="download-card-head"><span className="download-card-icon"><FolderOpen size={18} /></span><div><h2>Destination folder</h2><small>Downloads stay on this PC.</small></div></div>
          <div className="download-folder-row">
            <span title={folder}>{folder || 'No folder selected'}</span>
            <button className="folder-action" onClick={() => void pickFolder()} disabled={busy}>Choose folder</button>
          </div>

          <label className="download-label" htmlFor="download-links">Video or playlist links — one per line</label>
          <textarea
            id="download-links"
            className="download-links"
            rows={5}
            value={linksText}
            onChange={(event) => setLinksText(event.target.value)}
            placeholder={'https://www.youtube.com/watch?v=VIDEO_ID\nhttps://www.youtube.com/playlist?list=PLAYLIST_ID'}
            disabled={busy}
          />

          <label className="download-check">
            <input
              type="checkbox"
              checked={hd}
              onChange={(event) => {
                setHd(event.target.checked)
                if (event.target.checked) setInstallFfmpeg(true)
              }}
              disabled={busy}
            />
            <span><strong>HD quality · H.264/AAC MP4</strong><small>Uses FFmpeg to combine 720p/1080p AVC with AAC. Install it below or fetch it from GitHub.</small></span>
          </label>

          <div className="download-winget">
            <div className="download-label">Install tools with WinGet</div>
            <label className="download-check compact">
              <input type="checkbox" checked={installYtDlp} onChange={(event) => setInstallYtDlp(event.target.checked)} disabled={busy || preparing} />
              <span><strong>yt-dlp</strong><small>YouTube downloader · updated to latest</small></span>
            </label>
            <label className="download-check compact">
              <input type="checkbox" checked={installFfmpeg} onChange={(event) => setInstallFfmpeg(event.target.checked)} disabled={busy || preparing} />
              <span><strong>FFmpeg</strong><small>HD video/audio merge</small></span>
            </label>
            <button
              className="folder-action"
              onClick={() => void installWithWinget()}
              disabled={busy || preparing || (!installYtDlp && !installFfmpeg)}
            >
              {preparing ? 'Installing…' : 'Install selected with WinGet'}
            </button>
          </div>

          <div className="download-actions">
            <button className="button-primary" onClick={() => void run()} disabled={busy || preparing}>
              {busy ? <LoaderCircle size={16} className="spin-icon" /> : <Download size={16} />}
              {busy ? 'Downloading…' : 'Download'}
            </button>
            {busy && <button className="folder-cancel" onClick={() => void cancel()}><X size={15} /> Cancel</button>}
            {needsTools && !busy && <button className="folder-action" onClick={() => void prepare()} disabled={preparing}>{preparing ? 'Preparing…' : 'Prepare from GitHub'}</button>}
          </div>
          <p className="download-hint">Only public videos and playlists can be downloaded. Private or sign-in-only links are not supported.</p>
          {tools?.hint && !hd && <p className="download-hint">{tools.hint}</p>}
        </div>

        <div className="downloader-card">
          <div className="download-card-head"><span className="download-card-icon"><Download size={18} /></span><div><h2>Downloader status</h2><small>Offline tools, no account required.</small></div></div>
          <div className="download-tool-status">
            <span className={`status-pill ${tools?.yt_dlp_found ? 'ok' : ''}`}><span className="status-dot" /> yt-dlp {tools?.yt_dlp_found ? 'ready' : 'not installed'}</span>
            <span className={`status-pill ${tools?.ffmpeg_found ? 'ok' : ''}`}><span className="status-dot" /> FFmpeg {tools?.ffmpeg_found ? 'ready' : 'not installed'}</span>
          </div>
          {installDetail && <div className="download-install-detail">{installDetail}</div>}
          {orderedProgress.length > 0 && <div className="download-progress-list">
            {orderedProgress.map((item) => (
              <div key={item.index} className="download-progress-row">
                <span className={`download-phase phase-${item.phase}`}>{item.phase === 'complete' ? <Check size={14} /> : item.phase === 'failed' ? <AlertCircle size={14} /> : <LoaderCircle size={14} className="spin-icon" />}</span>
                <span className="download-progress-copy">
                  <strong>{item.total > 1 ? `Link ${item.index + 1} of ${item.total}` : 'Download'}</strong>
                  <small>{item.detail}</small>
                  {item.percent !== null && item.phase !== 'failed' && item.phase !== 'cancelled' && (
                    <span className="download-progress-track" aria-hidden="true">
                      <span style={{ width: `${Math.max(0, Math.min(100, item.percent))}%` }} />
                    </span>
                  )}
                </span>
              </div>
            ))}
          </div>}
          {!orderedProgress.length && !installDetail && <p className="download-hint">Nothing downloading yet. Files are saved as MP4 and indexed automatically.</p>}
        </div>
      </div>

      {error && <div className="library-message error" role="alert"><AlertCircle size={16} /> {error}</div>}
      {notice && <div className="library-message success" role="status"><Check size={16} /> {notice}</div>}
    </section>
  )
}
