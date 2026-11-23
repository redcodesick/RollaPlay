import { useEffect, useRef, useState } from 'react'
import useSWR from 'swr'
import {
  AlertCircle, Cast, Check, Globe, LoaderCircle, Pause, Play,
  RefreshCw, ShieldCheck, Square, Volume2, VolumeX,
} from 'lucide-react'
import { isDesktopApp } from '../media'
import {
  castDeviceName, castSendControl, castStateLabel, formatSeconds, getCastDiagnostics,
  getCastPlaybackState, getCastSenderStatus, isCastAvailable, launchCastSender, onCastPlaybackEvent,
  onCastSenderStatusChanged, playerStateLabel, sessionStateLabel, stopCastSender,
  type CastPlaybackSnapshot, type CastSenderStatus,
} from '../cast'

const senderWindowLabels: Record<string, string> = {
  not_launched: 'Cast sender not started',
  launching: 'Starting the Cast sender panel…',
  running: 'Cast sender connected',
  closed: 'Cast sender closed',
  failed: 'Cast sender failed',
}

function formatAgo(ms: number | null) {
  if (ms === null) return '—'
  if (ms < 1500) return 'just now'
  if (ms < 90_000) return `${Math.round(ms / 1000)}s ago`
  if (ms < 3_600_000) return `${Math.round(ms / 60_000)}m ago`
  return `${Math.round(ms / 3_600_000)}h ago`
}

function formatWhen(epochMs: number | null) {
  if (!epochMs) return 'Never'
  return new Date(epochMs).toLocaleString()
}

function discoveryHint(snapshot: CastPlaybackSnapshot, status: CastSenderStatus | undefined) {
  if (status && !status.chrome_installed) {
    return 'Chrome is not installed. Install Google Chrome on this PC, then launch the sender again.'
  }
  if (snapshot.cast_state === 'NO_DEVICES_AVAILABLE') {
    return 'No Cast devices found. Make sure the Xiaomi Mi Box is on the same Wi-Fi network and powered up, then wait a moment.'
  }
  if (snapshot.last_error) return snapshot.last_error
  return undefined
}

