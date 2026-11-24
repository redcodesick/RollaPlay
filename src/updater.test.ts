import { describe, expect, it } from 'vitest'
import {
  MAX_RELEASE_NOTES_CHARS, applyDownloadEvent, beginCheck, canCheckForUpdates, checkAvailable,
  checkUpToDate, compareVersions, downloadFinished, downloadStarted, initialUpdateState,
  installFinished, isRetryableUpdaterError, parseReleaseNotes, restartFailed, resolveCheckResult,
  retryUpdate, updaterErrorKind, updateFailed,
} from './updater'

const CHECKED_AT = '2026-11-24T12:00:00.000Z'

describe('update check results', () => {
  it('reports no update when the endpoint returns no newer release', () => {
    const checked = checkUpToDate(beginCheck(initialUpdateState('0.3.0')), CHECKED_AT)
    expect(checked.phase).toBe('upToDate')
    expect(checked.latestVersion).toBe('0.3.0')
    expect(checked.lastCheckedAt).toBe(CHECKED_AT)
    expect(checked.error).toBeUndefined()
  })

  it('reports an available update and keeps its real release metadata', () => {
    const checked = checkAvailable(beginCheck(initialUpdateState('0.2.0')), {
      version: '0.3.0',
      date: '2026-11-24T10:16:41Z',
      body: '## What changed\n- Signed updates\n- **No telemetry**',
    }, CHECKED_AT)
    expect(checked.phase).toBe('available')
    expect(checked.latestVersion).toBe('0.3.0')
    expect(checked.releaseDate).toBe('2026-11-24T10:16:41Z')
    expect(checked.notes?.blocks.some((block) => block.kind === 'heading')).toBe(true)
    expect(checked.notes?.blocks.some((block) => block.kind === 'bullet')).toBe(true)
  })

  it('does not offer a downgrade', () => {
    expect(resolveCheckResult('0.3.0', '0.2.0')).toBe('upToDate')
    expect(resolveCheckResult('0.3.0', null)).toBe('upToDate')
    expect(compareVersions('0.3.0', '0.2.0')).toBe(1)
    expect(compareVersions('v0.2.0', '0.3.0')).toBe(-1)
    expect(compareVersions('0.3.0', '0.3.0')).toBe(0)
    expect(compareVersions('0.3.0-beta.1', '0.3.0')).toBe(-1)
  })
})

describe('release notes parsing', () => {
  it('keeps only a constrained subset of Markdown', () => {
    const parsed = parseReleaseNotes(
      '### Notes\n- Add `signed` updates with [releases](https://github.com/redcodesick/RollaPlay/releases)\n<script>alert(1)</script>\n',
    )
    expect(parsed.blocks[0]).toMatchObject({ kind: 'heading', level: 3 })
    expect(parsed.blocks.some((block) => block.kind === 'bullet')).toBe(true)
    expect(parsed.truncated).toBe(false)
    const serialized = JSON.stringify(parsed)
    expect(serialized).not.toContain('<script>')
    expect(serialized).not.toContain('dangerouslySetInnerHTML')
  })

  it('does not turn non-GitHub or non-HTTPS links into clickable links', () => {
    const parsed = parseReleaseNotes(
      '[safe](https://github.com/redcodesick/RollaPlay/releases) [bad](http://example.test) [js](javascript:alert(1))',
    )
    const links = parsed.blocks.flatMap((block) => 'inlines' in block ? block.inlines : []).filter((inline) => inline.kind === 'link')
    expect(links).toHaveLength(1)
    expect(links[0]).toMatchObject({ kind: 'link', href: 'https://github.com/redcodesick/RollaPlay/releases' })
  })

  it('truncates oversized release notes', () => {
    const parsed = parseReleaseNotes('x'.repeat(MAX_RELEASE_NOTES_CHARS + 500))
    expect(parsed.truncated).toBe(true)
    expect(JSON.stringify(parsed).length).toBeLessThan(MAX_RELEASE_NOTES_CHARS + 500)
  })
})

describe('download progress', () => {
  it('tracks real byte counts and clamps the percent', () => {
    let progress = applyDownloadEvent(initialUpdateState('0.3.0').progress, {
      event: 'Started',
      data: { contentLength: 200 },
    })
    progress = applyDownloadEvent(progress, { event: 'Progress', data: { chunkLength: 50 } })
    expect(progress).toEqual({ loaded: 50, total: 200, percent: 25 })
    progress = applyDownloadEvent(progress, { event: 'Progress', data: { chunkLength: 400 } })
    expect(progress.percent).toBe(100)
    progress = applyDownloadEvent(progress, { event: 'Finished' })
    expect(progress.percent).toBe(100)
  })

  it('classifies an interrupted download without installing', () => {
    const failed = updateFailed(downloadStarted(initialUpdateState('0.3.0')), new Error('download stream interrupted'), 'download')
    expect(failed.phase).toBe('error')
    expect(failed.error?.kind).toBe('download')
    expect(failed.error?.retryAction).toBe('download')
    expect(failed.error?.message).toContain('Nothing was installed')
  })
})

describe('safe error handling', () => {
  it('rejects a signature mismatch and does not mark it as safely retryable', () => {
    const failed = updateFailed(initialUpdateState('0.3.0'), new Error('signature verification failed'), 'check')
    expect(updaterErrorKind('signature verification failed')).toBe('signature')
    expect(isRetryableUpdaterError('signature verification failed')).toBe(false)
    expect(failed.error?.kind).toBe('signature')
    expect(failed.error?.message).toContain('not installed')
  })

  it('classifies missing and invalid manifests', () => {
    expect(updaterErrorKind('release not found')).toBe('missingRelease')
    expect(updaterErrorKind('failed to deserialize update manifest')).toBe('invalidManifest')
    expect(updaterErrorKind('targets not found')).toBe('invalidManifest')
    const failed = updateFailed(initialUpdateState('0.3.0'), new Error('manifest parse failed'), 'check')
    expect(failed.error?.kind).toBe('invalidManifest')
    expect(failed.error?.message).toContain('invalid update manifest')
  })

  it('retries after a network failure', () => {
    const failed = updateFailed(initialUpdateState('0.3.0'), new Error('network timeout'), 'check')
    const retried = retryUpdate(failed)
    expect(failed.phase).toBe('error')
    expect(retried.phase).toBe('checking')
    expect(retried.error).toBeUndefined()
    expect(canCheckForUpdates(failed)).toBe(true)
    expect(canCheckForUpdates(retried)).toBe(false)
  })

  it('does not change the installed version before an explicit restart', () => {
    const ready = installFinished(downloadFinished(initialUpdateState('0.2.0')))
    expect(ready.phase).toBe('restartReady')
    expect(ready.installedVersion).toBe('0.2.0')
    expect(ready.latestVersion).toBeUndefined()

    const failedRestart = restartFailed(ready, new Error('relaunch failed'))
    expect(failedRestart.phase).toBe('error')
    expect(failedRestart.error?.retryAction).toBe('restart')
    expect(retryUpdate(failedRestart).phase).toBe('restartReady')
    expect(failedRestart.installedVersion).toBe('0.2.0')
  })
})
