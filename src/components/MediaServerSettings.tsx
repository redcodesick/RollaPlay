import { useEffect, useState } from 'react'
import useSWR from 'swr'
import { listen } from '@tauri-apps/api/event'
import {
  AlertCircle, Check, Clipboard, Network, Play, RotateCw,
  Server, ShieldCheck, Square, Trash2, Wifi,
} from 'lucide-react'
import { isDesktopApp } from '../media'
import {
  clearMediaStreamDiagnostics, getMediaPairingToken, getMediaServerStatus,
  getMediaStreamDiagnostics, getNetworkDiagnostics, restartMediaServer, setMediaServerPort,
  startMediaServer, stopMediaServer, testMediaServerConnection,
  type ConnectionTestResult, type MediaServerStatus,
} from '../media-server'

function formatBytes(value: number) {
  if (value < 1024) return `${value} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let size = value / 1024
  let unit = 0
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024
    unit += 1
  }
  return `${size.toFixed(size >= 100 ? 0 : 1)} ${units[unit]}`
}

function formatUptime(seconds: number | null) {
  if (seconds === null) return '—'
  const hours = Math.floor(seconds / 3600)
  const minutes = Math.floor((seconds % 3600) / 60)
  const remaining = seconds % 60
  return hours ? `${hours}h ${minutes}m` : minutes ? `${minutes}m ${remaining}s` : `${remaining}s`
}

function statusLabel(status?: MediaServerStatus) {
  if (!status) return 'Loading status'
  switch (status.state) {
    case 'running': return 'Server running'
    case 'starting': return 'Starting'
    case 'network_unavailable': return 'Network unavailable'
    case 'port_conflict': return 'Port conflict'
    case 'error': return 'Error'
    default: return 'Server stopped'
  }
}

export default function MediaServerSettings() {
  const isDesktop = isDesktopApp()
  const [portInput, setPortInput] = useState('8765')
  const [isWorking, setIsWorking] = useState(false)
  const [message, setMessage] = useState('')
  const [errorMessage, setErrorMessage] = useState('')
  const [connectionTest, setConnectionTest] = useState<ConnectionTestResult | null>(null)
  const [pairingToken, setPairingToken] = useState('')
  const [showPairingToken, setShowPairingToken] = useState(false)
  const { data: status, mutate: refreshStatus } = useSWR(isDesktop ? 'media-server-status' : null, getMediaServerStatus, { refreshInterval: 2000 })
  const { data: network } = useSWR(isDesktop ? 'media-server-network' : null, getNetworkDiagnostics, { refreshInterval: 10000 })
  const { data: diagnostics, mutate: refreshDiagnostics } = useSWR(isDesktop ? 'media-server-streams' : null, getMediaStreamDiagnostics, { refreshInterval: 1500 })

  useEffect(() => {
    if (status) setPortInput(String(status.configured_port))
  }, [status?.configured_port])

  useEffect(() => {
    if (status && status.state !== 'running') {
      setPairingToken('')
      setShowPairingToken(false)
    }
  }, [status?.state])

  useEffect(() => {
    if (!isDesktop) return
    let disposed = false
    let unlisten: (() => void) | undefined
    listen('media-server-status-changed', () => {
      void refreshStatus()
      void refreshDiagnostics()
    }).then((stopListening) => {
      if (disposed) stopListening()
      else unlisten = stopListening
    }).catch(() => setErrorMessage('Server status events are unavailable.'))
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [isDesktop, refreshDiagnostics, refreshStatus])

  async function runAction(action: () => Promise<unknown>, successText: string) {
    setIsWorking(true)
    setErrorMessage('')
    setMessage('')
    try {
      await action()
      await Promise.all([refreshStatus(), refreshDiagnostics()])
      setMessage(successText)
    } catch (error) {
      setErrorMessage(typeof error === 'string' ? error : error instanceof Error ? error.message : String(error))
    } finally {
      setIsWorking(false)
    }
  }

  async function savePort() {
    const port = Number(portInput)
    if (!Number.isInteger(port) || port < 1024 || port > 65535) {
      setErrorMessage('Enter a port between 1024 and 65535.')
      return
    }
    await runAction(() => setMediaServerPort(port), 'Streaming port saved.')
  }

  async function copyMediaUrl() {
    if (!status?.media_url_template) return
    try {
      await navigator.clipboard.writeText(status.media_url_template)
      setMessage('LAN media URL pattern copied.')
      setErrorMessage('')
    } catch {
      setErrorMessage('Clipboard access is unavailable. Select and copy the media URL pattern instead.')
    }
  }

  async function revealPairingToken() {
    try {
      const token = await getMediaPairingToken()
      setPairingToken(token)
      setShowPairingToken(true)
      setErrorMessage('')
    } catch (error) {
      setErrorMessage(typeof error === 'string' ? error : error instanceof Error ? error.message : String(error))
    }
  }

  async function copyPairingToken() {
    if (!pairingToken) return
    try {
      await navigator.clipboard.writeText(pairingToken)
      setMessage('Read-only media access token copied. Share it only with a trusted device.')
      setErrorMessage('')
    } catch {
      setErrorMessage('Clipboard access is unavailable. Reveal the token and copy it manually.')
    }
  }

  async function runConnectionTest() {
    setIsWorking(true)
    setErrorMessage('')
    setMessage('')
    try {
      const result = await testMediaServerConnection()
      setConnectionTest(result)
      if (result.reachable) setMessage(result.message)
      else setErrorMessage(result.message)
    } catch (error) {
      setErrorMessage(typeof error === 'string' ? error : error instanceof Error ? error.message : String(error))
    } finally {
      setIsWorking(false)
    }
  }

  const stateClass = isDesktop ? status?.state ?? 'unknown' : 'unknown'
  const currentStatusLabel = isDesktop ? statusLabel(status) : 'Windows app only'
  const currentUrl = status?.media_url_template ?? 'No active LAN endpoint'
  const streams = diagnostics?.active_streams ?? []
  const recentRequests = diagnostics?.recent_requests ?? []

  return (
    <section className="media-server-settings" aria-labelledby="media-server-title">
      <div className="server-section-heading">
        <div>
          <div className="section-eyebrow"><span className="eyebrow-line" /> PHASE 3 · LOCAL STREAMING</div>
          <h2 id="media-server-title">Media server</h2>
          <p>Serve registered original files to devices on your private LAN. No conversion or public listener.</p>
        </div>
        <div className={`server-status-pill ${stateClass}`} role="status"><i />{currentStatusLabel}</div>
      </div>

      {!isDesktop && <div className="library-runtime-note"><ShieldCheck size={16} /><span>Server controls and network discovery are available in the installed Windows app. This preview does not start a local media server.</span></div>}
      {errorMessage && <div className="library-message error" role="alert"><AlertCircle size={16} />{errorMessage}</div>}
      {message && <div className="library-message success" role="status"><Check size={16} />{message}</div>}
      {status?.error && <div className={`library-message ${status.state === 'running' ? 'success' : 'error'}`} role="status"><AlertCircle size={16} />{status.error}</div>}

      <div className="server-info-grid">
        <article className="server-info-card"><span><Network size={15} /> LAN IPv4</span><strong>{status?.lan_ipv4 ?? network?.selected?.ipv4 ?? 'Unavailable'}</strong><small>{status?.interface_display_name ?? network?.selected?.display_name ?? network?.selected?.interface_name ?? 'No active private IPv4 interface'}</small></article>
        <article className="server-info-card"><span><Server size={15} /> LISTENING ADDRESS</span><strong>{status?.listening_address ?? 'Not listening'}</strong><small>Selected adapter only · not bound to all interfaces</small></article>
        <article className="server-info-card"><span><Wifi size={15} /> STREAMING PORT</span><strong>{status?.port ?? status?.configured_port ?? 8765}</strong><small>{status?.port && status.port !== status.configured_port ? `Configured ${status.configured_port}; fallback selected` : 'Default port is 8765'}</small></article>
        <article className="server-info-card"><span><ShieldCheck size={15} /> FIREWALL</span><strong>{status?.firewall_status ?? 'Unknown'}</strong><small>Remote firewall access requires a second-device test.</small></article>
        <article className="server-info-card"><span><Play size={15} /> ACTIVE STREAMS</span><strong>{status?.active_streams ?? 0}</strong><small>Uptime {formatUptime(status?.uptime_seconds ?? null)}</small></article>
      </div>
      <details className="server-network-interfaces">
        <summary>Detected interfaces ({network?.interfaces.length ?? 0})</summary>
        <div className="server-interface-list">{network?.interfaces.map((item) => <div className="server-interface-row" key={`${item.name}-${item.ipv4_addresses.join(',')}`}><strong>{item.display_name ?? item.name}</strong><span>{item.ipv4_addresses.length ? item.ipv4_addresses.map((address, index) => `${address}/${item.ipv4_prefix_lengths[index] ?? ''}`).join(', ') : 'No IPv4 address'}</span><small>{item.interface_type} · {item.is_up && item.is_running ? 'up' : 'down'}{item.is_physical ? ' · physical' : ''}{item.is_default_route ? ' · default route' : ''}</small></div>)}</div>
      </details>

      <div className="server-controls-card">
        <div className="server-controls-heading"><div><h3>Media URL pattern</h3><p>Replace <code>{'{media_id}'}</code> with a registered library ID. Requests require the local pairing token.</p></div><button className="folder-action copy-url-button" onClick={copyMediaUrl} disabled={!isDesktop || !status?.media_url_template}><Clipboard size={15} /> Copy URL pattern</button></div>
        <div className="server-endpoint-value" title={currentUrl}>{currentUrl}</div>
        <div className="server-pairing-panel">
          <div><strong>Trusted device access</strong><small>This token grants read-only access to approved media files, not server settings. Share it only with a trusted device.</small></div>
          <div className="server-pairing-actions">
            <code>{showPairingToken ? pairingToken : 'Access token hidden'}</code>
            <button className="folder-action" onClick={showPairingToken ? () => { setShowPairingToken(false); setPairingToken('') } : revealPairingToken} disabled={!isDesktop || status?.state !== 'running'}>{showPairingToken ? 'Hide token' : 'Show token'}</button>
            <button className="folder-action" onClick={copyPairingToken} disabled={!isDesktop || !pairingToken}><Clipboard size={14} /> Copy token</button>
          </div>
        </div>
        <div className="server-control-row">
          <label htmlFor="server-port">Port</label>
          <input id="server-port" type="number" min={1024} max={65535} step={1} value={portInput} onChange={(event) => setPortInput(event.target.value)} disabled={!isDesktop || isWorking} />
          <button className="folder-action" onClick={savePort} disabled={!isDesktop || isWorking}>Save port</button>
          <span className="server-control-spacer" />
          <button className="folder-action" onClick={() => runAction(startMediaServer, 'Start request sent.')} disabled={!isDesktop || isWorking || status?.state === 'running'}><Play size={14} /> Start</button>
          <button className="folder-action" onClick={() => runAction(stopMediaServer, 'Server stopped.')} disabled={!isDesktop || isWorking || status?.state === 'stopped'}><Square size={13} /> Stop</button>
          <button className="folder-action" onClick={() => runAction(restartMediaServer, 'Server restart requested.')} disabled={!isDesktop || isWorking}><RotateCw size={14} /> Restart</button>
          <button className="button-primary server-test-button" onClick={runConnectionTest} disabled={!isDesktop || isWorking}><Wifi size={15} /> Test connection</button>
        </div>
        {connectionTest && <p className={`connection-test-result ${connectionTest.reachable ? 'passed' : 'failed'}`}>{connectionTest.status_code ? `HTTP ${connectionTest.status_code} · ` : ''}{connectionTest.message}</p>}
      </div>

      <div className="server-diagnostics-card">
        <div className="server-controls-heading"><div><h3>Streaming diagnostics</h3><p>Recent request records contain no access tokens.</p></div><button className="folder-action" onClick={() => runAction(clearMediaStreamDiagnostics, 'Diagnostic history cleared.')} disabled={!isDesktop || isWorking}><Trash2 size={14} /> Clear history</button></div>
        {streams.length ? <div className="stream-diagnostic-list">{streams.map((stream) => <article className="stream-diagnostic-row" key={stream.stream_id}><div><strong>{stream.filename}</strong><span>{stream.client_ip} · ID {stream.media_id}</span></div><span>{formatBytes(stream.bytes_transferred)}</span><span>{stream.duration_seconds}s</span><span className="stream-active-status">{stream.request_status}</span></article>)}</div> : <div className="server-empty-diagnostics">No active streams.</div>}
        {recentRequests.length > 0 && <details className="recent-requests"><summary>Recent requests ({recentRequests.length})</summary><div className="stream-diagnostic-list">{recentRequests.map((request) => <article className="stream-diagnostic-row" key={request.stream_id}><div><strong>{request.filename}</strong><span>{request.client_ip} · ID {request.media_id}</span></div><span>{formatBytes(request.bytes_transferred)}</span><span>{request.duration_seconds}s</span><span>{request.request_status}</span></article>)}</div></details>}
      </div>

      <div className="server-network-note"><ShieldCheck size={15} /><span>Administrative controls are Tauri-only. The HTTP listener binds only to the selected private IPv4 address; LAN media requests use an authenticated bearer token. Firewall acceptance is not inferred by the local probe.</span></div>
    </section>
  )
}
