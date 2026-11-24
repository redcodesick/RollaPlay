export const UPDATE_ENDPOINT = 'https://github.com/redcodesick/RollaPlay/releases/latest/download/latest.json'
export const RELEASES_URL = 'https://github.com/redcodesick/RollaPlay/releases'
export const MAX_RELEASE_NOTES_CHARS = 4000

export type UpdaterErrorKind =
  | 'network'
  | 'missingRelease'
  | 'invalidManifest'
  | 'missingArtifact'
  | 'signature'
  | 'download'
  | 'install'
  | 'unsupported'
  | 'versionMismatch'
  | 'missingSigning'
  | 'unknown'

export type NoteInline =
  | { kind: 'text'; text: string }
  | { kind: 'strong'; text: string }
  | { kind: 'code'; text: string }
  | { kind: 'link'; text: string; href: string }

export type ReleaseNoteBlock =
  | { kind: 'heading'; level: 1 | 2 | 3; inlines: NoteInline[] }
  | { kind: 'paragraph'; inlines: NoteInline[] }
  | { kind: 'bullet'; inlines: NoteInline[] }
  | { kind: 'code'; text: string }

export type ParsedReleaseNotes = {
  blocks: ReleaseNoteBlock[]
  truncated: boolean
}

export type UpdaterDownloadEvent =
  | { event: 'Started'; data: { contentLength?: number } }
  | { event: 'Progress'; data: { chunkLength: number } }
  | { event: 'Finished' }

export type DownloadProgressState = {
  loaded: number
  total?: number
  percent?: number
}

export const emptyDownloadProgress: DownloadProgressState = { loaded: 0 }

function stripHtml(value: string): string {
  return value.replace(/<[^>]*>/g, '').trim()
}

function isSafeExternalUrl(value: string): boolean {
  try {
    const url = new URL(value)
    return url.protocol === 'https:' && (
      url.hostname === 'github.com' ||
      url.hostname === 'raw.githubusercontent.com'
    )
  } catch {
    return false
  }
}

function parseInlines(raw: string): NoteInline[] {
  const inlines: NoteInline[] = []
  const pattern = /(\*\*[^*]+\*\*|`[^`]+`|\[[^\]]+\]\((https:\/\/[^)\s]+)\))/g
  let lastIndex = 0
  for (const match of raw.matchAll(pattern)) {
    const index = match.index ?? 0
    const before = stripHtml(raw.slice(lastIndex, index))
    if (before) inlines.push({ kind: 'text', text: before })
    const token = match[0]
    if (token.startsWith('**')) {
      inlines.push({ kind: 'strong', text: stripHtml(token.slice(2, -2)) })
    } else if (token.startsWith('`')) {
      inlines.push({ kind: 'code', text: stripHtml(token.slice(1, -1)) })
    } else {
      const label = stripHtml(token.slice(1, token.indexOf(']')))
      const href = match[2]
      inlines.push(isSafeExternalUrl(href)
        ? { kind: 'link', text: label, href }
        : { kind: 'text', text: label })
    }
    lastIndex = index + token.length
  }
  const after = stripHtml(raw.slice(lastIndex))
  if (after) inlines.push({ kind: 'text', text: after })
  return inlines
}

