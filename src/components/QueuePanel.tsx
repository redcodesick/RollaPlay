import { useCallback, useMemo, useState } from 'react'
import {
  AlertCircle, ArrowDown, ArrowUp, Clock3, GripVertical,
  ListMusic, LoaderCircle, Pause, Play, Plus, RefreshCw, Repeat, Repeat1,
  Shuffle, SkipBack, SkipForward, Trash2, X,
} from 'lucide-react'
import {
  clearWatchHistory, formatDuration, isPlaylistAvailable,
  playNext as commandNext, playNow, playPrevious,
  queueAdd, queueClear, queueRemove, queueReorder, recoverPlayback,
  setPlaybackMode, setRepeat, setShuffle, totalDuration,
  type PlaybackSnapshot, type QueueAddMode,
} from '../playlist'
import PlaylistManager from './PlaylistManager'

type Props = {
  snapshot: PlaybackSnapshot | null
  castConnected: boolean
  selectedMediaIds: number[]
  onNotice: (message: string, level?: 'info' | 'success' | 'warning' | 'error') => void
  onSelectionConsumed?: () => void
  onTogglePlay: () => void
  isPlaying: boolean
  onHistoryCleared?: () => void
}

function friendlyError(error: unknown): string {
  return typeof error === 'string' ? error : error instanceof Error ? error.message : String(error)
}

function entryTone(entry: PlaybackSnapshot['queue'][number], snapshot: PlaybackSnapshot) {
  if (entry.availability_state !== 'available' || entry.status === 'unavailable') return 'unavailable'
  if (entry.entry_id === snapshot.current_entry_id) return 'current'
  if (entry.entry_id === snapshot.next_entry_id) return 'next'
  if (entry.status === 'completed') return 'completed'
  return 'pending'
}

function entryLabel(tone: string) {
  switch (tone) {
    case 'current': return 'NOW'
    case 'next': return 'NEXT'
    case 'completed': return 'DONE'
    case 'unavailable': return 'MISSING'
    default: return 'QUEUED'
  }
}

