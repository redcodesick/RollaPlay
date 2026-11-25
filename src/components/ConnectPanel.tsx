import { useEffect, useRef, useState } from 'react'
import useSWR from 'swr'
import {
  AlertCircle, Cast, Check, ChevronDown, LoaderCircle, Monitor, Pause, Play,
  Power, RefreshCw, Speaker, Square, Tv, Volume2, VolumeX, X,
} from 'lucide-react'
import {
  castDeviceName, castRequestSession, castSendControl, castStateLabel, discoverCastDevices,
  discoverLgTvDevices, disconnectDlna, disconnectLgTv, getDlnaStatus, getLgTvInputs, getLgTvStatus,
  launchCastSender, lgTvControl, lgTvOpenBrowser, lgTvSwitchInput, onCastPlaybackEvent,
  pairLgTv, refreshLgTv, selectDlnaDevice, selectLgTvDevice,
  type CastDevice, type CastPlaybackSnapshot, type DlnaStatus, type LgTvDevice,
  type LgTvInput, type LgTvStatus,
} from '../cast'

type Notice = (message: string, level?: 'info' | 'success' | 'warning' | 'error') => void

type Props = {
  open: boolean
  onClose: () => void
  onNotice: Notice
  playback: CastPlaybackSnapshot | null
}

type DeviceTone = 'playing' | 'connected' | 'connecting' | 'failed' | 'available'

function errorMessage(error: unknown) {
  return typeof error === 'string' ? error : error instanceof Error ? error.message : String(error)
}

function toneLabel(tone: DeviceTone, transport: string) {
  switch (tone) {
    case 'playing': return 'PLAYING HERE'
    case 'connected': return transport === 'dlna' ? 'DLNA READY' : 'CONNECTED'
    case 'connecting': return transport === 'dlna' ? 'SELECTING' : 'CHOOSE IN CHROME'
    case 'failed': return 'FAILED'
    default: return transport === 'dlna' ? 'DLNA DEVICE' : 'AVAILABLE'
  }
}

function deviceLabel(device: CastDevice | LgTvDevice) {
  const name = device.name?.trim()
  if (name && !/^(cast|chromecast|google cast|tv|television|device|receiver|lge)(\s+(device|receiver|0x[0-9a-f]+))?$/i.test(name)) return name
  const model = device.model?.trim()
  if (model && model.toLocaleLowerCase() !== name?.toLocaleLowerCase()) return `${model} receiver`
  return `Receiver ${device.id.slice(0, 8)}`
}

function DeviceIcon({ device }: { device: CastDevice }) {
  const label = `${device.name ?? ''} ${device.model ?? ''}`.toLocaleLowerCase()
  if (/speaker|audio|soundbar/i.test(label)) return <Speaker size={18} />
  if (/display|monitor|desktop|laptop/i.test(label)) return <Monitor size={18} />
  return <Tv size={18} />
}

