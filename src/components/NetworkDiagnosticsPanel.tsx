import { useState } from 'react'
import {
  Activity, AlertCircle, LoaderCircle, RefreshCw, ShieldCheck,
} from 'lucide-react'
import { isDesktopApp } from '../media'
import {
  checkStateLabel, checkStateTone, eventCodeLabel, getNetworkCompatibilityDiagnostics,
  linkQualityLabel, transportLabel, type CompatibilityCheck, type NetworkCompatibilityDiagnostics,
} from '../network-diagnostics'

const checks: { key: keyof Omit<NetworkCompatibilityDiagnostics, 'checked_at_epoch_ms' | 'guidance' | 'transport' | 'link_quality' | 'events'>; label: string }[] = [
  { key: 'lan_adapter', label: 'Private LAN adapter' },
  { key: 'local_server', label: 'Local media server' },
  { key: 'receiver_discovery', label: 'Receiver discovery' },
  { key: 'receiver_network', label: 'Receiver subnet' },
  { key: 'receiver_match', label: 'Active receiver match' },
  { key: 'receiver_control', label: 'Receiver control port' },
  { key: 'media_url', label: 'Media URL probe' },
  { key: 'receiver_media', label: 'Receiver media requests' },
  { key: 'stream_throughput', label: 'Stream throughput tier' },
  { key: 'stream_stalls', label: 'Stream interruption tier' },
  { key: 'playback', label: 'Receiver playback state' },
  { key: 'queue', label: 'Queue continuation' },
  { key: 'media_format', label: 'Current media container' },
]

function formatCheckedAt(epochMs: number) {
  if (!epochMs) return 'Never'
  return new Date(epochMs).toLocaleTimeString()
}

function formatEventTime(epochMs: number) {
  if (!epochMs) return '--:--:--'
  return new Date(epochMs).toLocaleTimeString()
}

function CheckRow({ label, check }: { label: string; check: CompatibilityCheck }) {
  return (
    <article className="diagnostics-check-row">
      <div><strong>{label}</strong><span>{check.detail}</span></div>
      <span className={`diagnostics-state ${checkStateTone(check.state)}`}>{checkStateLabel(check.state)}</span>
    </article>
  )
}

export default function NetworkDiagnosticsPanel() {
  const isDesktop = isDesktopApp()
  const [report, setReport] = useState<NetworkCompatibilityDiagnostics | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  async function runChecks() {
    setBusy(true)
    setError('')
    try {
      setReport(await getNetworkCompatibilityDiagnostics())
    } catch {
      setError('The network checks could not run. Try again after confirming the app still has LAN access.')
    } finally {
      setBusy(false)
    }
  }

  return (
    <section className="diagnostics-panel" id="settings-network-diagnostics" aria-labelledby="diagnostics-panel-title">
      <div className="diagnostics-heading">
        <div>
          <div className="section-eyebrow"><span className="eyebrow-line" /> CONNECTION HEALTH</div>
          <h2 id="diagnostics-panel-title">Diagnostics</h2>
          <p>Checks the real local path a receiver needs to stream video, plus link-quality tiers and a short redacted event log. Run them while the TV is on and connected.</p>
        </div>
        <button className="button-primary" onClick={() => void runChecks()} disabled={!isDesktop || busy}>
          {busy ? <LoaderCircle size={15} className="spin-icon" /> : <RefreshCw size={15} />}
          {busy ? 'Checking…' : 'Run checks'}
        </button>
      </div>

      {!isDesktop && <div className="library-runtime-note"><ShieldCheck size={16} /><span>Network diagnostics require the installed Windows desktop app.</span></div>}
      {error && <div className="library-message error" role="alert"><AlertCircle size={16} />{error}</div>}
      {!report && !error && <div className="diagnostics-empty"><Activity size={18} /><span>Run checks to compare this PC, the discovered receiver, and the local media server. Nothing is sent off this device.</span><small>Last check: {formatCheckedAt(0)}</small></div>}

      {report && <>
        <div className="diagnostics-summary">
          <span><Activity size={15} /> Last check</span>
          <strong>{formatCheckedAt(report.checked_at_epoch_ms)}</strong>
          <span><Activity size={15} /> Playback path</span>
          <strong>{transportLabel(report.transport)}</strong>
        </div>
        <div className="diagnostics-link-grid">
          <div><span>Adapter</span><strong>{linkQualityLabel(report.link_quality.adapter_kind)}</strong></div>
          <div><span>Band</span><strong>{linkQualityLabel(report.link_quality.band)}</strong></div>
          <div><span>Channel</span><strong>{report.link_quality.channel ?? '—'}</strong></div>
          <div><span>Channel width</span><strong>{linkQualityLabel(report.link_quality.channel_width)}</strong></div>
          <div><span>Signal tier</span><strong>{linkQualityLabel(report.link_quality.signal)}</strong></div>
          <div><span>Link speed tier</span><strong>{linkQualityLabel(report.link_quality.link_speed)}</strong></div>
        </div>
        <div className="diagnostics-check-list">
          {checks.map(({ key, label }) => {
            const check = report[key]
            return <CheckRow key={key} label={label} check={check} />
          })}
        </div>
        {report.events.length > 0 && <div className="diagnostics-events">
          <strong>Recent events</strong>
          <ul>
            {report.events.slice(-8).reverse().map((event, index) => (
              <li key={`${event.at_epoch_ms}-${event.code}-${index}`}>
                <time>{formatEventTime(event.at_epoch_ms)}</time>
                <span>{eventCodeLabel(event.code)}</span>
                <em>{event.detail}</em>
              </li>
            ))}
          </ul>
        </div>}
        {report.guidance.length > 0 && <div className="diagnostics-guidance">
          <strong>What to try</strong>
          <ul>{report.guidance.map((line) => <li key={line}>{line}</li>)}</ul>
        </div>}
      </>}

      <div className="diagnostics-privacy"><ShieldCheck size={15} /><span>These checks show fixed status text, allow-listed tiers, and in-memory event codes only. Tokens, media URLs, file paths, pairing keys, SSIDs, device addresses, and raw output are never displayed or stored here.</span></div>
    </section>
  )
}
