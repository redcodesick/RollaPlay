import type { DownloadPhase, DownloadProgress } from './downloader'

type ProgressIdentity = Pick<DownloadProgress, 'index' | 'total' | 'title'>

const PHASE_LABELS: Record<DownloadPhase, string> = {
  starting: 'Starting',
  downloading: 'Downloading',
  merging: 'Combining',
  converting: 'Optimizing',
  complete: 'Complete',
  cancelled: 'Cancelled',
  failed: 'Failed',
}

export const phaseLabel = (phase: DownloadPhase) => PHASE_LABELS[phase] ?? 'Working'

export const progressIdentity = (item: ProgressIdentity) => {
  const title = item.title?.trim()
  if (title) return title
  return progressCounter(item) || 'Download'
}

export const progressCounter = (item: Pick<DownloadProgress, 'index' | 'total'>) =>
  item.total > 1 ? `Link ${item.index + 1} of ${item.total}` : ''

export const clampedPercent = (percent: number | null) =>
  percent === null || !Number.isFinite(percent)
    ? null
    : Math.max(0, Math.min(100, Math.round(percent)))