export default function ConnectPanel({
  open, onClose, onNotice, playback,
}: Props) {
  const panelRef = useRef<HTMLElement | null>(null)
  const pendingRef = useRef<CastDevice | null>(null)
  const [pending, setPending] = useState<CastDevice | null>(null)
  const [connecting, setConnecting] = useState(false)
  const [failed, setFailed] = useState<string | null>(null)
  const [dlnaStatus, setDlnaStatus] = useState<DlnaStatus | null>(null)
  const [lgStatus, setLgStatus] = useState<LgTvStatus | null>(null)
  const [lgBusy, setLgBusy] = useState(false)
  const [lgInputs, setLgInputs] = useState<LgTvInput[]>([])
  const [lgInputId, setLgInputId] = useState('')

  const {
    data: devices = [], error: discoveryError, isLoading, mutate: refreshDevices,
  } = useSWR<CastDevice[]>(open ? 'cast-devices' : null, discoverCastDevices, {
    refreshInterval: open ? 10_000 : 0,
    revalidateOnFocus: false,
  })
  // LG discovery is a separate real SSDP pass. Keeping it out of the Cast
  // command means a TV with webOS network control disabled cannot break Cast.
  const {
    data: lgDevices = [], error: lgDiscoveryError, isLoading: lgLoading,
    mutate: refreshLgDevices,
  } = useSWR<LgTvDevice[]>(open ? 'lg-tv-devices' : null, discoverLgTvDevices, {
    refreshInterval: open ? 20_000 : 0,
    revalidateOnFocus: false,
  })

  useEffect(() => {
    if (!open) return
    let active = true
    const update = async () => {
      try {
        if (active) setDlnaStatus(await getDlnaStatus())
      } catch {
        if (active) setDlnaStatus(null)
      }
    }
    void update()
    const timer = window.setInterval(() => void update(), 3_000)
    return () => {
      active = false
      window.clearInterval(timer)
    }
  }, [open])

  useEffect(() => {
    if (!open) return
    let active = true
    const update = async () => {
      try {
        if (active) setLgStatus(await getLgTvStatus())
      } catch {
        if (active) setLgStatus(null)
      }
    }
    void update()
    const timer = window.setInterval(() => void update(), 5_000)
    return () => {
      active = false
      window.clearInterval(timer)
    }
  }, [open])

  const connectedDevice = castDeviceName(playback)
  const receiverPlaying = connectedDevice && playback?.player_state === 'PLAYING'
  const receiverPaused = connectedDevice && playback?.player_state === 'PAUSED'
  const activeIsDlna = !!dlnaStatus?.selected && connectedDevice
    ?.toLocaleLowerCase() === dlnaStatus.selected.name.toLocaleLowerCase()

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
      setLgBusy(false)
      setLgInputs([])
      setLgInputId('')
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
    if (device.transport === 'dlna' && dlnaStatus?.selected?.id === device.id) {
      return receiverPlaying ? 'playing' : 'connected'
    }
    if (device.transport !== 'dlna' && connectedDevice
      && device.name.toLocaleLowerCase() === connectedDevice.toLocaleLowerCase()) {
      return receiverPlaying ? 'playing' : 'connected'
    }
    return 'available'
  }

  async function endCastSession() {
    try {
      await castSendControl('disconnect')
    } catch {
      // No Chrome receiver may be connected; selection can still proceed.
    }
  }

  async function connect(device: CastDevice) {
    if (connecting) return
    setPending(device)
    setConnecting(true)
    setFailed(null)
    try {
      if (device.transport === 'dlna') {
        await endCastSession()
        const status = await selectDlnaDevice(device)
        setDlnaStatus(status)
        onNotice(`${device.name} selected. Video will play through its DLNA AVTransport service.`, 'success')
      } else {
        await disconnectDlna().catch(() => undefined)
        setDlnaStatus(await getDlnaStatus().catch(() => null))
        await launchCastSender()
        await castRequestSession(device)
        onNotice(`Opening Chrome’s Cast dialog for ${device.name}. Choose it to finish connecting.`, 'info')
      }
      if (device.transport === 'dlna') {
        setPending(null)
        setConnecting(false)
      }
    } catch (error) {
      const message = errorMessage(error)
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
      if (dlnaStatus?.selected) {
        await disconnectDlna()
        setDlnaStatus(await getDlnaStatus().catch(() => null))
      } else {
        await castSendControl('disconnect')
      }
      onNotice('Receiver disconnected. Choose another device.', 'info')
    } catch (error) {
      onNotice(errorMessage(error), 'error')
    }
  }

  async function selectLg(device: LgTvDevice) {
    if (lgBusy) return
    setLgBusy(true)
    try {
      const status = await selectLgTvDevice(device)
      setLgStatus(status)
      setLgInputs([])
      setLgInputId('')
      onNotice(
        status.paired
          ? `${device.name} selected with its saved pairing key.`
          : `${device.name} selected. Pair with the TV to enable controls.`,
        status.paired ? 'success' : 'info',
      )
    } catch (error) {
      onNotice(errorMessage(error), 'error')
    } finally {
      setLgBusy(false)
    }
  }

  async function pairLg() {
    if (lgBusy) return
    setLgBusy(true)
    try {
      const result = await pairLgTv()
      setLgStatus(await getLgTvStatus())
      onNotice(result.message, result.status === 'paired' ? 'success' : 'info')
    } catch (error) {
      onNotice(errorMessage(error), 'error')
    } finally {
      setLgBusy(false)
    }
  }

  async function sendLg(action: string, value?: number) {
    if (lgBusy) return
    setLgBusy(true)
    try {
      setLgStatus(await lgTvControl(action, value))
    } catch (error) {
      onNotice(errorMessage(error), 'error')
    } finally {
      setLgBusy(false)
    }
  }

  async function loadLgInputs() {
    if (lgBusy) return
    setLgBusy(true)
    try {
      const inputs = await getLgTvInputs()
      setLgInputs(inputs)
      setLgInputId(inputs.find((input) => input.connected)?.id ?? inputs[0]?.id ?? '')
      if (!inputs.length) onNotice('The TV did not report external inputs.', 'info')
    } catch (error) {
      onNotice(errorMessage(error), 'error')
    } finally {
      setLgBusy(false)
    }
  }

  async function switchLgInput() {
    if (!lgInputId) return
    setLgBusy(true)
    try {
      setLgStatus(await lgTvSwitchInput(lgInputId))
    } catch (error) {
      onNotice(errorMessage(error), 'error')
    } finally {
      setLgBusy(false)
    }
  }

  async function openLocalBrowser() {
    if (lgBusy) return
    const endpoint = window.prompt('Plain local URL to open in the TV browser (no token or query string):')
    if (!endpoint) return
    setLgBusy(true)
    try {
      await lgTvOpenBrowser(endpoint)
      onNotice('The TV was asked to open the local URL.', 'success')
    } catch (error) {
      onNotice(errorMessage(error), 'error')
    } finally {
      setLgBusy(false)
    }
  }

  async function disconnectLg() {
    if (lgBusy) return
    try {
      await disconnectLgTv()
      setLgStatus(await getLgTvStatus())
      setLgInputs([])
      setLgInputId('')
    } catch (error) {
      onNotice(errorMessage(error), 'error')
    }
  }

  if (!open) return null

  return (
    <section ref={panelRef} className="connect-panel" role="dialog" aria-label="Cast devices" aria-labelledby="connect-title">
      <header className="connect-header">
        <div>
          <span className="connect-eyebrow">CAST CONNECT</span>
          <h2 id="connect-title">Connect</h2>
        </div>
        <button className="connect-close" onClick={onClose} aria-label="Close Connect panel"><X size={17} /></button>
      </header>

      {connectedDevice && (
        <div className="connect-current">
          <span className="connect-current-icon"><Tv size={19} /></span>
          <div className="connect-current-copy">
            <strong>{connectedDevice}</strong>
            <span>{activeIsDlna ? 'DLNA / UPnP' : 'Google Cast'} · {castStateLabel(playback?.cast_state)}</span>
          </div>
          <span className={`connect-indicator ${receiverPlaying ? 'live' : receiverPaused ? 'paused' : ''}`}>
            {receiverPlaying && <i />}
            {receiverPlaying ? 'PLAYING HERE' : receiverPaused ? 'PAUSED' : 'READY'}
          </span>
          <button className="connect-switch" onClick={() => void switchDevice()} title="Disconnect to choose another receiver">SWITCH</button>
        </div>
      )}

      <div className="connect-list-head">
        <span>NEARBY RECEIVERS</span>
        <button onClick={() => void refreshDevices()} disabled={isLoading} aria-label="Refresh receivers">
          <RefreshCw size={13} className={isLoading ? 'spin-icon' : ''} /> REFRESH DEVICES
        </button>
      </div>

      {failed && <div className="connect-error" role="alert"><AlertCircle size={14} /><span>{failed}</span></div>}
      {discoveryError && !failed && <div className="connect-error" role="alert"><AlertCircle size={14} /><span>Receiver discovery is unavailable on this network.</span></div>}

      <div className="connect-list" role="list">
        {!devices.length && isLoading && (
          <div className="connect-empty">
            <LoaderCircle size={20} className="spin-icon" />
            <strong>Searching for receivers…</strong>
            <span>Checking every local network adapter for Cast and DLNA devices.</span>
          </div>
        )}
        {!devices.length && !isLoading && (
          <div className="connect-empty">
            <Cast size={20} />
            <strong>No receivers found</strong>
            <span>Make sure the receiver is powered on and connected to the same LAN, then refresh.</span>
          </div>
        )}
        {devices.map((device) => {
          const tone = toneFor(device)
          const active = tone === 'connected' || tone === 'playing'
          const transport = device.transport === 'dlna' ? 'dlna' : 'cast'
          return (
            <button
              key={device.id}
              role="listitem"
              className={`connect-device tone-${tone} ${active ? 'active' : ''}`}
              aria-current={active ? 'true' : undefined}
              onClick={() => void connect(device)}
              disabled={active || connecting}
            >
              <span className="connect-device-icon"><DeviceIcon device={device} /></span>
              <span className="connect-device-copy">
                <strong title={device.name}>{deviceLabel(device)}</strong>
                <small>{device.model || (transport === 'dlna' ? 'DLNA MediaRenderer' : 'Google Cast')} · {device.address || 'Local network'}</small>
              </span>
              <span className={`connect-device-state state-${tone}`}>
                {tone === 'connecting' && <LoaderCircle size={12} className="spin-icon" />}
                {tone === 'connected' || tone === 'playing' ? <Check size={12} /> : null}
                {toneLabel(tone, transport)}
              </span>
            </button>
          )
        })}
      </div>

      {connecting && pending && pending.transport !== 'dlna' && (
        <div className="connect-picker-note" role="status">
          <LoaderCircle size={14} className="spin-icon" />
          <span>Chrome’s Cast dialog is opening. Choose <b>{pending.name}</b> to finish connecting.</span>
          <button onClick={cancelConnection}>Cancel</button>
        </div>
      )}

      <div className="connect-lg">
        <div className="connect-list-head">
          <span>LG WEBOS CONTROL</span>
          <button
            onClick={() => { void refreshLgDevices(); void refreshLgTv().then(setLgStatus).catch(() => setLgStatus(null)) }}
            disabled={lgLoading || lgBusy}
            aria-label="Refresh LG TV devices"
          >
            <RefreshCw size={13} className={lgLoading ? 'spin-icon' : ''} /> REFRESH LG TV
          </button>
        </div>

        {lgDiscoveryError && !lgDevices.length && !lgStatus?.selected && (
          <div className="connect-error" role="alert"><AlertCircle size={14} /><span>LG TV discovery is unavailable on this network.</span></div>
        )}
        {!lgDevices.length && !lgStatus?.selected && !lgLoading && !lgDiscoveryError && (
          <div className="connect-note">No webOS TVs answered the second-screen discovery request.</div>
        )}
        {lgDevices.map((device) => {
          const selectedHere = lgStatus?.selected?.id === device.id
          const paired = selectedHere ? !!lgStatus?.paired : device.paired
          return (
            <button
              key={device.id}
              className={`connect-device connect-lg-device ${selectedHere ? 'active' : ''}`}
              onClick={() => void selectLg(device)}
              disabled={lgBusy || selectedHere}
            >
              <span className="connect-device-icon"><Tv size={18} /></span>
              <span className="connect-device-copy">
                <strong title={device.name}>{deviceLabel(device)}</strong>
                <small>{device.model} · {device.address} · {device.secure ? 'WSS :3001' : 'WS :3000'}</small>
              </span>
              <span className="connect-device-state">
                {paired ? <Check size={12} /> : null}
                {paired ? 'PAIRED' : 'PAIR REQUIRED'}
              </span>
            </button>
          )
        })}

        {lgStatus?.selected && (
          <div className="connect-lg-controls">
            <div className="connect-lg-summary">
              <div>
                <strong>{lgStatus.selected.name}</strong>
                <span>
                  {lgStatus.paired ? 'PAIRED' : 'PAIRING REQUIRED'}
                  {lgStatus.power_state ? ` · ${lgStatus.power_state.toUpperCase()}` : ''}
                  {lgStatus.volume !== null ? ` · VOL ${Math.round(lgStatus.volume)}` : ''}
                </span>
              </div>
              <button onClick={() => void disconnectLg()} disabled={lgBusy}>CLEAR</button>
            </div>

            {lgStatus.last_error && <div className="connect-error" role="alert"><AlertCircle size={14} /><span>{lgStatus.last_error}</span></div>}

            <div className="connect-lg-actions">
              <button onClick={() => void pairLg()} disabled={lgBusy}>
                {lgBusy ? <LoaderCircle size={13} className="spin-icon" /> : <Power size={13} />}
                {lgStatus.paired ? 'RE-PAIR' : 'PAIR WITH TV'}
              </button>
              <button onClick={() => void refreshLgTv().then(setLgStatus).catch((error) => onNotice(errorMessage(error), 'error'))} disabled={lgBusy || !lgStatus.paired}>
                <RefreshCw size={13} /> REFRESH
              </button>
              <button onClick={() => void sendLg('power_on')} disabled={lgBusy || !lgStatus.paired}><Power size={13} /> ON</button>
              <button onClick={() => void sendLg('power_off')} disabled={lgBusy || !lgStatus.paired}><Power size={13} /> OFF</button>
              <button onClick={() => void sendLg('play')} disabled={lgBusy || !lgStatus.paired}><Play size={13} /> PLAY</button>
              <button onClick={() => void sendLg('pause')} disabled={lgBusy || !lgStatus.paired}><Pause size={13} /> PAUSE</button>
              <button onClick={() => void sendLg('stop')} disabled={lgBusy || !lgStatus.paired}><Square size={13} /> STOP</button>
              <button onClick={() => void sendLg('volume_down')} disabled={lgBusy || !lgStatus.paired}><VolumeX size={13} /> VOL −</button>
              <button onClick={() => void sendLg('volume_up')} disabled={lgBusy || !lgStatus.paired}><Volume2 size={13} /> VOL +</button>
            </div>

            <div className="connect-lg-row">
              <button onClick={() => void loadLgInputs()} disabled={lgBusy || !lgStatus.paired}>
                <ChevronDown size={13} /> INPUTS
              </button>
              {lgInputs.length > 0 ? (
                <>
                  <select value={lgInputId} onChange={(event) => setLgInputId(event.target.value)} aria-label="TV input">
                    {lgInputs.map((input) => (
                      <option key={input.id} value={input.id}>{input.label}{input.connected ? ' · connected' : ''}</option>
                    ))}
                  </select>
                  <button onClick={() => void switchLgInput()} disabled={lgBusy || !lgInputId}>SWITCH</button>
                </>
              ) : <span className="connect-note">Load inputs to switch HDMI or TV source.</span>}
            </div>

            <button className="connect-lg-browser" onClick={() => void openLocalBrowser()} disabled={lgBusy || !lgStatus.paired}>
              OPEN LOCAL URL IN TV BROWSER
            </button>
          </div>
        )}

        <p className="connect-note">
          Video playback uses DLNA/UPnP on smart TVs. Newer webOS firmware does not reliably play
          URLs through its legacy media viewer, so SSAP is used for TV control only.
        </p>
      </div>
    </section>
  )
}
