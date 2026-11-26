import { describe, expect, it } from 'vitest'
import {
  clampedPercent, downloadsReadyToIndex, phaseLabel, progressCounter, progressIdentity,
  resumeDownloadIndices, type ProgressMap,
} from './downloader-progress'

describe('downloader progress display', () => {
  it('uses the real sanitized title, with a safe fallback', () => {
    expect(progressIdentity({ index: 1, total: 3, title: 'RollaPlay Studio Clip' }))
      .toBe('RollaPlay Studio Clip')
    expect(progressIdentity({ index: 1, total: 3, title: '' })).toBe('Link 2 of 3')
    expect(progressIdentity({ index: 0, total: 1, title: '' })).toBe('Download')
  })

  it('shows the real link position only for multi-link jobs', () => {
    expect(progressCounter({ index: 2, total: 4 })).toBe('Link 3 of 4')
    expect(progressCounter({ index: 0, total: 1 })).toBe('')
  })

  it('labels every real phase without inventing one', () => {
    expect(phaseLabel('starting')).toBe('Starting')
    expect(phaseLabel('downloading')).toBe('Downloading')
    expect(phaseLabel('merging')).toBe('Combining')
    expect(phaseLabel('converting')).toBe('Optimizing')
    expect(phaseLabel('complete')).toBe('Complete')
    expect(phaseLabel('cancelled')).toBe('Cancelled')
    expect(phaseLabel('skipped')).toBe('Skipped')
    expect(phaseLabel('failed')).toBe('Failed')
  })

  it('shows only real percentages, clamped to the bar', () => {
    expect(clampedPercent(null)).toBeNull()
    expect(clampedPercent(45.4)).toBe(45)
    expect(clampedPercent(120)).toBe(100)
    expect(clampedPercent(-4)).toBe(0)
  })

  it('indexes only when every original link is settled and at least one file completed', () => {
    const items: ProgressMap = {
      0: { index: 0, total: 3, phase: 'complete', percent: null, title: 'One', detail: '' },
      1: { index: 1, total: 3, phase: 'skipped', percent: null, title: 'Two', detail: '' },
      2: { index: 2, total: 3, phase: 'cancelled', percent: null, title: 'Three', detail: '' },
    }
    expect(downloadsReadyToIndex(items, 3)).toBe(true)

    items[1] = { ...items[1], phase: 'failed' }
    expect(downloadsReadyToIndex(items, 3)).toBe(false)

    delete items[2]
    expect(downloadsReadyToIndex(items, 3)).toBe(false)
  })

  it('resumes retry and skip from the original position without repeating finished links', () => {
    const items: ProgressMap = {
      0: { index: 0, total: 4, phase: 'complete', percent: null, title: 'One', detail: '' },
      1: { index: 1, total: 4, phase: 'failed', percent: null, title: 'Two', detail: '' },
      2: { index: 2, total: 4, phase: 'skipped', percent: null, title: 'Three', detail: '' },
      3: { index: 3, total: 4, phase: 'starting', percent: null, title: 'Four', detail: '' },
    }
    expect(resumeDownloadIndices(items, 1, 4, true)).toEqual([1, 3])
    expect(resumeDownloadIndices(items, 1, 4, false)).toEqual([3])
    expect(resumeDownloadIndices(items, 2, 4, true)).toEqual([2, 3])
    expect(resumeDownloadIndices(items, 3, 4, false)).toEqual([])
  })
})
