import type { DownloadPhase, DownloadProgress } from './downloader'

type ProgressIdentity = Pick<DownloadProgress, 'index' | 'total' | 'title'>
export type ProgressMap = Record<number, DownloadProgress>

const PHASE_LABELS: Record<DownloadPhase, string> = {
  starting: 'Starting',
  downloading: 'Downloading',
  merging: 'Combining',
  converting: 'Optimizing',
  complete: 'Complete',
  cancelled: 'Cancelled',
  skipped: 'Skipped',
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

const settledPhases: DownloadPhase[] = ['complete', 'skipped', 'cancelled']

/** A link is done with the batch when it finished, was skipped, or was cancelled. */
export const isSettledPhase = (phase?: DownloadPhase) =>
  !!phase && settledPhases.includes(phase)

/** Index only when every original link is settled and at least one file was saved. */
export const downloadsReadyToIndex = (items: ProgressMap, total: number) => {
  if (total <= 0) return false
  for (let index = 0; index < total; index += 1) {
    if (!isSettledPhase(items[index]?.phase)) return false
  }
  return Object.values(items).some((item) => item.phase === 'complete')
}

/**
 * Resumes after a failed or skipped link while keeping its original position.
 * Retry includes the chosen link; skip starts with the next unfinished link.
 * Already completed, skipped or cancelled links are never downloaded twice.
 */
export const resumeDownloadIndices = (
  items: ProgressMap,
  startIndex: number,
  total: number,
  includeStart: boolean,
): number[] => {
  if (startIndex < 0 || startIndex >= total) return []
  const indices: number[] = []
  for (let index = startIndex; index < total; index += 1) {
    if (index === startIndex && !includeStart) continue
    if (index !== startIndex && isSettledPhase(items[index]?.phase)) continue
    indices.push(index)
  }
  return indices
}
