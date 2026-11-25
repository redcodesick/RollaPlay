import { listen } from '@tauri-apps/api/event'
import { invoke, isTauri } from '@tauri-apps/api/core'
import type { ConnectionTestResult } from './media-server'

export type CastDevice = {
  id: string
  name: string
  model: string
  address: string
  port: number
  source: string
  /** `cast` for Google Cast, `dlna` for UPnP AV renderers. */
  transport?: string
  avtransport_url?: string | null
  rendering_url?: string | null
  description_url?: string | null
}

export type DlnaStatus = {
  selected: CastDevice | null
  snapshot: CastPlaybackSnapshot
}

export type LgTvDevice = {
  id: string
  name: string
  model: string
  address: string
  port: number
  secure: boolean
  paired: boolean
}

export type LgTvStatus = {
  selected: LgTvDevice | null
  paired: boolean
  power_state: string | null
  volume: number | null
  muted: boolean | null
  last_error: string | null
}

export type LgTvPairResult = {
  status: 'paired' | 'prompt' | string
  device: LgTvDevice
  message: string
}

export type LgTvInput = {
  id: string
  label: string
  connected: boolean
}

export type CastSenderStatus = {
  chrome_installed: boolean
  chrome_path: string | null
  chrome_source: string | null
  bridge_state: 'stopped' | 'running' | 'starting' | 'error' | string
  bridge_port: number | null
  sender_page_connected: boolean
  sender_page_last_seen_ms_ago: number | null
  sender_window_state: 'not_launched' | 'launching' | 'running' | 'closed' | 'failed' | string
  sender_pid: number | null
  relaunch_attempts: number
  last_error: string | null
  last_launch_attempt_epoch_ms: number | null
}

export type CastPlaybackSnapshot = {
  media_file_id: number | null
  filename: string | null
  player_state: 'PLAYING' | 'PAUSED' | 'BUFFERING' | 'IDLE' | string | null
  position_seconds: number | null
  duration_seconds: number | null
  volume_level: number | null
  muted: boolean | null
  device_name: string | null
  cast_state: 'NO_DEVICES_AVAILABLE' | 'NOT_CONNECTED' | 'CONNECTING' | 'CONNECTED' | string | null
  session_state: string | null
  last_error: string | null
  updated_at_epoch_ms: number
}

export type CastDiagnostics = {
  sender: CastSenderStatus
  discovery: CastPlaybackSnapshot
  media_url_probe: ConnectionTestResult | null
  active_cast_grants: number
}

export type CastLoadOutcome = {
  media_file_id: number
  filename: string
  mime_type: string
  expires_in_seconds: number
}

export type CastEventReport = {
  kind: 'sender_ready' | 'cast_state' | 'session_started' | 'session_resumed' | 'session_ended' | 'session_start_failed'
    | 'media_loaded' | 'media_status' | 'load_failed' | 'sender_error' | string
  device_name: string | null
  cast_state: string | null
  session_state: string | null
  player_state: string | null
  position_seconds: number | null
  duration_seconds: number | null
  message: string | null
  playback_token?: string | null
  cast_session_id?: string | null
  media_session_id?: string | null
  idle_reason?: string | null
}

export type CastControlAction =
  | 'play'
  | 'pause'
  | 'toggle_play'
  | 'stop'
  | 'seek'
  | 'set_volume'
  | 'set_mute'
  | 'disconnect'
  | 'request_session'
  | 'close_sender'

export type CastControlOptions = {
  /** Tauri v2 expects camelCase argument names for Rust snake_case parameters. */
  positionSeconds?: number
  volume?: number
  muted?: boolean
  deviceId?: string
  deviceName?: string
}

/** Containers the Cast Default Media Receiver accepts. */
export const castSupportedExtensions = ['mp4', 'm4v', 'webm', 'mkv']

export const isCastSupportedExtension = (extension: string) =>
  castSupportedExtensions.includes(extension.replace(/^\.+/, '').toLowerCase())

export const isCastAvailable = () => isTauri()

export type SenderHostRect = { x: number; y: number; width: number; height: number }

/** Real LAN receivers found through mDNS (Cast) and SSDP (DLNA renderers). */
export const discoverCastDevices = () => invoke<CastDevice[]>('discover_cast_devices')
export const getCastSenderStatus = () => invoke<CastSenderStatus>('get_cast_sender_status')
export const launchCastSender = () => invoke<CastSenderStatus>('launch_cast_sender')
/** Embeds the managed Chrome sender window into the in-app sender panel. */
export const embedCastSender = (rect: SenderHostRect) => invoke<boolean>('embed_cast_sender', { rect })
export const showCastSender = (rect: SenderHostRect) => invoke<boolean>('show_cast_sender', { rect })
export const moveCastSender = (rect: SenderHostRect) => invoke<void>('move_cast_sender', { rect })
export const hideCastSender = () => invoke<void>('hide_cast_sender')
/** Supplies the Chrome user gesture required by the official Cast picker. */
export const clickCastSender = () => invoke<void>('click_cast_sender')
export const stopCastSender = () => invoke<CastSenderStatus>('stop_cast_sender')
export const getCastDiagnostics = () => invoke<CastDiagnostics>('get_cast_diagnostics')
export const getCastPlaybackState = () => invoke<CastPlaybackSnapshot>('get_cast_playback_state')
export const castLoadMedia = (mediaFileId: number) => invoke<CastLoadOutcome>('cast_load_media', { mediaFileId })
export const castSendControl = (action: CastControlAction, options: CastControlOptions = {}) =>
  invoke<void>('cast_send_control', { action, ...options })