export default function QueuePanel({
  snapshot, castConnected, selectedMediaIds, onNotice, onSelectionConsumed, onTogglePlay, isPlaying,
  onHistoryCleared,
}: Props) {
  const available = isPlaylistAvailable()
  const [busy, setBusy] = useState(false)
  const [dragEntry, setDragEntry] = useState<string | null>(null)

  const guard = useCallback(async (action: () => Promise<unknown>) => {
    setBusy(true)
    try {
      await action()
    } catch (error) {
      onNotice(friendlyError(error), 'error')
    } finally {
      setBusy(false)
    }
  }, [onNotice])

  async function addToQueue(mode: QueueAddMode, ids = selectedMediaIds) {
    if (!ids.length) { onNotice('Select videos in the Media library or choose a playlist.', 'warning'); return }
    await guard(async () => {
      await queueAdd(ids, mode)
      onNotice(mode === 'play_now' ? 'Playing selected videos.' : `${ids.length} video${ids.length === 1 ? '' : 's'} added to the queue.`, 'success')
    })
  }

  const queueDuration = useMemo(() => totalDuration(snapshot?.queue ?? []), [snapshot?.queue])
  const canControl = available && castConnected && !!snapshot?.current_entry_id

  async function clearHistory() {
    if (!snapshot?.history.length) return
    await guard(async () => {
      await clearWatchHistory()
      onHistoryCleared?.()
      onNotice('Watch history cleared.', 'success')
    })
  }

  return (
    <div className="queue-panel">
      <PlaylistManager
        snapshot={snapshot}
        selectedMediaIds={selectedMediaIds}
        onNotice={onNotice}
        onSelectionConsumed={onSelectionConsumed}
      />

      <section className="queue-section" aria-labelledby="queue-title">
        <div className="queue-section-head">
          <div><div className="section-eyebrow">PLAYBACK</div><h2 id="queue-title">Playback queue</h2><p>Order, mode and recovery are owned by the app — not the browser.</p></div>
          <button className="queue-ghost-button" onClick={() => void guard(async () => { await queueClear(); onNotice('Queue cleared.', 'info') })} disabled={busy || !snapshot?.queue.length}><Trash2 size={15} /> Clear</button>
        </div>

        <div className="queue-modes" role="group" aria-label="Playback options">
          <div className="queue-mode-switch">
            <button className={snapshot?.mode === 'manual' ? 'active' : ''} onClick={() => void guard(async () => setPlaybackMode('manual'))} disabled={busy}>Manual</button>
            <button className={snapshot?.mode === 'auto' ? 'active' : ''} onClick={() => void guard(async () => setPlaybackMode('auto'))} disabled={busy}>Automatic</button>
          </div>
          <button className={`queue-toggle ${snapshot?.shuffle ? 'active' : ''}`} onClick={() => void guard(async () => setShuffle(!snapshot?.shuffle))} disabled={busy} aria-pressed={!!snapshot?.shuffle} title="Shuffle keeps the current video in place"><Shuffle size={16} /> Shuffle</button>
          <div className="queue-repeat" role="group" aria-label="Repeat mode">
            {(['off', 'one', 'all'] as const).map((mode) => (
              <button key={mode} className={snapshot?.repeat === mode ? 'active' : ''} onClick={() => void guard(async () => setRepeat(mode))} disabled={busy} title={`Repeat ${mode}`} aria-label={`Repeat ${mode}`}>
                {mode === 'one' ? <Repeat1 size={16} /> : <Repeat size={16} />}<span>{mode.toUpperCase()}</span>
              </button>
            ))}
          </div>
        </div>

        <div className="queue-status-row">
          <span className={`queue-state state-${snapshot?.state ?? 'idle'}`} />
          <strong>{snapshot?.state ?? 'idle'}</strong>
          <span>{snapshot?.message ?? (snapshot?.queue.length ? `${snapshot.queue.length} items · ${formatDuration(queueDuration)}` : 'Queue is empty')}</span>
          {snapshot?.state === 'recovering' && <button className="queue-recover" onClick={() => void guard(async () => { await recoverPlayback(); onNotice('Reconnecting and resuming.', 'info') })} disabled={busy}><RefreshCw size={15} /> Reconnect &amp; resume</button>}
          {snapshot?.state === 'disconnected' && <button className="queue-recover" onClick={() => void guard(async () => { await recoverPlayback(); onNotice('Reconnecting and resuming.', 'info') })} disabled={busy}><RefreshCw size={15} /> Reconnect &amp; resume</button>}
        </div>
        {snapshot?.last_error && <div className="queue-error" role="alert"><AlertCircle size={15} />{snapshot.last_error}</div>}

        <div className="queue-actions">
          <button className="queue-primary" onClick={() => void addToQueue('play_now')} disabled={busy || !selectedMediaIds.length || !available}>Play selected now</button>
          <button className="queue-ghost-button" onClick={() => void addToQueue('play_next')} disabled={busy || !selectedMediaIds.length || !available}><SkipForward size={15} /> Play next</button>
          <button className="queue-ghost-button" onClick={() => void addToQueue('append')} disabled={busy || !selectedMediaIds.length || !available}><Plus size={15} /> Add to queue</button>
        </div>

        <div className="queue-list" role="list">
          {!snapshot?.queue.length ? <div className="queue-empty"><ListMusic size={22} /><span>No videos in the queue. Select videos in the Media library or add a playlist above.</span></div> : snapshot.queue.map((entry, index) => {
            const tone = entryTone(entry, snapshot)
            return (
              <div
                key={entry.entry_id}
                className={`queue-item tone-${tone} ${dragEntry === entry.entry_id ? 'dragging' : ''}`}
                role="listitem"
                draggable
                onDragStart={() => setDragEntry(entry.entry_id)}
                onDragEnd={() => setDragEntry(null)}
                onDragOver={(event) => event.preventDefault()}
                onDrop={(event) => {
                  event.preventDefault()
                  const from = dragEntry
                  setDragEntry(null)
                  if (from && from !== entry.entry_id) void guard(async () => queueReorder(from, index))
                }}
              >
                <GripVertical size={16} className="queue-grip" aria-hidden="true" />
                <span className="queue-index">{index + 1}</span>
                <div className="queue-item-copy"><strong title={entry.filename}>{entry.filename}</strong><span>{formatDuration(entry.duration_seconds)}</span></div>
                <span className={`queue-badge badge-${tone}`}>{entryLabel(tone)}</span>
                <div className="queue-item-actions">
                  <button onClick={() => void guard(async () => playNow(entry.entry_id))} disabled={busy || !available} title="Play now" aria-label={`Play ${entry.filename} now`}><Play size={14} /></button>
                  <button onClick={() => void guard(async () => queueReorder(entry.entry_id, Math.max(0, index - 1)))} disabled={busy || index === 0} title="Move up" aria-label="Move up"><ArrowUp size={14} /></button>
                  <button onClick={() => void guard(async () => queueReorder(entry.entry_id, index + 1))} disabled={busy || index === snapshot.queue.length - 1} title="Move down" aria-label="Move down"><ArrowDown size={14} /></button>
                  <button onClick={() => void guard(async () => queueRemove(entry.entry_id))} disabled={busy} title="Remove from queue" aria-label={`Remove ${entry.filename} from queue`}><X size={14} /></button>
                </div>
              </div>
            )
          })}
        </div>
      </section>

      <section className="queue-section" aria-labelledby="history-title">
        <div className="queue-section-head"><div><div className="section-eyebrow">ACTIVITY</div><h2 id="history-title">Watch history</h2><p>Recorded from actual receiver playback.</p></div><button className="queue-ghost-button" onClick={() => void clearHistory()} disabled={busy || !snapshot?.history.length}><Trash2 size={15} /> Clear history</button></div>
        <div className="history-list">
          {snapshot?.history.length ? snapshot.history.map((entry) => (
            <div key={entry.id} className="history-row"><Clock3 size={14} /><span title={entry.filename}>{entry.filename}</span><small>{entry.completion_state ?? 'started'}</small><small>{formatDuration(entry.watched_seconds)}</small></div>
          )) : <div className="playlist-empty">No completed playback recorded yet.</div>}
        </div>
      </section>

      <div className="queue-player-hints">
        <button onClick={onTogglePlay} disabled={!canControl}><span>{isPlaying ? <Pause size={15} /> : <Play size={15} />}</span> {isPlaying ? 'Pause' : 'Play'}</button>
        <button onClick={() => void guard(async () => playPrevious())} disabled={!canControl}><SkipBack size={15} /> Previous</button>
        <button onClick={() => void guard(async () => commandNext())} disabled={!canControl}><SkipForward size={15} /> Next</button>
        <small>Previous restarts the current video after {snapshot?.previous_restart_threshold_seconds ?? 30}s. With Shuffle on, it follows actual play history.</small>
      </div>
      {!available && <div className="queue-error"><AlertCircle size={15} />Queue controls require the installed Windows desktop app.</div>}
      {busy && <div className="queue-busy"><LoaderCircle size={15} className="spin-icon" /> Updating…</div>}
    </div>
  )
}
