import { describe, expect, it } from 'vitest'
import { checkStateLabel, checkStateTone, eventCodeLabel, linkQualityLabel, transportLabel } from './network-diagnostics'

describe('diagnostics state rendering', () => {
  it('labels transports without exposing device details', () => {
    expect(transportLabel('dlna')).toBe('DLNA / UPnP')
    expect(transportLabel('cast')).toBe('Google Cast')
    expect(transportLabel('unknown')).toBe('Not selected')
  })

  it('maps backend states to fixed safe labels', () => {
    expect(checkStateLabel('ok')).toBe('OK')
    expect(checkStateLabel('warning')).toBe('CHECK')
    expect(checkStateLabel('failed')).toBe('FAILED')
    expect(checkStateLabel('not_run')).toBe('NOT RUN')
    expect(checkStateLabel(undefined)).toBe('UNKNOWN')
  })

  it('maps backend states to visual tones without exposing detail text', () => {
    expect(checkStateTone('ok')).toBe('ok')
    expect(checkStateTone('warning')).toBe('warn')
    expect(checkStateTone('failed')).toBe('bad')
    expect(checkStateTone('unknown')).toBe('idle')
  })

  it('maps link tiers and event codes to fixed safe labels', () => {
    expect(linkQualityLabel('2_4_ghz')).toBe('2.4 GHz')
    expect(linkQualityLabel('weak')).toBe('Weak')
    expect(linkQualityLabel('unavailable')).toBe('Not exposed by Windows')
    expect(eventCodeLabel('CONTROL_OK_MEDIA_STALLED')).toBe('Control answered, media stalled')
    expect(eventCodeLabel('unexpected')).toBe('Logged check event')
  })
})
