import { useCallback, useEffect, useRef, useState } from 'react'
import { useSWRConfig } from 'swr'
import { open } from '@tauri-apps/plugin-dialog'
import { AlertCircle, Check, Download, FolderOpen, LoaderCircle, RotateCcw, SkipForward, X } from 'lucide-react'
import { getMediaDirectories, isDesktopApp, type MediaDirectory, type ScanResult } from '../media'
import { debugLog, diagnosticError, friendlyError, invokeTracked } from '../diagnostics'
import {
  cancelDownload, getDownloaderTools, installToolsWinget, onDownloadProgress, onToolInstallProgress,
  prepareTools, startDownload, type DownloadProgress, type DownloaderTools, type DownloadQuality,
} from '../downloader'
import {
  clampedPercent, downloadsReadyToIndex, phaseLabel, progressCounter, progressIdentity,
  resumeDownloadIndices,
} from '../downloader-progress'

type DownloadBatch = {
  folder: string
  urls: string[]
  quality: DownloadQuality
  tvCompatible: boolean
  total: number
}

// The backend job outlives this view. Keep UI state in memory so switching
// pages does not make an active download look like it vanished.
const storedUi = {
  folder: '',
  linksText: '',
  quality: '1080p' as DownloadQuality,
  tvCompatible: false,
  installYtDlp: true,
  installFfmpeg: true,
  tools: null as DownloaderTools | null,
  busy: false,
  preparing: false,
  installDetail: '',
  installStep: 0,
  installTotal: 1,
  progress: {} as Record<number, DownloadProgress>,
  batch: null as DownloadBatch | null,
  error: '',
  notice: '',
  indexed: false,
}

