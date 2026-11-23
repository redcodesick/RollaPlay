import { invoke } from '@tauri-apps/api/core'

export type DiagnosticArea = 'wizard' | 'media-folders' | 'playlists'

export function friendlyError(error: unknown): string {
  if (typeof error === 'string') return error
  if (error instanceof Error) return error.message
  return String(error)
}

const SENSITIVE_KEY = /token|secret|password|credential|authorization|bearer|url|stream|path/i

function redact(value: unknown, depth = 0): unknown {
  if (depth > 4) return '[depth]'
  if (Array.isArray(value)) {
    return value.length > 20
      ? `${value.length} items`
      : value.map((item) => redact(item, depth + 1))
  }
  if (value && typeof value === 'object') {
    const safe: Record<string, unknown> = {}
    for (const [key, item] of Object.entries(value)) {
      safe[key] = SENSITIVE_KEY.test(key) ? '[redacted]' : redact(item, depth + 1)
    }
    return safe
  }
  if (typeof value === 'string') {
    if (/^(https?:\/\/|wss?:\/\/)/i.test(value.trim())) return '[redacted-url]'
    return value.length > 180 ? `${value.slice(0, 90)}…` : value
  }
  if (value instanceof Error) return value.message
  return value
}

function redactedArgs(args?: Record<string, unknown>): Record<string, unknown> | undefined {
  return args ? redact(args) as Record<string, unknown> : undefined
}

function resultSummary(result: unknown): unknown {
  if (Array.isArray(result)) return { type: 'array', count: result.length }
  if (result && typeof result === 'object') return { type: 'object', fields: Object.keys(result as Record<string, unknown>) }
  return redact(result)
}

/**
 * Forwards a redacted line to the packaged Tauri sink. Webview console output
 * is not visible in release builds; failures here must never affect the app.
 */
function sink(area: DiagnosticArea, action: string, details: unknown): void {
  try {
    let text = ''
    if (details !== undefined && details !== null && details !== '') {
      const safe = redact(details)
      text = typeof safe === 'string' ? safe : JSON.stringify(safe)
    }
    void invoke<void>('wizard_diag_log', { area, action, details: text }).catch(() => {})
  } catch {
    /* diagnostics are best-effort */
  }
}

/**
 * Temporary first-run diagnostic logger. It records control flow and command
 * outcomes without logging pairing tokens, bearer tokens or streaming URLs.
 */
export function debugLog(area: DiagnosticArea, action: string, details?: unknown): void {
  sink(area, action, details)
  if (typeof console === 'undefined') return
  console.debug(`[WMS:${area}] ${action}`, details ?? '')
}

export function diagnosticError(area: DiagnosticArea, action: string, error: unknown, details?: unknown): void {
  const base = details && typeof details === 'object' && !Array.isArray(details)
    ? details as Record<string, unknown>
    : { details }
  sink(area, action, { ...base, error: friendlyError(error) })
  if (typeof console === 'undefined') return
  console.error(`[WMS:${area}] ${action}`, details ?? '', friendlyError(error))
}

/** Invoke a Tauri command with a consistent, redacted audit trail. */
export async function invokeTracked<T>(
  command: string,
  args: Record<string, unknown> | undefined,
  area: DiagnosticArea,
): Promise<T> {
  debugLog(area, 'tauri_invoke', { command, args: redactedArgs(args) })
  try {
    const result = await invoke<T>(command, args)
    debugLog(area, 'tauri_response', { command, result: resultSummary(result) })
    return result
  } catch (error) {
    diagnosticError(area, 'tauri_error', error, { command })
    throw error
  }
}
