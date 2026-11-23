import { useCallback, useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import {
  AlertCircle, ArrowDown, ArrowUp, Clock3, GripVertical,
  ListMusic, ListPlus, LoaderCircle, Pause, Play, Plus, RefreshCw, Repeat, Repeat1,
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

type QueueDrag = {
  entryId: string
  pointerId: number
  startY: number
  deltaY: number
  targetIndex: number
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
  const [drag, setDrag] = useState<QueueDrag | null>(null)
  const queueListRef = useRef<HTMLDivElement | null>(null)
  const dragRef = useRef<QueueDrag | null>(null)

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

  const queue = snapshot?.queue ?? []
  const queueRef = useRef(queue)
  queueRef.current = queue
  const remainingQueue = drag ? queue.filter((entry) => entry.entry_id !== drag.entryId) : queue
  const dropBeforeId = drag ? (remainingQueue[drag.targetIndex]?.entry_id ?? '') : ''

  // Return the insertion position in the array *after* the dragged item is
  // removed, which is exactly what `queueReorder` expects.
  function targetIndexFor(clientY: number, entryId: string) {
    const rows = Array.from(queueListRef.current?.querySelectorAll<HTMLDivElement>('[data-queue-entry]') ?? [])
      .filter((row) => row.dataset.queueEntry !== entryId)
    for (let index = 0; index < rows.length; index += 1) {
      const rect = rows[index].getBoundingClientRect()
      if (clientY < rect.top + rect.height / 2) return index
    }
    return rows.length
  }

  function beginDrag(event: ReactPointerEvent<HTMLButtonElement>, entryId: string, index: number) {
    if (!available || busy) return
    event.preventDefault()
    event.stopPropagation()
    const next = { entryId, pointerId: event.pointerId, startY: event.clientY, deltaY: 0, targetIndex: index }
    dragRef.current = next
    setDrag(next)
  }

  // Listen on the window instead of the grip button: element-level pointer
  // capture is unreliable inside WebView2, and the mouse often leaves the
  // small grip while dragging.
  const dragging = drag !== null
  useEffect(() => {
    if (!dragging) return
    const onMove = (event: PointerEvent) => {
      const current = dragRef.current
      if (!current || (event.pointerType === 'mouse' && event.buttons === 0)) return
      const targetIndex = targetIndexFor(event.clientY, current.entryId)
      const deltaY = event.clientY - current.startY
      if (targetIndex === current.targetIndex && deltaY === current.deltaY) return
      const next = { ...current, targetIndex, deltaY }
      dragRef.current = next
      setDrag(next)
    }
    const finish = (commit: boolean) => {
      const current = dragRef.current
      dragRef.current = null
      setDrag(null)
      if (!commit || !current) return
      const from = queueRef.current.findIndex((entry) => entry.entry_id === current.entryId)
      if (from >= 0 && from !== current.targetIndex) {
        void guard(() => queueReorder(current.entryId, current.targetIndex))
      }
    }
    const onUp = () => finish(true)
    const onCancel = () => finish(false)
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    window.addEventListener('pointercancel', onCancel)
    return () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onCancel)
    }
  }, [dragging, guard])

  function playNextInQueue(entryId: string) {
    const from = queue.findIndex((entry) => entry.entry_id === entryId)
    if (from < 0) return
    if (entryId === snapshot?.current_entry_id) {
      onNotice('This video is playing now.', 'info')
      return
    }
    const currentIndex = queue.findIndex((entry) => entry.entry_id === snapshot?.current_entry_id)
    // `queueReorder` inserts after removing the item, so account for the index
    // shift when the moving item started above the current one.
    const target = currentIndex < 0
      ? 0
      : Math.max(0, Math.min(queue.length - 1, (currentIndex > from ? currentIndex - 1 : currentIndex) + 1))
    if (from === target) {
      onNotice('This video is already next.', 'info')
      return
    }
    void guard(() => queueReorder(entryId, target))
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

        <div className="queue-list" role="list" ref={queueListRef}>
          {!snapshot?.queue.length ? <div className="queue-empty"><ListMusic size={22} /><span>No videos in the queue. Select videos in the Media library or add a playlist above.</span></div> : snapshot.queue.map((entry, index) => {
            const tone = entryTone(entry, snapshot)
            const isDragging = drag?.entryId === entry.entry_id
            const isDropTarget = !!drag && !isDragging && dropBeforeId === entry.entry_id
            const isCurrent = entry.entry_id === snapshot.current_entry_id
            return (
              <div
                key={entry.entry_id}
                data-queue-entry={entry.entry_id}
                className={`queue-item tone-${tone}${isDragging ? ' dragging' : ''}${isDropTarget ? ' drop-before' : ''}`}
                role="listitem"
                style={isDragging && drag ? { transform: `translateY(${drag.deltaY}px)` } : undefined}
              >
                <button
                  type="button"
                  className="queue-grip-button"
                  onPointerDown={(event) => beginDrag(event, entry.entry_id, index)}
                  disabled={busy || !available}
                  title="Drag to reorder"
                  aria-label={`Drag ${entry.filename} to reorder`}
                ><GripVertical size={16} className="queue-grip" aria-hidden="true" /></button>
                <span className="queue-index">{index + 1}</span>
                <div className="queue-item-copy"><strong title={entry.filename}>{entry.filename}</strong><span>{formatDuration(entry.duration_seconds)}</span></div>
                <span className={`queue-badge badge-${tone}`}>{entryLabel(tone)}</span>
                <div className="queue-item-actions">
                  <button onClick={() => void guard(async () => playNow(entry.entry_id))} disabled={busy || !available} title="Play now" aria-label={`Play ${entry.filename} now`}><Play size={14} /></button>
                  <button onClick={() => playNextInQueue(entry.entry_id)} disabled={busy || !available || isCurrent} title="Play next in queue" aria-label={`Play ${entry.filename} next in queue`}><ListPlus size={14} /></button>
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