export default function CastPanel() {
  const isDesktop = isDesktopApp()
  const castReady = isCastAvailable()
  const [isWorking, setIsWorking] = useState(false)
  const [message, setMessage] = useState('')
  const [errorMessage, setErrorMessage] = useState('')
  const [seekPreview, setSeekPreview] = useState<number | null>(null)
  const [volumePreview, setVolumePreview] = useState<number | null>(null)
  const [playingOverride, setPlayingOverride] = useState<boolean | null>(null)
  const seekDragging = useRef(false)
  const volumeDragging = useRef(false)
  const volumeTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const { data: status, mutate: refreshStatus } = useSWR(
    isDesktop ? 'cast-sender-status' : null,
    getCastSenderStatus,
    { refreshInterval: 2500 },
  )
  const { data: diagnostics, mutate: refreshDiagnostics } = useSWR(
    isDesktop ? 'cast-diagnostics' : null,
    getCastDiagnostics,
    { refreshInterval: 3000 },
  )
  const { data: playback, mutate: refreshPlayback } = useSWR(
    isDesktop ? 'cast-playback-state' : null,
    getCastPlaybackState,
    { refreshInterval: 1500 },
  )

  useEffect(() => {
    if (!castReady) return
    let disposed = false
    let stops: (() => void)[] = []
    Promise.all([
      onCastPlaybackEvent((event) => {
        if (event.kind === 'load_failed' || event.kind === 'sender_error' || event.kind === 'session_start_failed') {
          setErrorMessage(event.message ?? 'The receiver reported an error during playback.')
        }
        void refreshPlayback()
        void refreshDiagnostics()
        void refreshStatus()
      }),
      onCastSenderStatusChanged(() => {
        void refreshStatus()
        void refreshDiagnostics()
      }),
    ]).then((stopListening) => {
      if (disposed) stopListening.forEach((stop) => stop())
      else stops = stopListening
    }).catch(() => setErrorMessage('Cast background events are unavailable.'))
    return () => {
      disposed = true
      stops.forEach((stop) => stop())
    }
  }, [castReady, refreshDiagnostics, refreshPlayback, refreshStatus])

  async function runAction(action: () => Promise<unknown>, successText?: string): Promise<boolean> {
    setIsWorking(true)
    setErrorMessage('')
    setMessage('')
    try {
      await action()
      await Promise.all([refreshStatus(), refreshDiagnostics(), refreshPlayback()])
      if (successText) setMessage(successText)
      return true
    } catch (error) {
      setErrorMessage(typeof error === 'string' ? error : error instanceof Error ? error.message : String(error))
      return false
    } finally {
      setIsWorking(false)
    }
  }

  async function togglePlay() {
    const next = !playing
    setPlayingOverride(next)
    const ok = await runAction(() => castSendControl('toggle_play'))
    if (!ok) setPlayingOverride(null)
  }

  async function commitSeek() {
    seekDragging.current = false
    const target = seekPreview
    setSeekPreview(null)
    if (target === null || target === undefined) return
    await runAction(() => castSendControl('seek', { positionSeconds: target }))
  }

  async function commitVolume(target?: number) {
    volumeDragging.current = false
    if (volumeTimer.current) {
      clearTimeout(volumeTimer.current)
      volumeTimer.current = null
    }
    const value = target ?? volumePreview
    if (value === null || value === undefined) return
    await runAction(() => castSendControl('set_volume', { volume: value }))
  }

  useEffect(() => () => {
    if (volumeTimer.current) clearTimeout(volumeTimer.current)
  }, [])

  const snapshot = playback ?? ({ updated_at_epoch_ms: 0 } as CastPlaybackSnapshot)
  const connectedDevice = castDeviceName(snapshot)
  const connected = !!connectedDevice
  const hasMedia = snapshot.media_file_id !== null && snapshot.media_file_id !== undefined
  const playing = playingOverride ?? snapshot.player_state === 'PLAYING'
  useEffect(() => {
    if (playingOverride !== null && snapshot.player_state && (snapshot.player_state === 'PLAYING') === playingOverride) {
      setPlayingOverride(null)
    }
  }, [snapshot.player_state, playingOverride])
  useEffect(() => {
    const receiverVolume = snapshot.volume_level
    if (volumePreview !== null && receiverVolume !== null && receiverVolume !== undefined && Math.abs(receiverVolume - volumePreview) < 0.02) {
      setVolumePreview(null)
    }
  }, [snapshot.volume_level, volumePreview])
  const position = seekPreview ?? snapshot.position_seconds ?? 0
  const duration = snapshot.duration_seconds ?? 0
  const hint = discoveryHint(snapshot, status)
  const volume = snapshot.volume_level ?? 1
  const muted = !!snapshot.muted
  const deviceLabel = connected ? `Casting to ${connectedDevice}` : 'No Cast device connected'
  const stateClass = connected ? (playing ? 'playing' : 'paused') : 'offline'

  return (
    <section className="cast-panel" aria-labelledby="cast-panel-title">
      <div className="cast-section-heading">
        <div>
          <div className="section-eyebrow"><span className="eyebrow-line" /> PHASE 4 · GOOGLE CAST</div>
          <h2 id="cast-panel-title">Cast connection</h2>
          <p>Stream registered videos to a Cast receiver — for example a Xiaomi Mi Box — over your home network.</p>
        </div>
        <div className={`cast-status-pill ${stateClass}`} role="status"><i />{deviceLabel}</div>
      </div>

      {!isDesktop && <div className="library-runtime-note"><ShieldCheck size={16} /><span>Cast controls and diagnostics are available in the installed Windows app. This preview does not launch Chrome or the Cast sender.</span></div>}
      {errorMessage && <div className="library-message error" role="alert"><AlertCircle size={16} />{errorMessage}</div>}
      {message && <div className="library-message success" role="status"><Check size={16} />{message}</div>}
      {hint && !errorMessage && <div className="library-message info" role="status"><AlertCircle size={16} />{hint}</div>}

      <div className="cast-info-grid">
        <article className="cast-info-card"><span><Globe size={15} /> GOOGLE CHROME</span><strong>{status?.chrome_installed ? 'Detected' : 'Not found'}</strong><small>{status?.chrome_installed ? status.chrome_path ?? 'Registered install location' : 'The official Cast sender SDK requires Chrome'}</small></article>
        <article className="cast-info-card"><span><Cast size={15} /> SENDER WINDOW</span><strong>{senderWindowLabels[status?.sender_window_state ?? 'not_launched'] ?? 'Unknown'}</strong><small>Sender page {status?.sender_page_connected ? 'connected' : 'not connected'} · last seen {formatAgo(status?.sender_page_last_seen_ms_ago ?? null)}</small></article>
        <article className="cast-info-card"><span><Cast size={15} /> DISCOVERY</span><strong>{castStateLabel(snapshot.cast_state)}</strong><small>Session: {sessionStateLabel(snapshot.session_state)}</small></article>
        <article className="cast-info-card"><span><Cast size={15} /> CONNECTED DEVICE</span><strong>{connectedDevice ?? 'None'}</strong><small>{status?.bridge_state === 'running' ? `Local sender bridge on 127.0.0.1:${status.bridge_port ?? ''}` : 'Local sender bridge stopped'}</small></article>
      </div>

      <div className="cast-controls-card">
        <div className="cast-controls-heading">
          <div>
            <h3>Cast sender</h3>
            <p>The player stays in this window. Chrome runs only the official Cast SDK inside the sender panel; once a receiver is selected it stays connected, and hiding the panel does not stop playback.</p>
          </div>
          <div className="cast-controls-actions">
            <button className="button-primary" onClick={() => runAction(() => launchCastSender(), connectedDevice ? `Sender ready. Casting to ${connectedDevice}.` : 'Sender ready. Choose your TV in the sender panel.')} disabled={!isDesktop || isWorking}>
              {isWorking ? <LoaderCircle size={15} className="spin-icon" /> : <Cast size={15} />} Launch sender
            </button>
            <button className="folder-action" onClick={() => runAction(() => stopCastSender(), 'The Cast sender was closed.')} disabled={!isDesktop || isWorking}>Close sender</button>
          </div>
        </div>
        {status?.last_error && <p className="cast-error-line" role="status"><AlertCircle size={14} />{status.last_error}</p>}
      </div>

      <div className="cast-now-playing" aria-label="Cast playback">
        <div className="cast-now-track">
          <span className="cast-now-icon"><Cast size={16} /></span>
          <div className="cast-now-meta">
            <strong>{snapshot.filename ?? 'Nothing is casting'}</strong>
            <span>{playerStateLabel(snapshot.player_state)} · {deviceLabel}</span>
          </div>
          {hasMedia && <span className="cast-token-note" title="The streaming URL expires and renews automatically while the receiver is fetching the video.">Scoped URL active</span>}
        </div>
        <div className="cast-slider-row">
          <input
            type="range"
            min={0}
            max={Math.max(1, Math.round(duration * 1000))}
            step={1}
            value={Math.max(0, Math.round(position * 1000))}
            aria-label="Seek position"
            disabled={!hasMedia || !connected || !isDesktop}
            onPointerDown={() => { seekDragging.current = true }}
            onChange={(event) => setSeekPreview(Number(event.target.value) / 1000)}
            onPointerUp={commitSeek}
            onTouchEnd={commitSeek}
            onKeyUp={(event) => { if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') void commitSeek() }}
          />
          <div className="cast-times"><span>{formatSeconds(position)}</span><span>{formatSeconds(duration)}</span></div>
        </div>
        <div className="cast-controls">
          <button className="cast-control primary" onClick={() => void togglePlay()} disabled={!hasMedia || !connected || !isDesktop} aria-label={playing ? 'Pause' : 'Play'}>
            {playing ? <Pause size={16} /> : <Play size={16} fill="currentColor" />}
            {playing ? 'Pause' : 'Play'}
          </button>
          <button className="cast-control" onClick={() => runAction(() => castSendControl('stop'))} disabled={!hasMedia || !connected || !isDesktop} aria-label="Stop">
            <Square size={14} /> Stop
          </button>
          <button className="cast-control" onClick={() => runAction(() => castSendControl('set_mute', { muted: !muted }))} disabled={!connected || !isDesktop} aria-label={muted ? 'Unmute' : 'Mute'}>
            {muted ? <VolumeX size={15} /> : <Volume2 size={15} />} {muted ? 'Unmute' : 'Mute'}
          </button>
          <button className="cast-control danger" onClick={() => runAction(() => castSendControl('disconnect'), 'The Cast session ended. Your library and playback position are kept.')} disabled={!connected || !isDesktop} aria-label="Disconnect">
            <Cast size={15} /> Disconnect
          </button>
        </div>
        <div className="cast-slider-row">
          <label htmlFor="cast-volume">Receiver volume · {Math.round((volumePreview ?? volume) * 100)}%</label>
          <input
            id="cast-volume"
            type="range"
            min={0}
            max={1}
            step={0.01}
            value={volumePreview ?? volume}
            aria-label="Receiver volume"
            disabled={!connected || !isDesktop}
            onPointerDown={() => { volumeDragging.current = true }}
            onChange={(event) => {
              const value = Number(event.target.value)
              setVolumePreview(value)
              if (volumeTimer.current) clearTimeout(volumeTimer.current)
              volumeTimer.current = setTimeout(() => { void commitVolume(value) }, 120)
            }}
            onPointerUp={() => { void commitVolume() }}
            onTouchEnd={() => { void commitVolume() }}
            onKeyUp={(event) => { if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') void commitVolume() }}
          />
        </div>
      </div>

      <div className="cast-diagnostics-card">
        <div className="cast-controls-heading">
          <div>
            <h3>Cast diagnostics</h3>
            <p>Sender, discovery, and streaming health. Credentials are never displayed here.</p>
          </div>
          <button className="folder-action" onClick={() => runAction(() => refreshDiagnostics())} disabled={!isDesktop}>
            <RefreshCw size={14} /> Refresh
          </button>
        </div>
        <div className="cast-diagnostic-list">
          <article className="cast-diagnostic-row"><div><strong>Sender status</strong><span>{status?.sender_page_connected ? 'Connected to the desktop bridge' : 'Waiting for the Chrome sender page'}</span></div><span className={status?.sender_page_connected ? 'cast-ok' : ''}>{status?.sender_page_connected ? 'OK' : 'IDLE'}</span></article>
          <article className="cast-diagnostic-row"><div><strong>Chrome status</strong><span>{status?.chrome_installed ? status.chrome_path ?? 'Detected' : 'Chrome is not installed on this PC'}</span></div><span className={status?.chrome_installed ? 'cast-ok' : 'cast-bad'}>{status?.chrome_installed ? 'OK' : 'MISSING'}</span></article>
          <article className="cast-diagnostic-row"><div><strong>Discovery status</strong><span>{castStateLabel(snapshot.cast_state)}</span></div><span className={snapshot.cast_state === 'CONNECTED' ? 'cast-ok' : ''}>{snapshot.cast_state ?? 'IDLE'}</span></article>
          <article className="cast-diagnostic-row"><div><strong>Connected device</strong><span>{connectedDevice ?? 'None yet'}</span></div><span>{connectedDevice ? 'ACTIVE' : 'NONE'}</span></article>
          <article className="cast-diagnostic-row"><div><strong>Current session</strong><span>{sessionStateLabel(snapshot.session_state)}</span></div><span>{snapshot.session_state ?? 'NONE'}</span></article>
          <article className="cast-diagnostic-row"><div><strong>Media URL reachability</strong><span>{diagnostics?.media_url_probe ? diagnostics.media_url_probe.message : 'Probe a cast to check the LAN URL'}</span></div><span className={diagnostics?.media_url_probe?.reachable ? 'cast-ok' : ''}>{diagnostics?.media_url_probe ? (diagnostics.media_url_probe.reachable ? `HTTP ${diagnostics.media_url_probe.status_code ?? 206}` : 'FAILED') : '—'}</span></article>
          <article className="cast-diagnostic-row"><div><strong>Active streaming grants</strong><span>Short-lived, media-scoped receiver credentials held in memory</span></div><span>{diagnostics?.active_cast_grants ?? 0}</span></article>
          <article className="cast-diagnostic-row"><div><strong>Last error</strong><span>{snapshot.last_error ?? 'None recorded'}</span></div><span>{snapshot.last_error ? 'ERROR' : 'OK'}</span></article>
          <article className="cast-diagnostic-row"><div><strong>Last connection attempt</strong><span>{formatWhen(status?.last_launch_attempt_epoch_ms ?? null)}{status?.relaunch_attempts ? ` · ${status.relaunch_attempts} recovery attempt${status.relaunch_attempts === 1 ? '' : 's'}` : ''}</span></div><span>{status?.last_launch_attempt_epoch_ms ? 'DONE' : '—'}</span></article>
        </div>
        <div className="cast-network-note"><ShieldCheck size={15} /><span>The sender page receives only the minimum needed to cast: a scoped, expiring media URL, its MIME type, and the title. The pairing token and DPAPI secrets never reach Chrome or the receiver.</span></div>
      </div>
    </section>
  )
}