export function parseReleaseNotes(notes?: string | null): ParsedReleaseNotes {
  const original = (notes ?? '').replace(/\r\n?/g, '\n').trim()
  if (!original) return { blocks: [], truncated: false }

  const truncated = original.length > MAX_RELEASE_NOTES_CHARS
  let body = truncated ? original.slice(0, MAX_RELEASE_NOTES_CHARS) : original
  if (truncated) {
    const lastBreak = body.lastIndexOf('\n')
    body = `${lastBreak > 0 ? body.slice(0, lastBreak) : body}\n…`
  }

  const blocks: ReleaseNoteBlock[] = []
  let paragraph: string[] = []
  let code: string[] = []
  let inCode = false

  const flushParagraph = () => {
    const text = paragraph.join(' ').trim()
    if (text) blocks.push({ kind: 'paragraph', inlines: parseInlines(text) })
    paragraph = []
  }
  const flushCode = () => {
    if (code.length) blocks.push({ kind: 'code', text: code.join('\n') })
    code = []
  }

  for (const line of body.split('\n')) {
    if (line.trim().startsWith('```')) {
      if (inCode) flushCode()
      else flushParagraph()
      inCode = !inCode
      continue
    }
    if (inCode) {
      code.push(line)
      continue
    }
    const heading = /^(#{1,3})\s+(.+)$/.exec(line)
    const bullet = /^\s*(?:[-*+]|\d+\.)\s+(.+)$/.exec(line)
    if (heading) {
      flushParagraph()
      blocks.push({
        kind: 'heading',
        level: heading[1].length as 1 | 2 | 3,
        inlines: parseInlines(heading[2]),
      })
    } else if (bullet) {
      flushParagraph()
      blocks.push({ kind: 'bullet', inlines: parseInlines(bullet[1]) })
    } else if (!line.trim()) {
      flushParagraph()
    } else {
      paragraph.push(line.trim())
    }
  }
  flushParagraph()
  flushCode()
  return { blocks, truncated }
}

export function compareVersions(current: string, candidate: string): -1 | 0 | 1 {
  const parse = (value: string) => {
    const clean = value.trim().replace(/^v/i, '')
    const [core, ...pre] = clean.split('-')
    const numbers = core.split('.').map((part) => {
      const parsed = Number.parseInt(part, 10)
      return Number.isFinite(parsed) ? parsed : 0
    })
    return { numbers, pre }
  }
  const a = parse(current)
  const b = parse(candidate)
  const length = Math.max(a.numbers.length, b.numbers.length)
  for (let index = 0; index < length; index += 1) {
    const left = a.numbers[index] ?? 0
    const right = b.numbers[index] ?? 0
    if (left < right) return -1
    if (left > right) return 1
  }
  if (a.pre.length === 0 && b.pre.length > 0) return 1
  if (a.pre.length > 0 && b.pre.length === 0) return -1
  const preLength = Math.max(a.pre.length, b.pre.length)
  for (let index = 0; index < preLength; index += 1) {
    const left = a.pre[index]
    const right = b.pre[index]
    if (left === undefined) return -1
    if (right === undefined) return 1
    if (left === right) continue
    const leftNumeric = /^\d+$/.test(left)
    const rightNumeric = /^\d+$/.test(right)
    if (leftNumeric && rightNumeric) {
      const diff = Number(left) - Number(right)
      return diff < 0 ? -1 : 1
    }
    if (leftNumeric) return -1
    if (rightNumeric) return 1
    return left < right ? -1 : 1
  }
  return 0
}

export function resolveCheckResult(currentVersion: string, remoteVersion: string | null): 'upToDate' | 'available' {
  if (!remoteVersion) return 'upToDate'
  return compareVersions(remoteVersion, currentVersion) > 0 ? 'available' : 'upToDate'
}

export function applyDownloadEvent(
  current: DownloadProgressState,
  event: UpdaterDownloadEvent,
): DownloadProgressState {
  if (event.event === 'Started') {
    const total = event.data.contentLength && event.data.contentLength > 0
      ? event.data.contentLength
      : undefined
    return { loaded: 0, total, percent: undefined }
  }
  if (event.event === 'Finished') {
    return {
      loaded: current.loaded,
      total: current.total,
      percent: current.total && current.total > 0 ? 100 : undefined,
    }
  }
  const loaded = current.loaded + Math.max(0, event.data.chunkLength)
  const percent = current.total && current.total > 0
    ? Math.min(100, Math.max(0, Math.round((loaded / current.total) * 100)))
    : undefined
  return { loaded, total: current.total, percent }
}

const ERROR_PATTERNS: Array<[UpdaterErrorKind, RegExp]> = [
  ['signature', /signature|minisign|verification|require.?signed/i],
  ['missingSigning', /pubkey|private key|signing key|emptyendpoints|insecure.?transport/i],
  ['invalidManifest', /manifest|deserialize|json|parse|targets?\s+not\s+found/i],
  ['missingRelease', /release\s+not\s+found|releasenotfound|no release|404/i],
  ['missingArtifact', /missing.*(asset|artifact|file)|asset.*missing/i],
  ['versionMismatch', /version|downgrade/i],
  ['unsupported', /unsupported|not supported|invalid updater format/i],
  ['install', /install|msiexec|nsis|shell|permission|admin/i],
  ['download', /download|stream|chunk|interrupted/i],
  ['network', /network|timeout|timed out|dns|connect|offline|unreachable|http/i],
]

export function updaterErrorKind(error: unknown): UpdaterErrorKind {
  const text = error instanceof Error ? error.message : String(error ?? '')
  for (const [kind, pattern] of ERROR_PATTERNS) {
    if (pattern.test(text)) return kind
  }
  return 'unknown'
}

const ERROR_MESSAGES: Record<UpdaterErrorKind, string> = {
  network: 'Could not reach GitHub. Check your internet connection and try again.',
  missingRelease: 'No stable release is available yet. Try again later.',
  invalidManifest: 'GitHub returned an invalid update manifest. Try again later.',
  missingArtifact: 'The release is missing its update file. Try again later.',
  signature: 'The update failed signature verification and was not installed.',
  download: 'The download was interrupted. Nothing was installed.',
  install: 'The update could not be installed. Your library and settings were not changed.',
  unsupported: 'This installer format is not supported for automatic updates.',
  versionMismatch: 'The update version did not match this installation.',
  missingSigning: 'The release is missing signing information and was rejected.',
  unknown: 'The update check failed. Your library and settings were not changed.',
}

export function updaterErrorMessage(error: unknown): string {
  return ERROR_MESSAGES[updaterErrorKind(error)]
}

export function isRetryableUpdaterError(error: unknown): boolean {
  const kind = updaterErrorKind(error)
  return kind === 'network' || kind === 'missingRelease' || kind === 'invalidManifest'
    || kind === 'missingArtifact' || kind === 'download' || kind === 'install' || kind === 'unknown'
}

export type UpdatePhase =
  | 'idle'
  | 'checking'
  | 'upToDate'
  | 'available'
  | 'downloading'
  | 'installing'
  | 'restartReady'
  | 'error'

export type RetryAction = 'check' | 'download' | 'restart'

export type UpdateFailure = {
  kind: UpdaterErrorKind
  message: string
  retryable: boolean
  retryAction: RetryAction
}

export type UpdateViewState = {
  phase: UpdatePhase
  installedVersion: string
  latestVersion?: string
  releaseDate?: string
  notes?: ParsedReleaseNotes
  lastCheckedAt?: string
  progress: DownloadProgressState
  error?: UpdateFailure
}

export type RemoteUpdateInfo = {
  version: string
  date?: string
  body?: string
}

export const initialUpdateState = (installedVersion: string): UpdateViewState => ({
  phase: 'idle',
  installedVersion,
  progress: emptyDownloadProgress,
})

export function beginCheck(state: UpdateViewState): UpdateViewState {
  return {
    ...state,
    phase: 'checking',
    error: undefined,
    progress: emptyDownloadProgress,
  }
}

export function checkUpToDate(state: UpdateViewState, checkedAt: string): UpdateViewState {
  return {
    ...state,
    phase: 'upToDate',
    latestVersion: state.installedVersion,
    releaseDate: undefined,
    notes: undefined,
    lastCheckedAt: checkedAt,
    progress: emptyDownloadProgress,
    error: undefined,
  }
}

export function checkAvailable(
  state: UpdateViewState,
  remote: RemoteUpdateInfo,
  checkedAt: string,
): UpdateViewState {
  return {
    ...state,
    phase: 'available',
    latestVersion: remote.version,
    releaseDate: remote.date,
    notes: parseReleaseNotes(remote.body),
    lastCheckedAt: checkedAt,
    progress: emptyDownloadProgress,
    error: undefined,
  }
}

export function downloadStarted(state: UpdateViewState): UpdateViewState {
  return {
    ...state,
    phase: 'downloading',
    error: undefined,
    progress: emptyDownloadProgress,
  }
}

export function downloadFinished(state: UpdateViewState): UpdateViewState {
  return {
    ...state,
    phase: 'installing',
    progress: {
      loaded: state.progress.loaded,
      total: state.progress.total,
      percent: state.progress.total && state.progress.total > 0 ? 100 : undefined,
    },
  }
}

export function installFinished(state: UpdateViewState): UpdateViewState {
  return { ...state, phase: 'restartReady', error: undefined }
}

export function updateFailed(
  state: UpdateViewState,
  error: unknown,
  retryAction: RetryAction,
): UpdateViewState {
  return {
    ...state,
    phase: 'error',
    error: {
      kind: updaterErrorKind(error),
      message: updaterErrorMessage(error),
      retryable: isRetryableUpdaterError(error),
      retryAction,
    },
  }
}

export function restartFailed(state: UpdateViewState, error: unknown): UpdateViewState {
  return {
    ...state,
    phase: 'error',
    error: {
      kind: updaterErrorKind(error),
      message: 'RollaPlay could not restart. Close and reopen it manually.',
      retryable: true,
      retryAction: 'restart',
    },
  }
}

export function retryUpdate(state: UpdateViewState): UpdateViewState {
  const action = state.error?.retryAction ?? 'check'
  if (action === 'restart') return { ...state, phase: 'restartReady', error: undefined }
  if (action === 'download' && state.latestVersion) {
    return { ...state, phase: 'available', error: undefined, progress: emptyDownloadProgress }
  }
  return beginCheck(state)
}

export function canCheckForUpdates(state: UpdateViewState): boolean {
  return state.phase !== 'checking' && state.phase !== 'downloading' && state.phase !== 'installing'
}

export function formatCheckedAt(value?: string): string {
  if (!value) return 'Not checked yet'
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return 'Not checked yet'
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(date)
}

export function formatReleaseDate(value?: string): string {
  if (!value) return 'Not published'
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return value
  return new Intl.DateTimeFormat(undefined, { dateStyle: 'long' }).format(date)
}