export default function Downloader() {
  const { mutate: globalMutate } = useSWRConfig()
  const isDesktop = isDesktopApp()
  const [folder, setFolder] = useState(storedUi.folder)
  const [linksText, setLinksText] = useState(storedUi.linksText)
  const [quality, setQuality] = useState<DownloadQuality>(storedUi.quality)
  const [tvCompatible, setTvCompatible] = useState(storedUi.tvCompatible)
  const [installYtDlp, setInstallYtDlp] = useState(storedUi.installYtDlp)
  const [installFfmpeg, setInstallFfmpeg] = useState(storedUi.installFfmpeg)
  const [tools, setTools] = useState<DownloaderTools | null>(storedUi.tools)
  const [busy, setBusy] = useState(storedUi.busy)
  const [preparing, setPreparing] = useState(storedUi.preparing)
  const [installDetail, setInstallDetail] = useState(storedUi.installDetail)
  const [installStep, setInstallStep] = useState(storedUi.installStep)
  const [installTotal, setInstallTotal] = useState(storedUi.installTotal)
  const [progress, setProgress] = useState<Record<number, DownloadProgress>>(storedUi.progress)
  const [batch, setBatch] = useState<DownloadBatch | null>(storedUi.batch)
  const [error, setError] = useState(storedUi.error)
  const [notice, setNotice] = useState(storedUi.notice)
  const progressRef = useRef(storedUi.progress)
  const batchRef = useRef(storedUi.batch)
  const launchingRef = useRef(false)
  const indexedRef = useRef(storedUi.indexed)

  useEffect(() => {
    storedUi.folder = folder
    storedUi.linksText = linksText
    storedUi.quality = quality
    storedUi.tvCompatible = tvCompatible
    storedUi.installYtDlp = installYtDlp
    storedUi.installFfmpeg = installFfmpeg
    storedUi.tools = tools
    storedUi.busy = busy
    storedUi.preparing = preparing
    storedUi.installDetail = installDetail
    storedUi.installStep = installStep
    storedUi.installTotal = installTotal
    storedUi.progress = progress
    storedUi.batch = batch
    storedUi.error = error
    storedUi.notice = notice
    storedUi.indexed = indexedRef.current
    progressRef.current = progress
    batchRef.current = batch
  }, [folder, linksText, quality, tvCompatible, installYtDlp, installFfmpeg, tools, busy, preparing, installDetail, installStep, installTotal, progress, batch, error, notice])

  const addToLibrary = useCallback(async (directoryPath: string) => {
    if (indexedRef.current || !directoryPath) return
    indexedRef.current = true
    try {
      const existing = await getMediaDirectories()
      let directory: MediaDirectory | undefined = existing.find((entry) => entry.path === directoryPath)
      if (!directory) {
        directory = await invokeTracked<MediaDirectory>('add_media_directory', { path: directoryPath }, 'media-folders')
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
  }, [globalMutate])

  const addToLibraryRef = useRef(addToLibrary)
  useEffect(() => { addToLibraryRef.current = addToLibrary }, [addToLibrary])

  useEffect(() => {
    if (!isDesktop) return
    let disposed = false
    const stops: Array<() => void> = []
    // Probing never installs. Tools are only fetched when the user presses a
    // button below, so opening the Downloader is always a read-only check.
    void getDownloaderTools()
      .then((result) => { if (!disposed) setTools(result) })
      .catch((loadError) => diagnosticError('downloader', 'tools_load_failed', loadError))

    void onDownloadProgress((update) => {
      if (disposed) return
      debugLog('downloader', 'progress_event', { index: update.index, phase: update.phase, percent: update.percent })
      const previous = progressRef.current[update.index]
      const merged: DownloadProgress = { ...update, title: update.title || previous?.title || '' }
      const next = { ...progressRef.current, [update.index]: merged }
      progressRef.current = next
      setProgress(next)

      if (update.phase === 'complete') {
        if (downloadsReadyToIndex(progressRef.current, update.total)) {
          setBusy(false)
          setNotice('Downloads finished. Adding them to your library…')
          void addToLibraryRef.current(batchRef.current?.folder ?? '')
        }
      } else if (update.phase === 'failed') {
        setBusy(false)
        setError(update.detail || 'A download failed. Check the link, then try again.')
        // A tool can be marked ready by a shallow version check and still fail
        // on a real link. Refresh the pills so they reflect a fresh probe.
        void getDownloaderTools().then((result) => { if (!disposed) setTools(result) }).catch(() => {})
      } else if (update.phase === 'cancelled') {
        setBusy(false)
        setNotice('Download cancelled. Already downloaded files were kept.')
      }
    }).then((stop) => { if (disposed) stop(); else stops.push(stop) }).catch((listenError) => diagnosticError('downloader', 'progress_listener_failed', listenError))

    void onToolInstallProgress((update) => {
      if (disposed) return
      const total = Math.max(1, update.total || 1)
      setInstallDetail(update.detail)
      setInstallTotal(total)
      setInstallStep(Math.max(0, Math.min(total, update.step || 0)))
      if (update.phase === 'complete') {
        void getDownloaderTools().then((result) => { if (!disposed) setTools(result) }).catch(() => {})
      }
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
    setNotice('')
    setInstallDetail('')
    setInstallStep(0)
    setInstallTotal(1)
    const wantFfmpeg = quality !== 'standard' || tvCompatible
    try {
      const result = await prepareTools(wantFfmpeg)
      setTools(result)
      setNotice(result.yt_dlp_ready && (!wantFfmpeg || result.ffmpeg_ready) ? 'yt-dlp and FFmpeg responded to their version checks.' : 'yt-dlp is ready for standard downloads.')
    } catch (prepareError) {
      diagnosticError('downloader', 'tools_prepare_failed', prepareError, { quality, tvCompatible })
      setError(friendlyError(prepareError))
    } finally {
      setPreparing(false)
    }
  }, [quality, tvCompatible])

  const installWithWinget = useCallback(async () => {
    setPreparing(true)
    setError('')
    setNotice('')
    setInstallDetail('')
    setInstallStep(0)
    setInstallTotal(1)
    try {
      const result = await installToolsWinget(installYtDlp, installFfmpeg)
      setTools(result)
      setNotice('Selected tools are installed and responding.')
    } catch (installError) {
      diagnosticError('downloader', 'winget_install_failed', installError, { installYtDlp, installFfmpeg })
      setError(friendlyError(installError))
    } finally {
      setPreparing(false)
    }
  }, [installYtDlp, installFfmpeg])

  const launchBatch = useCallback(async (currentBatch: DownloadBatch, indices: number[]) => {
    if (launchingRef.current || !indices.length) return
    launchingRef.current = true
    setError('')
    setNotice('')
    try {
      // Re-probe, but never install: a missing tool must be installed by the
      // user with the buttons in this view before a download can start.
      const current = await getDownloaderTools()
      setTools(current)
      const wantFfmpeg = currentBatch.quality !== 'standard' || currentBatch.tvCompatible
      const missing: string[] = []
      if (!current.yt_dlp_ready) missing.push('yt-dlp')
      if (wantFfmpeg && !current.ffmpeg_ready) missing.push('FFmpeg')
      if (missing.length) {
        throw new Error(`Install ${missing.join(' and ')} below first. RollaPlay only downloads tools when you press an install button.`)
      }
      setBusy(true)
      setNotice(currentBatch.tvCompatible
        ? 'Downloading… TV-compatible files will replace the originals when verified.'
        : 'Downloading… your library will update when the videos are ready.')
      await startDownload({
        directory: currentBatch.folder,
        urls: indices.map((index) => currentBatch.urls[index]),
        quality: currentBatch.quality,
        tvCompatible: currentBatch.tvCompatible,
        indices,
        total: currentBatch.total,
      })
    } finally {
      launchingRef.current = false
    }
  }, [])

  const run = useCallback(async () => {
    const urls = linksText.split(/[\r\n]+/).map((line) => line.trim()).filter(Boolean)
    if (!folder) { setError('Choose a download folder first.'); return }
    if (!urls.length) { setError('Paste at least one YouTube video or playlist link.'); return }
    const nextBatch: DownloadBatch = { folder, urls, quality, tvCompatible, total: urls.length }
    setBatch(nextBatch)
    batchRef.current = nextBatch
    indexedRef.current = false
    const empty: Record<number, DownloadProgress> = {}
    progressRef.current = empty
    setProgress(empty)
    try {
      await launchBatch(nextBatch, urls.map((_, index) => index))
    } catch (downloadError) {
      diagnosticError('downloader', 'download_start_failed', downloadError, { quality, tvCompatible, linkCount: urls.length })
      setBusy(false)
      setNotice('')
      setError(friendlyError(downloadError))
    }
  }, [folder, quality, tvCompatible, linksText, launchBatch])

  const retryAt = useCallback(async (index: number) => {
    const currentBatch = batchRef.current
    if (!currentBatch || busy || launchingRef.current) return
    const indices = resumeDownloadIndices(progressRef.current, index, currentBatch.total, true)
    if (!indices.length) return
    debugLog('downloader', 'download_retry', { index, linkCount: indices.length })
    const previous = progressRef.current[index]
    const retrying: DownloadProgress = previous
      ? { ...previous, phase: 'starting', detail: 'Retrying download', percent: null }
      : { index, total: currentBatch.total, phase: 'starting', detail: 'Retrying download', percent: null, title: '' }
    const marked = { ...progressRef.current, [index]: retrying }
    progressRef.current = marked
    setProgress(marked)
    // A skipped row may have been indexed already; retrying it must rescan.
    indexedRef.current = false
    try {
      await launchBatch(currentBatch, indices)
    } catch (retryError) {
      if (previous) {
        const rollback = { ...progressRef.current, [index]: previous }
        progressRef.current = rollback
        setProgress(rollback)
      }
      diagnosticError('downloader', 'download_retry_failed', retryError, { index, linkCount: indices.length })
      setBusy(false)
      setNotice('')
      setError(friendlyError(retryError))
    }
  }, [busy, launchBatch])

  const skipAt = useCallback(async (index: number) => {
    const currentBatch = batchRef.current
    if (!currentBatch || busy || launchingRef.current) return
    const indices = resumeDownloadIndices(progressRef.current, index, currentBatch.total, false)
    debugLog('downloader', 'download_skip', { index, linkCount: indices.length })
    const previous = progressRef.current[index]
    const skipped: DownloadProgress = previous
      ? { ...previous, phase: 'skipped', detail: 'Skipped — continuing with remaining links', percent: null }
      : { index, total: currentBatch.total, phase: 'skipped', detail: 'Skipped — continuing with remaining links', percent: null, title: '' }
    const marked = { ...progressRef.current, [index]: skipped }
    progressRef.current = marked
    setProgress(marked)
    if (!indices.length) {
      setBusy(false)
      if (downloadsReadyToIndex(marked, currentBatch.total)) {
        setNotice('Downloads finished. Adding them to your library…')
        void addToLibraryRef.current(currentBatch.folder)
      } else {
        setNotice('Download skipped.')
      }
      return
    }
    try {
      await launchBatch(currentBatch, indices)
    } catch (skipError) {
      diagnosticError('downloader', 'download_resume_failed', skipError, { index, linkCount: indices.length })
      setBusy(false)
      setNotice('')
      setError(friendlyError(skipError))
    }
  }, [busy, launchBatch])

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

  const needsTools = !tools?.yt_dlp_ready || ((quality !== 'standard' || tvCompatible) && !tools?.ffmpeg_ready)
  const toolLabel = (found?: boolean, ready?: boolean) => ready ? 'ready' : found ? 'not responding' : 'not installed'
  const toolClass = (ready?: boolean, found?: boolean) => ready ? 'ok' : found ? 'warn' : ''
  const orderedProgress = Object.values(progress).sort((a, b) => a.index - b.index)
  const installPercent = Math.max(0, Math.min(100, Math.round((installStep / Math.max(1, installTotal)) * 100)))
  const completedCount = orderedProgress.filter((item) => item.phase === 'complete').length
  const failedCount = orderedProgress.filter((item) => item.phase === 'failed').length
  const cancelledCount = orderedProgress.filter((item) => item.phase === 'cancelled').length
  const skippedCount = orderedProgress.filter((item) => item.phase === 'skipped').length

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

          <div className="download-quality-picker">
            <label className="download-label" htmlFor="download-quality">Maximum quality</label>
            <select
              id="download-quality"
              className="download-quality-select"
              value={quality}
              onChange={(event) => {
                setQuality(event.target.value as DownloadQuality)
                if (event.target.value !== 'standard') setInstallFfmpeg(true)
              }}
              disabled={busy}
            >
              <option value="standard">Standard — single-file MP4 · no FFmpeg</option>
              <option value="720p">720p max — merged video + audio · needs FFmpeg</option>
              <option value="1080p">1080p max — merged video + audio · needs FFmpeg</option>
              <option value="4k">4K max — merged video + audio · needs FFmpeg</option>
            </select>
            <small>Quality is a maximum: RollaPlay picks the best real format at or below it. 4K sources often use AV1/VP9; tick TV-friendly for older receivers.</small>
          </div>

          <div className="download-quality-note">
            <div><strong>Standard</strong><span>Single-file MP4 · fastest, widest receiver support</span></div>
            <div><strong>720p / 1080p</strong><span>H.264/AAC when YouTube offers it · much smaller than 4K</span></div>
            <div><strong>4K max</strong><span>Best format up to 2160p · TV-friendly converts AV1/VP9 when needed</span></div>
            <div><strong>TV-friendly</strong><span>Verifies real codecs and converts unsupported files to H.264/AAC MP4</span></div>
          </div>

          <label className="download-check">
            <input
              type="checkbox"
              checked={tvCompatible}
              onChange={(event) => {
                setTvCompatible(event.target.checked)
                if (event.target.checked) setInstallFfmpeg(true)
              }}
              disabled={busy}
            />
            <span><strong>Convert to TV-friendly MP4</strong><small>For Cast and DLNA receivers. Checks the actual video and audio streams; re-encodes with FFmpeg only when needed. The original is replaced after the converted file is verified. Needs FFmpeg.</small></span>
          </label>

          <div className="download-winget">
            <div className="download-label">Install tools with WinGet</div>
            <label className="download-check compact">
              <input type="checkbox" checked={installYtDlp} onChange={(event) => setInstallYtDlp(event.target.checked)} disabled={busy || preparing} />
              <span><strong>yt-dlp</strong><small>YouTube downloader · updated to latest</small></span>
            </label>
            <label className="download-check compact">
              <input type="checkbox" checked={installFfmpeg} onChange={(event) => setInstallFfmpeg(event.target.checked)} disabled={busy || preparing} />
              <span><strong>FFmpeg</strong><small>HD merge and TV-friendly conversion</small></span>
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
            {!busy && <button className="folder-action" onClick={() => void prepare()} disabled={preparing}>{preparing ? 'Preparing…' : needsTools ? 'Prepare from GitHub' : 'Refresh tools from GitHub'}</button>}
          </div>
          <p className="download-hint">Only public videos and playlists can be downloaded. Private or sign-in-only links are not supported. If a link fails, use Retry or Skip on its status row.</p>
          {tools?.hint && <p className="download-hint">{tools.hint}</p>}
        </div>

        <div className="downloader-card">
          <div className="download-card-head"><span className="download-card-icon"><Download size={18} /></span><div><h2>Downloader status</h2><small>Offline tools, no account required.</small></div></div>
          <div className="download-tool-status">
            <span className={`status-pill ${toolClass(tools?.yt_dlp_ready, tools?.yt_dlp_found)}`}><span className="status-dot" /> yt-dlp {toolLabel(tools?.yt_dlp_found, tools?.yt_dlp_ready)}</span>
            <span className={`status-pill ${toolClass(tools?.ffmpeg_ready, tools?.ffmpeg_found)}`}><span className="status-dot" /> FFmpeg {toolLabel(tools?.ffmpeg_found, tools?.ffmpeg_ready)}</span>
          </div>
          <p className="download-hint">RollaPlay checks both tools but never installs them by itself. Use the buttons to install or refresh; each step shows its progress here.</p>
          {installDetail && <div className="download-install-detail">
            <span>{installDetail}</span>
            <span className="download-progress-track"><span style={{ width: `${installPercent}%` }} /></span>
            <span className="download-progress-percent">{installTotal > 1 ? `Step ${installStep} of ${installTotal}` : 'Installing'}</span>
          </div>}
          {orderedProgress.length > 0 && <div className="download-progress-list">
            {orderedProgress.map((item) => {
              const percent = clampedPercent(item.percent)
              const counter = progressCounter(item)
              return (
                <div key={item.index} className={`download-progress-row phase-${item.phase}`}>
                  <span className={`download-phase phase-${item.phase}`}>
                    {item.phase === 'complete' ? <Check size={14} />
                      : item.phase === 'failed' ? <AlertCircle size={14} />
                        : item.phase === 'cancelled' ? <X size={14} />
                          : item.phase === 'skipped' ? <SkipForward size={14} />
                            : <LoaderCircle size={14} className="spin-icon" />}
                  </span>
                  <span className="download-progress-copy">
                    <strong>{progressIdentity(item)}</strong>
                    <small>
                      {counter && <span className="download-progress-counter">{counter} · </span>}
                      <span className="download-progress-state">{phaseLabel(item.phase)}</span>
                      {item.detail && ` · ${item.detail}`}
                    </small>
                    {percent !== null && item.phase !== 'failed' && item.phase !== 'cancelled' && item.phase !== 'skipped' && <>
                      <span className="download-progress-track">
                        <span style={{ width: `${percent}%` }} />
                      </span>
                      <span className="download-progress-percent">{percent}%</span>
                    </>}
                  </span>
                  {(item.phase === 'failed' || item.phase === 'skipped') && !busy && <span className="download-progress-actions">
                    <button className="download-row-action" onClick={() => void retryAt(item.index)} aria-label={`Retry link ${item.index + 1}`}>
                      <RotateCcw size={13} /> Retry
                    </button>
                    {item.phase === 'failed' && <button className="download-row-action subtle" onClick={() => void skipAt(item.index)} aria-label={`Skip link ${item.index + 1}`}>
                      <SkipForward size={13} /> Skip
                    </button>}
                  </span>}
                </div>
              )
            })}
          </div>}
          {orderedProgress.length > 0 && <ul className="download-summary">
            <li>{completedCount} completed</li>
            {failedCount > 0 && <li className="is-failed">{failedCount} failed</li>}
            {skippedCount > 0 && <li>{skippedCount} skipped</li>}
            {cancelledCount > 0 && <li>{cancelledCount} cancelled</li>}
            <li>{indexedRef.current ? 'Indexed into library' : 'Indexing when finished'}</li>
          </ul>}
          {!orderedProgress.length && !installDetail && <p className="download-hint">Nothing downloading yet. Files are saved as MP4 and indexed automatically.</p>}
        </div>
      </div>

      {error && <div className="library-message error" role="alert"><AlertCircle size={16} /> {error}</div>}
      {notice && <div className="library-message success" role="status"><Check size={16} /> {notice}</div>}
    </section>
  )
}