/** Opens the official Web Sender device dialog for the selected receiver. */
export const castRequestSession = (device: CastDevice) =>
  castSendControl('request_session', { deviceId: device.id, deviceName: device.name })
export const verifyCastMediaUrl = (mediaFileId: number) =>
  invoke<ConnectionTestResult>('verify_cast_media_url', { mediaFileId })

/** DLNA renderers are discovered by SSDP and selected for real AVTransport playback. */
export const selectDlnaDevice = (device: CastDevice) =>
  invoke<DlnaStatus>('select_dlna_device', { device })
export const disconnectDlna = () => invoke<void>('disconnect_dlna')
export const getDlnaStatus = () => invoke<DlnaStatus>('get_dlna_status')

/** LG webOS control is local SSAP only; playback still uses DLNA/UPnP. */
export const discoverLgTvDevices = () => invoke<LgTvDevice[]>('discover_lg_tv_devices')
export const selectLgTvDevice = (device: LgTvDevice) =>
  invoke<LgTvStatus>('select_lg_tv_device', { device })
export const getLgTvStatus = () => invoke<LgTvStatus>('get_lg_tv_status')
export const pairLgTv = () => invoke<LgTvPairResult>('pair_lg_tv')
export const refreshLgTv = () => invoke<LgTvStatus>('refresh_lg_tv')
export const lgTvControl = (action: string, value?: number) =>
  invoke<LgTvStatus>('lg_tv_control', { action, value })
export const getLgTvInputs = () => invoke<LgTvInput[]>('get_lg_tv_inputs')
export const lgTvSwitchInput = (inputId: string) =>
  invoke<LgTvStatus>('lg_tv_switch_input', { inputId })
export const lgTvOpenBrowser = (target: string) =>
  invoke<void>('lg_tv_open_browser', { target })
export const disconnectLgTv = () => invoke<void>('disconnect_lg_tv')

export const onCastPlaybackEvent = (handler: (event: CastEventReport) => void) =>
  listen<CastEventReport>('cast-playback-event', (event) => handler(event.payload))

export const onCastSenderStatusChanged = (handler: () => void) =>
  listen('cast-sender-status-changed', () => handler())

const activeSessionStates = new Set([
  'SESSION_STARTED', 'SESSION_RESUMED',
  'SESSION_STATE_STARTED', 'SESSION_STATE_RESUMED',
])

/**
 * The sender normally reports the friendly receiver name with session events.
 * If those events arrive out of order (or from a second sender window), the
 * player can still be casting while `device_name` is briefly null. Treat an
 * active session/cast state or actual receiver playback as connected so the UI
 * never claims "No cast device" while video is playing on the TV.
 */
export function castDeviceName(snapshot: CastPlaybackSnapshot | null | undefined): string | null {
  if (!snapshot) return null
  if (snapshot.device_name) return snapshot.device_name
  const activelyCasting =
    snapshot.cast_state === 'CONNECTED' ||
    activeSessionStates.has(snapshot.session_state ?? '') ||
    snapshot.player_state === 'PLAYING'
  return activelyCasting ? 'Cast device' : null
}

export function castStateLabel(state: string | null | undefined) {
  switch (state) {
    case 'NO_DEVICES_AVAILABLE': return 'No Cast devices found'
    case 'NOT_CONNECTED': return 'Devices available · not connected'
    case 'CONNECTING': return 'Connecting…'
    case 'CONNECTED': return 'Connected'
    default: return 'Discovery idle'
  }
}

export function playerStateLabel(state: string | null | undefined) {
  switch (state) {
    case 'PLAYING': return 'Playing'
    case 'PAUSED': return 'Paused'
    case 'BUFFERING': return 'Buffering'
    case 'IDLE': return 'Idle'
    default: return 'Not playing'
  }
}

export function sessionStateLabel(state: string | null | undefined) {
  switch (state) {
    case 'SESSION_STARTED':
    case 'SESSION_STATE_STARTED': return 'Session active'
    case 'SESSION_RESUMED':
    case 'SESSION_STATE_RESUMED': return 'Session restored'
    case 'SESSION_STARTING':
    case 'SESSION_STATE_STARTING': return 'Starting session…'
    case 'SESSION_ENDING':
    case 'SESSION_STATE_ENDING': return 'Ending session…'
    case 'SESSION_ENDED':
    case 'SESSION_STATE_ENDED': return 'Session ended'
    case 'SESSION_STOPPED':
    case 'SESSION_STATE_STOPPED': return 'Session stopped'
    case 'SESSION_START_FAILED': return 'Session failed to start'
    case 'NO_SESSION': return 'No session'
    default: return 'No session'
  }
}

export function formatSeconds(value: number | null | undefined) {
  if (value === null || value === undefined || !Number.isFinite(value) || value < 0) return '--:--'
  const total = Math.floor(value)
  const hours = Math.floor(total / 3600)
  const minutes = Math.floor((total % 3600) / 60)
  const seconds = total % 60
  const pad = (n: number) => String(n).padStart(2, '0')
  return hours > 0 ? `${hours}:${pad(minutes)}:${pad(seconds)}` : `${minutes}:${pad(seconds)}`
}
