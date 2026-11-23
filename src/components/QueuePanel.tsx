import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  AlertCircle, ArrowDown, ArrowUp, Check, ChevronRight, Clock3, GripVertical,
  ListMusic, LoaderCircle, Pause, Play, Plus, RefreshCw, Repeat, Repeat1,
  Shuffle, SkipBack, SkipForward, Trash2, X,
} from 'lucide-react'
import {
  addPlaylistItems, createPlaylist, deletePlaylist, formatDuration, isPlaylistAvailable,
  listPlaylistItems, listPlaylists, playMediaNow, playNext as commandNext, playNow, playPrevious,
  queueAdd, queueAddPlaylist, queueClear, queueRemove, queueReorder, recoverPlayback,
  removePlaylistItem, renamePlaylist, reorderPlaylistItem, selectPlaylist,
  setPlaybackMode, setRepeat, setShuffle, totalDuration,
  type PlaybackSnapshot, type PlaylistItem, type PlaylistSummary,
  type QueueAddMode,
} from '../playlist'

type Props = {
  snapshot: PlaybackSnapshot | null
  castConnected: boolean
  selectedMediaIds: number[]
  onNotice: (message: string, level?: 'info' | 'success' | 'warning' | 'error') => void
  onTogglePlay: () => void
  isPlaying: boolean
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
  snapshot, castConnected, selectedMediaIds, onNotice, onTogglePlay, isPlaying,
}: Props) {
  const available = isPlaylistAvailable()
  const [playlists, setPlaylists] = useState<PlaylistSummary[]>([])
  const [selectedPlaylist, setSelectedPlaylist] = useState<number | null>(null)
  const [items, setItems] = useState<PlaylistItem[]>([])
  const [newName, setNewName] = useState('')
  const [renaming, setRenaming] = useState<number | null>(null)
  const [renameValue, setRenameValue] = useState('')
  const [busy, setBusy] = useState(false)
  const [dragEntry, setDragEntry] = useState<string | null>(null)
  const [dragItem, setDragItem] = useState<number | null>(null)

  const refresh = useCallback(async () => {
    if (!available) return
    try {
      const all = await listPlaylists()
      setPlaylists(all)
      const preferred = snapshot?.last_selected_playlist_id
        ?? all.find((playlist) => playlist.id === selectedPlaylist)?.id
        ?? all[0]?.id
        ?? null
      setSelectedPlaylist(preferred)
      if (preferred != null) setItems(await listPlaylistItems(preferred))
      else setItems([])
    } catch (error) {
      onNotice(friendlyError(error), 'error')
    }
  }, [available, onNotice, selectedPlaylist, snapshot?.last_selected_playlist_id])

  useEffect(() => { void refresh() }, [available])
  useEffect(() => {
    if (selectedPlaylist == null) { setItems([]); return }
    void listPlaylistItems(selectedPlaylist).then(setItems).catch((error) => onNotice(friendlyError(error), 'error'))
  }, [selectedPlaylist, onNotice])

  const guard = useCallback(async (action: () => Promise<unknown>) => {
    setBusy(true)
    try {
      await action()
      await refresh()
    } catch (error) {
      onNotice(friendlyError(error), 'error')
    } finally {
      setBusy(false)
    }
  }, [onNotice, refresh])

  function requireSelection(): number[] {
    if (!selectedMediaIds.length) {
      onNotice('Select one or more videos in the Media library first.', 'warning')
      return []
    }
    return selectedMediaIds
  }

  async function addToQueue(mode: QueueAddMode, ids = selectedMediaIds) {
    if (!ids.length) { onNotice('Select videos in the Media library or choose a playlist.', 'warning'); return }
    await guard(async () => {
      await queueAdd(ids, mode)
      onNotice(mode === 'play_now' ? 'Playing selected videos.' : `${ids.length} video${ids.length === 1 ? '' : 's'} added to the queue.`, 'success')
    })
  }

  const queueDuration = useMemo(() => totalDuration(snapshot?.queue ?? []), [snapshot?.queue])
  const canControl = available && castConnected && !!snapshot?.current_entry_id

  return (
    <div className="queue-panel">
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
          {!snapshot?.queue.length ? <div className="queue-empty"><ListMusic size={22} /><span>No videos in the queue. Select videos in the Media library or add a playlist below.</span></div> : snapshot.queue.map((entry, index) => {
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

      <section className="queue-section" aria-labelledby="playlists-title">
        <div className="queue-section-head"><div><div className="section-eyebrow">SAVED</div><h2 id="playlists-title">Playlists</h2><p>Deleting a playlist never deletes your media files.</p></div></div>
        <form className="playlist-create" onSubmit={(event) => {
          event.preventDefault()
          const name = newName.trim()
          if (!name) return
          void guard(async () => {
            const id = await createPlaylist(name)
            setNewName('')
            setSelectedPlaylist(id)
            await selectPlaylist(id)
            onNotice(`Playlist “${name}” created.`, 'success')
          })
        }}>
          <input value={newName} onChange={(event) => setNewName(event.target.value)} placeholder="New playlist name" aria-label="New playlist name" maxLength={120} disabled={busy || !available} />
          <button type="submit" className="queue-primary" disabled={busy || !newName.trim() || !available}><Plus size={15} /> Create</button>
        </form>
        <div className="playlist-grid">
          {playlists.map((playlist) => (
            <article key={playlist.id} className={`playlist-card ${selectedPlaylist === playlist.id ? 'selected' : ''}`}>
              <button className="playlist-open" onClick={() => { setSelectedPlaylist(playlist.id); void selectPlaylist(playlist.id) }}>
                <ListMusic size={17} /><span><strong>{playlist.name}</strong><small>{playlist.item_count} items · {formatDuration(playlist.duration_seconds)}</small></span><ChevronRight size={16} />
              </button>
              <div className="playlist-card-actions">
                <button onClick={() => { setRenaming(playlist.id); setRenameValue(playlist.name) }} disabled={busy} title="Rename" aria-label={`Rename ${playlist.name}`}><RefreshCw size={14} /></button>
                <button onClick={() => void guard(async () => {
                  await queueAddPlaylist(playlist.id, 'append')
                  onNotice(`“${playlist.name}” added to the queue.`, 'success')
                })} disabled={busy} title="Add playlist to queue" aria-label={`Add ${playlist.name} to queue`}><Plus size={14} /></button>
                <button onClick={() => void guard(async () => { await deletePlaylist(playlist.id); onNotice(`Playlist “${playlist.name}” deleted. Your files were not changed.`, 'info') })} disabled={busy} title="Delete playlist" aria-label={`Delete ${playlist.name}`}><Trash2 size={14} /></button>
              </div>
              {renaming === playlist.id && <form className="playlist-rename" onSubmit={(event) => {
                event.preventDefault()
                const name = renameValue.trim()
                if (!name) return
                void guard(async () => { await renamePlaylist(playlist.id, name); setRenaming(null); onNotice('Playlist renamed.', 'success') })
              }}><input value={renameValue} onChange={(event) => setRenameValue(event.target.value)} maxLength={120} autoFocus /><button type="submit" aria-label="Save name"><Check size={14} /></button><button type="button" onClick={() => setRenaming(null)} aria-label="Cancel rename"><X size={14} /></button></form>}
            </article>
          ))}
          {!playlists.length && <div className="playlist-empty">No playlists yet. Create one above, then add selected videos.</div>}
        </div>

        {selectedPlaylist != null && <div className="playlist-detail">
          <div className="playlist-detail-head"><h3>{playlists.find((playlist) => playlist.id === selectedPlaylist)?.name ?? 'Playlist'}</h3><button className="queue-primary" onClick={() => {
            const ids = requireSelection()
            if (ids.length) void guard(async () => { await addPlaylistItems(selectedPlaylist, ids); onNotice(`${ids.length} video(s) added to the playlist.`, 'success') })
          }} disabled={busy || !selectedMediaIds.length}>Add selected videos</button></div>
          {items.length ? <div className="playlist-items">{items.map((item, index) => (
            <div key={item.id} className={`playlist-item ${item.availability_state !== 'available' ? 'missing' : ''} ${dragItem === item.id ? 'dragging' : ''}`}
              draggable
              onDragStart={() => setDragItem(item.id)}
              onDragEnd={() => setDragItem(null)}
              onDragOver={(event) => event.preventDefault()}
              onDrop={(event) => { event.preventDefault(); const from = dragItem; setDragItem(null); if (from && from !== item.id) void guard(async () => reorderPlaylistItem(from, index)) }}>
              <GripVertical size={14} /><span className="playlist-item-index">{index + 1}</span>
              <span className="playlist-item-copy" title={item.filename}>{item.filename}{item.availability_state !== 'available' && <em> · missing</em>}</span>
              <span>{formatDuration(item.duration_seconds)}</span>
              <button onClick={() => void guard(async () => playMediaNow(item.media_file_id))} disabled={busy} title="Play now" aria-label={`Play ${item.filename}`}><Play size={13} /></button>
              <button onClick={() => void guard(async () => queueAdd([item.media_file_id], 'play_next'))} disabled={busy} title="Play next in queue" aria-label={`Play ${item.filename} next`}><SkipForward size={13} /></button>
              <button onClick={() => void guard(async () => removePlaylistItem(item.id))} disabled={busy} title="Remove from playlist" aria-label={`Remove ${item.filename} from playlist`}><X size={13} /></button>
            </div>
          ))}</div> : <div className="playlist-empty">This playlist is empty. Select videos in the Media library and choose “Add selected videos”.</div>}
        </div>}
      </section>

      <section className="queue-section" aria-labelledby="history-title">
        <div className="queue-section-head"><div><div className="section-eyebrow">ACTIVITY</div><h2 id="history-title">Watch history</h2><p>Recorded from actual receiver playback.</p></div></div>
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
