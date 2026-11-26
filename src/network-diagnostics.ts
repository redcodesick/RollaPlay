import { invoke } from '@tauri-apps/api/core'

export type CheckState = 'ok' | 'warning' | 'failed' | 'unknown' | 'not_run'

export type CompatibilityCheck = {
  state: CheckState
  detail: string
}

export type DiagnosticEvent = {
  code: string
  detail: string
  at_epoch_ms: number
}

export type LinkQuality = {
  adapter_kind: string
  band: string
  channel: number | null
  channel_width: string
  signal: string
  link_speed: string
}

export type NetworkCompatibilityDiagnostics = {
  checked_at_epoch_ms: number
  transport: string
  lan_adapter: CompatibilityCheck
  local_server: CompatibilityCheck
  receiver_discovery: CompatibilityCheck
  receiver_network: CompatibilityCheck
  receiver_match: CompatibilityCheck
  receiver_control: CompatibilityCheck
  media_url: CompatibilityCheck
  receiver_media: CompatibilityCheck
  link_quality: LinkQuality
  stream_throughput: CompatibilityCheck
  stream_stalls: CompatibilityCheck
  playback: CompatibilityCheck
  queue: CompatibilityCheck
  media_format: CompatibilityCheck
  guidance: string[]
  events: DiagnosticEvent[]
}

/** Returns only allow-listed states and fixed guidance from the Rust backend. */
export const getNetworkCompatibilityDiagnostics = () =>
  invoke<NetworkCompatibilityDiagnostics>('get_network_compatibility_diagnostics')

export function transportLabel(transport: string | undefined): string {
  switch (transport) {
    case 'dlna': return 'DLNA / UPnP'
    case 'cast': return 'Google Cast'
    default: return 'Not selected'
  }
}

export function checkStateLabel(state: CheckState | string | undefined): string {
  switch (state) {
    case 'ok': return 'OK'
    case 'warning': return 'CHECK'
    case 'failed': return 'FAILED'
    case 'not_run': return 'NOT RUN'
    default: return 'UNKNOWN'
  }
}

export function checkStateTone(state: CheckState | string | undefined): string {
  switch (state) {
    case 'ok': return 'ok'
    case 'failed': return 'bad'
    case 'warning': return 'warn'
    default: return 'idle'
  }
}

/** Maps backend tiers to fixed UI text; no raw adapter strings are rendered. */
export function linkQualityLabel(value: string | undefined): string {
  switch (value) {
    case 'wireless': return 'Wi-Fi'
    case 'wired': return 'Ethernet'
    case '2_4_ghz': return '2.4 GHz'
    case '5_or_6_ghz': return '5 / 6 GHz'
    case 'excellent': return 'Excellent'
    case 'good': return 'Good'
    case 'fair': return 'Fair'
    case 'weak': return 'Weak'
    case 'high': return 'High'
    case 'moderate': return 'Moderate'
    case 'low': return 'Low'
    case 'very_low': return 'Very low'
    case 'unavailable': return 'Not exposed by Windows'
    case 'not_applicable': return 'Not applicable'
    default: return 'Unknown'
  }
}

export function eventCodeLabel(code: string | undefined): string {
  switch (code) {
    case 'LAN_ADAPTER_MISSING': return 'Private LAN adapter missing'
    case 'WEAK_SIGNAL': return 'Weak PC Wi-Fi signal'
    case 'LOW_2_4_GHZ_HEADROOM': return 'Low 2.4 GHz headroom'
    case 'CONTROL_OK_MEDIA_STALLED': return 'Control answered, media stalled'
    case 'MEDIA_FETCH_STALLS': return 'Repeated media fetch stalls'
    case 'PLAYBACK_IDLE_WITH_MEDIA': return 'Idle state while bytes flow'
    case 'RECEIVER_AMBIGUOUS': return 'Receiver correlation uncertain'
    default: return 'Logged check event'
  }
}
