import { useCallback, useState } from 'react'
import useSWR from 'swr'
import {
  AlertCircle, Cast, Check, LoaderCircle, Monitor, RefreshCw, ShieldCheck, Wifi,
} from 'lucide-react'
import { isDesktopApp } from '../media'
import {
  castDeviceName, castStateLabel, getCastPlaybackState, getCastSenderStatus,
  launchCastSender, playerStateLabel, sessionStateLabel,
} from '../cast'
import {
  getMediaServerStatus, getNetworkDiagnostics, testMediaServerConnection,
  type ConnectionTestResult,
} from '../media-server'
import { debugLog, diagnosticError, friendlyError } from '../diagnostics'

type Notice = (message: string, level?: 'info' | 'success' | 'warning' | 'error') => void

function CheckRow({ label, value }: { label: string; value: string | null | undefined }) {
  return (
    <div className="setup-check-row">
      <span>{label}</span>
      <strong title={value ?? ''}>{value || 'Unavailable'}</strong>
    </div>
  )
}

function DesktopOnlyNote({ children }: { children: string }) {
  return <div className="setup-note"><ShieldCheck size={17} /><span>{children}</span></div>
}

export function ChromeCheck({ onNotice }: { onNotice: Notice }) {
  const isDesktop = isDesktopApp()
  const [busy, setBusy] = useState(false)
  const { data, error, isLoading, mutate } = useSWR(isDesktop ? 'cast-sender-status' : null, getCastSenderStatus, { refreshInterval: 5000 })

  const launch = useCallback(async () => {
    debugLog('wizard', 'button_click', { control: 'launch_cast_sender' })
    setBusy(true)
    try {
      const status = await launchCastSender()
      await mutate(status, { revalidate: false })
      onNotice(status.chrome_installed ? 'Chrome sender window launch requested.' : 'Google Chrome was not found on this device.', status.chrome_installed ? 'success' : 'warning')
    } catch (launchError) {
      diagnosticError('wizard', 'launch_cast_sender_failed', launchError)
      onNotice(friendlyError(launchError), 'error')
    } finally {
      setBusy(false)
    }
  }, [mutate, onNotice])

  if (!isDesktop) return <DesktopOnlyNote>Chrome detection and sender launch require the installed Windows desktop app.</DesktopOnlyNote>

  return (
    <div className="setup-checks">
      {error && <div className="library-message error" role="alert"><AlertCircle size={16} />{friendlyError(error)}</div>}
      {isLoading && <div className="library-loading"><LoaderCircle size={17} className="spin-icon" /> Checking Google Chrome…</div>}
      {data && <>
        <CheckRow label="Google Chrome" value={data.chrome_installed ? 'Detected' : 'Not found'} />
        <CheckRow label="Chrome path" value={data.chrome_path} />
        <CheckRow label="Sender bridge" value={data.bridge_state} />
        <CheckRow label="Sender window" value={data.sender_window_state} />
        {data.last_error && <div className="library-message error" role="alert"><AlertCircle size={16} />{data.last_error}</div>}
      </>}
      <div className="setup-check-actions">
        <button className="folder-action" onClick={() => { debugLog('wizard', 'button_click', { control: 'refresh_chrome_status' }); void mutate() }} disabled={isLoading || busy}><RefreshCw size={14} /> Refresh</button>
        <button className="folder-action" onClick={() => void launch()} disabled={busy || !data?.chrome_installed}><Cast size={14} /> {busy ? 'Starting…' : 'Start sender window'}</button>
      </div>
    </div>
  )
}

