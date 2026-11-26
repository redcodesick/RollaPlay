import { describe, expect, it } from 'vitest'
import { clampedPercent, phaseLabel, progressCounter, progressIdentity } from './downloader-progress'

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
    expect(phaseLabel('failed')).toBe('Failed')
  })

  it('shows only real percentages, clamped to the bar', () => {
    expect(clampedPercent(null)).toBeNull()
    expect(clampedPercent(45.4)).toBe(45)
    expect(clampedPercent(120)).toBe(100)
    expect(clampedPercent(-4)).toBe(0)
  })
})
