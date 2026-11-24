import { useCallback, useEffect, useRef, useState } from 'react'
import { check, type Update } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import { openUrl } from '@tauri-apps/plugin-opener'
import { AlertCircle, Check, Download, RefreshCw, RotateCcw } from 'lucide-react'
import { formatBytes, isDesktopApp } from '../media'
import {
  applyDownloadEvent, beginCheck, canCheckForUpdates, checkAvailable, checkUpToDate,
  compareVersions, downloadFinished, downloadStarted, formatCheckedAt, formatReleaseDate,
  initialUpdateState, installFinished, RELEASES_URL, restartFailed, retryUpdate,
  updaterErrorKind, updateFailed, type NoteInline, type ParsedReleaseNotes, type UpdateViewState,
} from '../updater'

const statusLabels: Record<UpdateViewState['phase'], string> = {
  idle: 'Not checked yet',
  checking: 'Checking',
  upToDate: 'Up to date',
  available: 'Update available',
  downloading: 'Downloading update',
  installing: 'Installing',
  restartReady: 'Ready to restart',
  error: 'Update failed',
}

function openExternal(url: string): void {
  void (async () => {
    try {
      if (isDesktopApp()) await openUrl(url)
      else window.open(url, '_blank', 'noopener,noreferrer')
    } catch {
      /* Opening release notes is optional and must never block the updater. */
    }
  })()
}

function ReleaseInlines({ inlines }: { inlines: NoteInline[] }) {
  return <>{inlines.map((inline, index) => {
    if (inline.kind === 'strong') return <strong key={index}>{inline.text}</strong>
    if (inline.kind === 'code') return <code key={index}>{inline.text}</code>
    if (inline.kind === 'link') {
      return (
        <button key={index} className="release-link" onClick={() => openExternal(inline.href)}>
          {inline.text}
        </button>
      )
    }
    return <span key={index}>{inline.text}</span>
  })}</>
}

function ReleaseNotes({ notes }: { notes?: ParsedReleaseNotes }) {
  if (!notes || notes.blocks.length === 0) {
    return <p className="release-empty">No release notes were published for this version.</p>
  }
  return (
    <div className="release-notes">
      {notes.blocks.map((block, index) => {
        if (block.kind === 'heading') {
          const Heading = block.level === 1 ? 'h4' : block.level === 2 ? 'h5' : 'h6'
          return <Heading key={index}><ReleaseInlines inlines={block.inlines} /></Heading>
        }
        if (block.kind === 'bullet') {
          return <li key={index}><ReleaseInlines inlines={block.inlines} /></li>
        }
        if (block.kind === 'code') return <pre key={index}><code>{block.text}</code></pre>
        return <p key={index}><ReleaseInlines inlines={block.inlines} /></p>
      })}
      {notes.truncated && <p className="release-empty">Release notes were shortened. View all releases for the full text.</p>}
    </div>
  )
}