export function NetworkCheck({ onNotice }: { onNotice: Notice }) {
  const isDesktop = isDesktopApp()
  const [busy, setBusy] = useState(false)
  const [testResult, setTestResult] = useState<ConnectionTestResult | null>(null)
  const { data: status, error: statusError, isLoading: statusLoading, mutate: refreshStatus } = useSWR(isDesktop ? 'media-server-status' : null, getMediaServerStatus, { refreshInterval: 2000 })
  const { data: network, error: networkError } = useSWR(isDesktop ? 'media-server-network' : null, getNetworkDiagnostics, { refreshInterval: 10000 })

  const testConnection = useCallback(async () => {
    debugLog('wizard', 'button_click', { control: 'test_media_server_connection' })
    setBusy(true)
    try {
      const result = await testMediaServerConnection()
      setTestResult(result)
      onNotice(result.message, result.reachable ? 'success' : 'warning')
    } catch (testError) {
      diagnosticError('wizard', 'connection_test_failed', testError)
      onNotice(friendlyError(testError), 'error')
    } finally {
      setBusy(false)
    }
  }, [onNotice])

  if (!isDesktop) return <DesktopOnlyNote>LAN interface detection, server binding and connection tests require the installed Windows desktop app.</DesktopOnlyNote>

  return (
    <div className="setup-checks">
      {(statusError || networkError) && <div className="library-message error" role="alert"><AlertCircle size={16} />{friendlyError(statusError ?? networkError)}</div>}
      {statusLoading && <div className="library-loading"><LoaderCircle size={17} className="spin-icon" /> Reading network status…</div>}
      <CheckRow label="Server state" value={status?.state ?? null} />
      <CheckRow label="Private LAN IPv4" value={status?.lan_ipv4 ?? network?.selected?.ipv4 ?? null} />
      <CheckRow label="Interface" value={status?.interface_display_name ?? network?.selected?.display_name ?? network?.selected?.interface_name ?? null} />
      <CheckRow label="Listening address" value={status?.listening_address ?? null} />
      <CheckRow label="Streaming port" value={String(status?.port ?? status?.configured_port ?? 8765)} />
      <CheckRow label="Firewall" value={status?.firewall_status ?? null} />
      {status?.error && <div className="library-message error" role="alert"><AlertCircle size={16} />{status.error}</div>}
      {testResult && <div className={`library-message ${testResult.reachable ? 'success' : 'error'}`} role="status">{testResult.reachable ? <Check size={16} /> : <AlertCircle size={16} />}{testResult.status_code ? `HTTP ${testResult.status_code} · ` : ''}{testResult.message}</div>}
      <div className="setup-note"><Wifi size={17} /><span>This probe runs on this PC. A Xiaomi Mi Box on the same LAN must still be tested separately.</span></div>
      <div className="setup-check-actions">
        <button className="folder-action" onClick={() => { debugLog('wizard', 'button_click', { control: 'refresh_network_status' }); void refreshStatus() }} disabled={statusLoading || busy}><RefreshCw size={14} /> Refresh</button>
        <button className="folder-action" onClick={() => void testConnection()} disabled={busy}><Wifi size={14} /> {busy ? 'Testing…' : 'Test connection'}</button>
      </div>
    </div>
  )
}

export function CastCheck({ onNotice }: { onNotice: Notice }) {
  const isDesktop = isDesktopApp()
  const [busy, setBusy] = useState(false)
  const { data: sender, error: senderError, isLoading: senderLoading, mutate: refreshSender } = useSWR(isDesktop ? 'cast-sender-status' : null, getCastSenderStatus, { refreshInterval: 5000 })
  const { data: playback, error: playbackError, isLoading: playbackLoading, mutate: refreshPlayback } = useSWR(isDesktop ? 'cast-playback-state' : null, getCastPlaybackState, { refreshInterval: 2000 })

  const launch = useCallback(async () => {
    debugLog('wizard', 'button_click', { control: 'start_cast_discovery' })
    setBusy(true)
    try {
      const status = await launchCastSender()
      await refreshSender(status, { revalidate: false })
      const latest = await refreshPlayback()
      const device = castDeviceName(latest)
      onNotice(
        device
          ? `Cast sender ready. Casting to ${device}; no need to choose a device again.`
          : 'Cast sender ready. Choose your TV in the Chrome sender window.',
        device ? 'success' : 'info',
      )
    } catch (launchError) {
      diagnosticError('wizard', 'cast_discovery_failed', launchError)
      onNotice(friendlyError(launchError), 'error')
    } finally {
      setBusy(false)
    }
  }, [onNotice, refreshPlayback, refreshSender])

  if (!isDesktop) return <DesktopOnlyNote>Cast device discovery uses the official Google Cast SDK in the Windows desktop app.</DesktopOnlyNote>

  const connectedDevice = castDeviceName(playback)

  return (
    <div className="setup-checks">
      {(senderError || playbackError) && <div className="library-message error" role="alert"><AlertCircle size={16} />{friendlyError(senderError ?? playbackError)}</div>}
      {(senderLoading || playbackLoading) && <div className="library-loading"><LoaderCircle size={17} className="spin-icon" /> Reading Cast status…</div>}
      <CheckRow label="Chrome" value={sender?.chrome_installed ? 'Detected' : 'Not found'} />
      <CheckRow label="Sender window" value={sender?.sender_window_state ?? null} />
      <CheckRow label="Discovery" value={castStateLabel(playback?.cast_state ?? null)} />
      <CheckRow label="Session" value={sessionStateLabel(playback?.session_state ?? null)} />
      <CheckRow label="Connected device" value={connectedDevice} />
      <CheckRow label="Receiver playback" value={playerStateLabel(playback?.player_state ?? null)} />
      {playback?.last_error && <div className="library-message error" role="alert"><AlertCircle size={16} />{playback.last_error}</div>}
      <div className="setup-note"><Monitor size={17} /><span>{connectedDevice ? `Connected to ${connectedDevice}. The sender stays selected until you disconnect.` : 'No device appears until Chrome discovers a Cast receiver on the same LAN.'}</span></div>
      <div className="setup-check-actions">
        <button className="folder-action" onClick={() => { debugLog('wizard', 'button_click', { control: 'refresh_cast_status' }); void Promise.all([refreshSender(), refreshPlayback()]) }} disabled={senderLoading || playbackLoading || busy}><RefreshCw size={14} /> Refresh</button>
        <button className="folder-action" onClick={() => void launch()} disabled={busy || !sender?.chrome_installed}><Cast size={14} /> {busy ? 'Starting…' : connectedDevice ? 'Open sender' : 'Start discovery'}</button>
      </div>
    </div>
  )
}
