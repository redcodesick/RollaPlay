import { useEffect, useRef, useState } from 'react'
import useSWR from 'swr'
import {
  AlertCircle, Cast, Check, LoaderCircle, Monitor, RefreshCw, Tv, X,
} from 'lucide-react'
import {
  castDeviceName, castRequestSession, castSendControl, castStateLabel, discoverCastDevices,
  launchCastSender, onCastPlaybackEvent, type CastDevice, type CastPlaybackSnapshot,
} from '../cast'

type Notice = (message: string, level?: 'info' | 'success' | 'warning' | 'error') => void

type Props = {
  open: boolean
  onClose: () => void
  onNotice: Notice
  playback: CastPlaybackSnapshot | null
  localPlaying: boolean
}

type DeviceTone = 'playing' | 'connected' | 'connecting' | 'failed' | 'available'

function toneLabel(tone: DeviceTone) {
  switch (tone) {
    case 'playing': return 'PLAYING HERE'
    case 'connected': return 'CONNECTED'
    case 'connecting': return 'CHOOSE IN DIALOG'
    case 'failed': return 'FAILED'
    default: return 'AVAILABLE'
  }
}

export default function ConnectPanel({
  open, onClose, onNotice, playback, localPlaying,
}: Props) {
  const panelRef = useRef<HTMLElement | null>(null)
  const pendingRef = useRef<CastDevice | null>(null)
  const [pending, setPending] = useState<CastDevice | null>(null)
  const [connecting, setConnecting] = useState(false)
  const [failed, setFailed] = useState<string | null>(null)
  const {
    data: devices = [], error: discoveryError, isLoading, mutate: refreshDevices,
  } = useSWR<CastDevice[]>(open ? 'cast-devices' : null, discoverCastDevices, {
    refreshInterval: open ? 10_000 : 0,
    revalidateOnFocus: false,
  })

  const connectedDevice = castDeviceName(playback)
  const receiverPlaying = connectedDevice && playback?.player_state === 'PLAYING'

  useEffect(() => { pendingRef.current = pending }, [pending])

  useEffect(() => {
    if (!open) return
    let active = true
    let closeTimer: number | undefined
    const unlisten = onCastPlaybackEvent((event) => {
      if (!active) return
      if (event.kind === 'session_started' || event.kind === 'session_resumed') {
        const selectedHere = !!pendingRef.current
        setPending(null)
        setConnecting(false)
        setFailed(null)
        void refreshDevices()
        // Only a receiver chosen from this menu may dismiss it. A restored
        // session or a stale CONNECTED state must leave the list visible.
        if (selectedHere) {
          closeTimer = window.setTimeout(() => { if (active) onClose() }, 400)
        }
      } else if (event.kind === 'session_start_failed') {
        setFailed(event.message ?? 'The receiver refused the connection.')
        setConnecting(false)
      } else if (event.kind === 'session_ended') {
        setPending(null)
        setConnecting(false)
      }
    })
    return () => {
      active = false
      if (closeTimer !== undefined) window.clearTimeout(closeTimer)
      void unlisten.then((off) => off())
    }
  }, [open, refreshDevices, onClose])

  useEffect(() => {
    if (!open) {
      setPending(null)
      setConnecting(false)
      setFailed(null)
    }
  }, [open])

  // Behave like a menu anchored to the Cast pill, not a blocking modal.
  useEffect(() => {
    if (!open) return
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target
      if (!(target instanceof Element)) return
      if (panelRef.current?.contains(target) || target.closest('.device-pill')) return
      onClose()
    }
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    window.addEventListener('pointerdown', onPointerDown)
    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('pointerdown', onPointerDown)
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [open, onClose])

  const toneFor = (device: CastDevice): DeviceTone => {
    if (pending?.id === device.id && connecting) return 'connecting'
    if (pending?.id === device.id && failed) return 'failed'
    if (connectedDevice && device.name.toLocaleLowerCase() === connectedDevice.toLocaleLowerCase()) {
      return receiverPlaying ? 'playing' : 'connected'
    }
    return 'available'
  }

  async function connect(device: CastDevice) {
    if (connecting) return
    setPending(device)
    setConnecting(true)
    setFailed(null)
    try {
      await launchCastSender()
      await castRequestSession(device)
      onNotice(`${device.name} selected. Choose it in the Cast dialog to finish connecting.`, 'info')
    } catch (error) {
      const message = typeof error === 'string' ? error : error instanceof Error ? error.message : String(error)
      setFailed(message)
      setConnecting(false)
      onNotice(message, 'error')
    }
  }

  function cancelConnection() {
    setPending(null)
    setConnecting(false)
    setFailed(null)
  }

  async function switchDevice() {
    try {
      await castSendControl('disconnect')
      onNotice('Cast session ended. Choose another receiver.', 'info')
    } catch (error) {
      onNotice(typeof error === 'string' ? error : error instanceof Error ? error.message : String(error), 'error')
    }
  }

  if (!open) return null

  return (
    <section ref={panelRef} className="connect-panel" role="dialog" aria-label="Cast devices" aria-labelledby="connect-title">
      <header className="connect-header">
          <div className="connect-brand">
            <span className="connect-mark" aria-hidden="true">RP</span>
            <div><b>ROLLA<b className="connect-emphasis">PLAY</b></b><span>ROCKNROLLA MEDIA STATION</span></div>
          </div>
          <button className="connect-close" onClick={onClose} aria-label="Close Connect panel"><X size={17} /></button>
        </header>

        <div className="connect-heading">
          <div className="connect-eyebrow">PLAYBACK DESTINATION</div>
          <h2 id="connect-title">CONNECT</h2>
          <p>Choose where ROLLAPLAY should play. Devices below are found on your local network.</p>
        </div>

        <div className={`connect-current ${localPlaying ? 'local' : ''} ${connectedDevice ? 'remote' : ''}`}>
          <span className="connect-current-icon">
            {connectedDevice ? <Tv size={19} /> : <Monitor size={19} />}
          </span>
          <div className="connect-current-copy">
            <strong>{connectedDevice ?? 'This PC'}</strong>
            <span>{connectedDevice ? `Google Cast · ${castStateLabel(playback?.cast_state)}` : 'Local playback · no receiver connected'}</span>
          </div>
          <span className={`connect-indicator ${receiverPlaying || (!connectedDevice && localPlaying) ? 'live' : ''}`}>
            {(receiverPlaying || (!connectedDevice && localPlaying)) && <i />}
            {receiverPlaying || (!connectedDevice && localPlaying) ? 'PLAYING HERE' : 'READY'}
          </span>
          {connectedDevice && <button className="connect-switch" onClick={() => void switchDevice()} title="Disconnect to choose another receiver">SWITCH</button>}
        </div>

        <div className="connect-list-head">
          <span>NEARBY DEVICES</span>
          <button onClick={() => void refreshDevices()} disabled={isLoading} aria-label="Refresh Cast devices">
            <RefreshCw size={13} className={isLoading ? 'spin-icon' : ''} /> REFRESH DEVICES
          </button>
        </div>

        {failed && <div className="connect-error" role="alert"><AlertCircle size={14} /><span>{failed}</span></div>}
        {discoveryError && !failed && <div className="connect-error" role="alert"><AlertCircle size={14} /><span>Cast discovery is unavailable on this network.</span></div>}

        <div className="connect-list" role="list">
          {!devices.length && isLoading && (
            <div className="connect-empty">
              <LoaderCircle size={20} className="spin-icon" />
              <strong>Searching for Cast devices…</strong>
              <span>Checking every local network adapter for Google Cast receivers.</span>
            </div>
          )}
          {!devices.length && !isLoading && (
            <div className="connect-empty">
              <Cast size={20} />
              <strong>No Cast devices found</strong>
              <span>Make sure your receiver is powered on and connected to the same LAN, then refresh.</span>
            </div>
          )}
          {devices.map((device) => {
            const tone = toneFor(device)
            const active = tone === 'connected' || tone === 'playing'
            return (
              <button
                key={device.id}
                role="listitem"
                className={`connect-device tone-${tone} ${active ? 'active' : ''}`}
                onClick={() => void connect(device)}
                disabled={active || connecting}
              >
                <span className="connect-device-icon"><Tv size={18} /></span>
                <span className="connect-device-copy">
                  <strong>{device.name}</strong>
                  <small>{device.model || 'Google Cast'} · Google Cast</small>
                </span>
                <span className={`connect-device-state state-${tone}`}>{tone === 'connecting' && <LoaderCircle size={12} className="spin-icon" />}{tone === 'connected' || tone === 'playing' ? <Check size={12} /> : null}{toneLabel(tone)}</span>
              </button>
            )
          })}
        </div>

        {connecting && pending && (
          <div className="connect-picker-note" role="status">
            <LoaderCircle size={14} className="spin-icon" />
            <span>Choose <b>{pending.name}</b> in Chrome’s Cast dialog. The Web SDK cannot preselect a receiver from the page.</span>
            <button onClick={cancelConnection}>Cancel</button>
          </div>
        )}

        <footer className="connect-footer">
          <div><AlertCircle size={13} /><span>DON&apos;T SEE YOUR DEVICE?</span></div>
          <button onClick={() => void refreshDevices()} disabled={isLoading}>REFRESH DEVICES</button>
        </footer>
    </section>
  )
}
