import { invoke } from '@tauri-apps/api/core'

export type MediaServerState = 'starting' | 'running' | 'stopped' | 'network_unavailable' | 'port_conflict' | 'error'

export type MediaServerStatus = {
  state: MediaServerState | string
  lan_ipv4: string | null
  interface_name: string | null
  interface_display_name: string | null
  listening_address: string | null
  configured_port: number
  port: number | null
  media_url_template: string | null
  network_status: string
  firewall_status: string
  active_streams: number
  uptime_seconds: number | null
  error: string | null
}

export type NetworkInterfaceInfo = {
  name: string
  display_name: string | null
  description: string | null
  interface_type: string
  ipv4_addresses: string[]
  ipv4_prefix_lengths: number[]
  is_up: boolean
  is_loopback: boolean
  is_tun: boolean
  is_running: boolean
  is_physical: boolean
  is_default_route: boolean
}

export type NetworkDiagnostics = {
  selected: {
    ipv4: string
    interface_name: string
    display_name: string | null
    is_default_route: boolean
    prefix_length: number
  } | null
  interfaces: NetworkInterfaceInfo[]
}

export type ConnectionTestResult = {
  reachable: boolean
  status_code: number | null
  endpoint: string | null
  message: string
}

export type StreamDiagnostic = {
  stream_id: string
  media_id: number
  filename: string
  client_ip: string
  bytes_transferred: number
  duration_seconds: number
  request_status: string
}

export type StreamDiagnostics = {
  active_streams: StreamDiagnostic[]
  recent_requests: StreamDiagnostic[]
}

export const getMediaServerStatus = () => invoke<MediaServerStatus>('get_media_server_status')
export const getMediaPairingToken = () => invoke<string>('get_media_pairing_token')
export const startMediaServer = () => invoke<MediaServerStatus>('start_media_server')
export const stopMediaServer = () => invoke<MediaServerStatus>('stop_media_server')
export const restartMediaServer = () => invoke<MediaServerStatus>('restart_media_server')
export const setMediaServerPort = (port: number) => invoke<MediaServerStatus>('set_media_server_port', { port })
export const getNetworkDiagnostics = () => invoke<NetworkDiagnostics>('get_media_network_diagnostics')
export const testMediaServerConnection = () => invoke<ConnectionTestResult>('test_media_server_connection')
export const getMediaStreamDiagnostics = () => invoke<StreamDiagnostics>('get_media_stream_diagnostics')
export const clearMediaStreamDiagnostics = () => invoke('clear_media_stream_diagnostics')
