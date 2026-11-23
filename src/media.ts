import { invoke, isTauri } from '@tauri-apps/api/core'

export type MediaDirectory = {
  id: number
  path: string
  added_at: string
  last_scanned_at: string | null
  availability_state: 'available' | 'partially_unavailable' | 'temporarily_unavailable' | 'removed_from_library'
}

export type MediaFile = {
  id: number
  directory_id: number
  path: string
  filename: string
  extension: string
  size_bytes: number
  modified_at: string
  indexed_at: string
  availability_state: 'available' | 'temporarily_unavailable' | 'permanently_missing' | 'removed_from_library'
  thumbnail_path: string | null
  thumbnail_status: 'pending' | 'ready' | 'failed'
  thumbnail_error: string | null
}

export type ScanProgress = {
  directory_id: number
  files_found: number
  inaccessible_entries: number
  phase: 'scanning' | 'complete' | 'cancelled' | 'temporarily_unavailable'
}

export type ThumbnailProgress = {
  media_file_id: number
  status: 'pending' | 'ready' | 'failed'
  error: string | null
}

export type ScanResult = {
  directory_id: number
  files_indexed: number
  files_removed: number
  renamed_files: number
  temporarily_unavailable_files: number
  permanently_missing_files: number
  inaccessible_entries: number
  playlists_imported: number
  cancelled: boolean
}

export type ThumbnailSettings = {
  width: number
  height: number
  quality: number
}

export type CacheCleanupResult = {
  files_removed: number
  bytes_removed: number
}

export const isDesktopApp = () => isTauri()

/** Immediate parent folder of an indexed file, used for folder-scoped selection. */
export function folderForPath(path: string) {
  const separator = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'))
  return separator > 0 ? path.slice(0, separator) : path
}

export async function getLocalStatus(): Promise<{ database_ready: boolean }> {
  return invoke<{ database_ready: boolean }>('get_local_status')
}

export function formatBytes(value: number) {
  if (value < 1024) return `${value} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let amount = value / 1024
  let unit = 0
  while (amount >= 1024 && unit < units.length - 1) {
    amount /= 1024
    unit += 1
  }
  return `${amount.toFixed(amount >= 100 ? 0 : 1)} ${units[unit]}`
}

export async function getMediaFiles([, search, extension, sortBy]: readonly [string, string, string, string]) {
  if (!isDesktopApp()) return [] as MediaFile[]
  return invoke<MediaFile[]>('list_media_files', {
    search: search || null,
    extension: extension || null,
    sortBy,
  })
}

export async function getMediaDirectories() {
  if (!isDesktopApp()) return [] as MediaDirectory[]
  return invoke<MediaDirectory[]>('list_media_directories')
}

export async function getThumbnailSettings() {
  if (!isDesktopApp()) return { width: 320, height: 180, quality: 82 } satisfies ThumbnailSettings
  return invoke<ThumbnailSettings>('get_thumbnail_settings')
}
