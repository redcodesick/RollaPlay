import { listen } from '@tauri-apps/api/event'
import { invokeTracked } from './diagnostics'

export type DownloaderTools = {
  yt_dlp_found: boolean
  yt_dlp_ready: boolean
  ffmpeg_found: boolean
  ffmpeg_ready: boolean
  hd_ready: boolean
  hint: string
}

export type DownloadPhase = 'starting' | 'downloading' | 'merging' | 'converting' | 'complete' | 'cancelled' | 'failed'

export type DownloadProgress = {
  index: number
  total: number
  phase: DownloadPhase
  percent: number | null
  title: string
  detail: string
}

export type ToolInstallProgress = {
  phase: 'starting' | 'output' | 'complete' | 'failed'
  detail: string
  step: number
  total: number
}

export const getDownloaderTools = () =>
  invokeTracked<DownloaderTools>('downloader_tools', undefined, 'downloader')

export const prepareTools = (includeFfmpeg: boolean) =>
  invokeTracked<DownloaderTools>('prepare_tools', { includeFfmpeg }, 'downloader')

export const installToolsWinget = (installYtDlp: boolean, installFfmpeg: boolean) =>
  invokeTracked<DownloaderTools>('install_tools_winget', { installYtDlp, installFfmpeg }, 'downloader')

export const startDownload = (directory: string, urls: string[], hd: boolean, tvCompatible: boolean) =>
  invokeTracked<void>('start_download', { directory, urls, hd, tvCompatible }, 'downloader')

export const cancelDownload = () =>
  invokeTracked<boolean>('cancel_download', undefined, 'downloader')

export const onDownloadProgress = (handler: (progress: DownloadProgress) => void) =>
  listen<DownloadProgress>('download-progress', (event) => handler(event.payload))

export const onToolInstallProgress = (handler: (progress: ToolInstallProgress) => void) =>
  listen<ToolInstallProgress>('ffmpeg-install-progress', (event) => handler(event.payload))