export default function SoftwareUpdates({ installedVersion }: { installedVersion: string }) {
  const [state, setState] = useState<UpdateViewState>(() => initialUpdateState(installedVersion))
  const activeUpdate = useRef<Update | null>(null)
  const mounted = useRef(true)
  const busy = useRef(false)

  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
      const pending = activeUpdate.current
      activeUpdate.current = null
      if (pending) void pending.close().catch(() => {})
    }
  }, [])

  const closeActiveUpdate = useCallback(async () => {
    const pending = activeUpdate.current
    activeUpdate.current = null
    if (pending) await pending.close().catch(() => {})
  }, [])

  const runCheck = useCallback(async () => {
    if (!isDesktopApp() || busy.current) return
    busy.current = true
    const checkedAt = new Date().toISOString()
    setState((current) => beginCheck(current))
    try {
      await closeActiveUpdate()
      const update = await check({ timeout: 15000 })
      if (!mounted.current) {
        if (update) await update.close().catch(() => {})
        return
      }
      if (!update) {
        setState((current) => checkUpToDate(current, checkedAt))
        return
      }
      if (compareVersions(update.version, installedVersion) <= 0) {
        await update.close().catch(() => {})
        setState((current) => checkUpToDate(current, checkedAt))
        return
      }
      activeUpdate.current = update
      setState((current) => checkAvailable(current, {
        version: update.version,
        date: update.date,
        body: update.body,
      }, checkedAt))
    } catch (error) {
      if (mounted.current) {
        setState((current) => updateFailed({ ...current, lastCheckedAt: checkedAt }, error, 'check'))
      }
    } finally {
      busy.current = false
    }
  }, [closeActiveUpdate, installedVersion])

  const downloadAndInstall = useCallback(async () => {
    const update = activeUpdate.current
    if (!isDesktopApp() || !update || busy.current) return
    busy.current = true
    setState((current) => downloadStarted(current))
    try {
      await update.download((event) => {
        if (!mounted.current) return
        setState((current) => ({ ...current, progress: applyDownloadEvent(current.progress, event) }))
      })
      if (!mounted.current) return
      setState((current) => downloadFinished(current))
      await update.install({ restartAfterInstall: false })
      if (mounted.current) setState((current) => installFinished(current))
    } catch (error) {
      if (!mounted.current) return
      const retryAction = updaterErrorKind(error) === 'download' ? 'download' : 'check'
      setState((current) => updateFailed(current, error, retryAction))
    } finally {
      busy.current = false
    }
  }, [])

  const restartNow = useCallback(async () => {
    if (!isDesktopApp() || busy.current) return
    busy.current = true
    try {
      await relaunch()
    } catch (error) {
      if (mounted.current) setState((current) => restartFailed(current, error))
    } finally {
      busy.current = false
    }
  }, [])

  const retry = useCallback(() => {
    const next = retryUpdate(state)
    setState(next)
    if (next.error) return
    if (next.phase === 'restartReady') void restartNow()
    else if (next.phase === 'available') void downloadAndInstall()
    else void runCheck()
  }, [downloadAndInstall, restartNow, runCheck, state])

  const showProgress = state.phase === 'downloading' || state.phase === 'installing' || state.phase === 'restartReady'
  const progressWidth = state.progress.percent !== undefined ? `${state.progress.percent}%` : '18%'

  return (
    <section className="update-panel" aria-labelledby="update-title">
      <div className="update-heading">
        <div>
          <h3 id="update-title">Software updates</h3>
          <p>Updates come from the official RollaPlay releases and are verified before installation.</p>
        </div>
        <button
          className="button-text"
          onClick={() => void runCheck()}
          disabled={!isDesktopApp() || !canCheckForUpdates(state)}
        >
          <RefreshCw size={14} className={state.phase === 'checking' ? 'update-spin' : undefined} />
          Check for Updates
        </button>
      </div>

      <div className="update-status-grid" aria-live="polite">
        <div><span>Installed</span><strong>{state.installedVersion}</strong></div>
        <div><span>Latest</span><strong>{state.latestVersion ?? '—'}</strong></div>
        <div><span>Status</span><strong>{statusLabels[state.phase]}</strong></div>
        <div><span>Last checked</span><strong>{formatCheckedAt(state.lastCheckedAt)}</strong></div>
      </div>

      {!isDesktopApp() && (
        <div className="update-message"><AlertCircle size={15} /><span>Update checks are available in the Windows desktop app.</span></div>
      )}

      {state.phase === 'upToDate' && (
        <div className="update-message success"><Check size={15} /><span>You are running the latest signed release.</span></div>
      )}

      {state.phase === 'available' && (
        <div className="update-available">
          <div className="update-message success"><Check size={15} /><span>Version {state.latestVersion} is available. Published {formatReleaseDate(state.releaseDate)}.</span></div>
          <div className="update-release-heading">Release notes</div>
          <ReleaseNotes notes={state.notes} />
          <button className="button-primary" onClick={() => void downloadAndInstall()}>
            <Download size={15} /> Download &amp; Install
          </button>
          <button className="button-text update-releases-link" onClick={() => openExternal(RELEASES_URL)}>View all releases <span>→</span></button>
        </div>
      )}

      {showProgress && (
        <div className="update-progress" aria-live="polite">
          <div className="update-progress-label">
            <span>{statusLabels[state.phase]}</span>
            <span>
              {state.progress.total
                ? `${formatBytes(state.progress.loaded)} / ${formatBytes(state.progress.total)}`
                : formatBytes(state.progress.loaded)}
              {state.progress.percent !== undefined ? ` · ${state.progress.percent}%` : ''}
            </span>
          </div>
          <div
            className="update-progress-track"
            role="progressbar"
            aria-label="Update download progress"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={state.progress.percent ?? undefined}
          >
            <span style={{ width: progressWidth }} />
          </div>
        </div>
      )}

      {state.phase === 'restartReady' && (
        <div className="update-restart">
          <div className="update-message"><Check size={15} /><span>Install finished. Restart now to run version {state.latestVersion}. Windows may close RollaPlay while the installer runs; reopen it from the Start menu if needed.</span></div>
          <button className="button-primary" onClick={() => void restartNow()}><RotateCcw size={15} /> Restart Now</button>
        </div>
      )}

      {state.phase === 'error' && state.error && (
        <div className="update-message error" role="alert">
          <AlertCircle size={15} />
          <span>
            {state.error.message}
            {state.error.kind === 'signature' && ' Re-check after a correctly signed release is published.'}
            {' '}Your library, playlists and settings were not changed.
          </span>
          <button className="button-text update-retry" onClick={retry}>
            {state.error.retryAction === 'restart' ? <RotateCcw size={14} /> : <RefreshCw size={14} />}
            Retry
          </button>
        </div>
      )}

    </section>
  )
}
