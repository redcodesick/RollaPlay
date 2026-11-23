import { invoke, isTauri } from '@tauri-apps/api/core'

export type PlaybackMode = 'manual' | 'auto'
export type RepeatMode = 'off' | 'one' | 'all'
export type EngineState =
  | 'idle' | 'connecting' | 'loading' | 'playing' | 'paused' | 'buffering'
  | 'completed' | 'waiting_manual' | 'recovering' | 'disconnected' | 'error'
export type EntryStatus = 'pending' | 'playing' | 'completed' | 'unavailable'
export type QueueAddMode = 'append' | 'play_next' | 'play_now'

export type QueueEntry = {
  entry_id: string
  media_file_id: number
  filename: string
  availability_state: string
  duration_seconds: number | null
  status: EntryStatus
  position: number
}

export type PlaylistSummary = {
  id: number
  name: string
  created_at: string
  updated_at: string
  item_count: number
  duration_seconds: number | null
}

export type PlaylistItem = {
  id: number
  media_file_id: number
  filename: string
  availability_state: string
  duration_seconds: number | null
  position: number
}

export type HistoryEntry = {
  id: number
  entry_id: string | null
  media_file_id: number
  filename: string
  started_at: string
  watched_seconds: number
  completion_state: string | null
  position_seconds: number
}

export type PlaybackSnapshot = {
  state: EngineState
  mode: PlaybackMode
  shuffle: boolean
  repeat: RepeatMode
  queue: QueueEntry[]
  current_entry_id: string | null
  next_entry_id: string | null
  history: HistoryEntry[]
  current_media_file_id: number | null
  position_seconds: number
  duration_seconds: number | null
  playback_token: string | null
  cast_session_id: string | null
  media_session_id: number | null
  last_completion_token: string | null
  last_selected_playlist_id: number | null
  previous_restart_threshold_seconds: number
  message: string | null
  last_error: string | null
}

export type Phase5Notification = { level: 'info' | 'success' | 'warning' | 'error'; message: string }

export function isPlaylistAvailable(): boolean {
  return isTauri()
}

const call = <T>(command: string, args?: Record<string, unknown>): Promise<T> =>
  invoke<T>(command, args)

export const getPlaybackState = () => call<PlaybackSnapshot>('phase5_get_state')

export const listPlaylists = () => call<PlaylistSummary[]>('phase5_list_playlists')
export const createPlaylist = (name: string) => call<number>('phase5_create_playlist', { name })
export const renamePlaylist = (playlistId: number, name: string) =>
  call<void>('phase5_rename_playlist', { playlistId, name })
export const deletePlaylist = (playlistId: number) =>
  call<void>('phase5_delete_playlist', { playlistId })
export const selectPlaylist = (playlistId: number) =>
  call<void>('phase5_select_playlist', { playlistId })
export const listPlaylistItems = (playlistId: number) =>
  call<PlaylistItem[]>('phase5_list_playlist_items', { playlistId })
export const addPlaylistItems = (playlistId: number, mediaFileIds: number[]) =>
  call<number>('phase5_add_playlist_items', { playlistId, mediaFileIds })
export const removePlaylistItem = (itemId: number) =>
  call<void>('phase5_remove_playlist_item', { itemId })
export const reorderPlaylistItem = (itemId: number, targetPosition: number) =>
  call<void>('phase5_reorder_playlist_item', { itemId, targetPosition })

export const queueAdd = (mediaFileIds: number[], mode: QueueAddMode) =>
  call<PlaybackSnapshot>('phase5_queue_add', { mediaFileIds, mode })
export const queueAddPlaylist = (playlistId: number, mode: QueueAddMode) =>
  call<PlaybackSnapshot>('phase5_queue_add_playlist', { playlistId, mode })
export const queueRemove = (entryId: string) =>
  call<PlaybackSnapshot>('phase5_queue_remove', { entryId })
export const queueReorder = (entryId: string, targetPosition: number) =>
  call<PlaybackSnapshot>('phase5_queue_reorder', { entryId, targetPosition })
export const queueClear = () => call<PlaybackSnapshot>('phase5_queue_clear')

export const setPlaybackMode = (mode: PlaybackMode) =>
  call<PlaybackSnapshot>('phase5_set_mode', { mode })
export const setShuffle = (enabled: boolean) =>
  call<PlaybackSnapshot>('phase5_set_shuffle', { enabled })
export const setRepeat = (mode: RepeatMode) =>
  call<PlaybackSnapshot>('phase5_set_repeat', { mode })
export const playNow = (entryId: string) => call<PlaybackSnapshot>('phase5_play_now', { entryId })
export const playMediaNow = (mediaFileId: number) =>
  call<PlaybackSnapshot>('phase5_play_media_now', { mediaFileId })
export const playNext = () => call<PlaybackSnapshot>('phase5_next')
export const playPrevious = () => call<PlaybackSnapshot>('phase5_previous')
export const recoverPlayback = () => call<PlaybackSnapshot>('phase5_recover')

export function formatDuration(seconds: number | null | undefined): string {
  if (!Number.isFinite(seconds) || !seconds || seconds <= 0) return '—'
  const total = Math.round(seconds as number)
  const hours = Math.floor(total / 3600)
  const minutes = Math.floor((total % 3600) / 60)
  const rest = total % 60
  const pad = (n: number) => String(n).padStart(2, '0')
  return hours > 0 ? `${hours}:${pad(minutes)}:${pad(rest)}` : `${minutes}:${pad(rest)}`
}

export function totalDuration(entries: { duration_seconds: number | null }[]): number | null {
  const known = entries.filter((entry) => Number.isFinite(entry.duration_seconds) && (entry.duration_seconds ?? 0) > 0)
  if (!known.length) return null
  return known.reduce((sum, entry) => sum + (entry.duration_seconds ?? 0), 0)
}

export function stateLabel(state: EngineState): string {
  switch (state) {
    case 'playing': return 'Playing'
    case 'paused': return 'Paused'
    case 'buffering': return 'Buffering'
    case 'loading': return 'Loading'
    case 'connecting': return 'Connecting'
    case 'completed': return 'Completed'
    case 'waiting_manual': return 'Waiting for your choice'
    case 'recovering': return 'Restore available'
    case 'disconnected': return 'Disconnected'
    case 'error': return 'Error'
    default: return 'Ready'
  }
}
