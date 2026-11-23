import { useCallback, useEffect, useId, useRef, useState } from 'react'
import { useSWRConfig } from 'swr'
import {
  AlertCircle, Check, ChevronRight, GripVertical, ListMusic, LoaderCircle, Play, Plus,
  RefreshCw, SkipForward, Trash2, X,
} from 'lucide-react'
import {
  addPlaylistItems, createPlaylist, deletePlaylist, formatDuration, getPlaybackState,
  isPlaylistAvailable, listPlaylistItems, listPlaylists, playMediaNow, playPlaylist,
  queueAdd, queueAddPlaylist, removePlaylistItem, renamePlaylist, reorderPlaylistItem, selectPlaylist,
  type PlaybackSnapshot, type PlaylistItem, type PlaylistSummary,
} from '../playlist'
import { debugLog, friendlyError } from '../diagnostics'

type Props = {
  snapshot: PlaybackSnapshot | null
  selectedMediaIds: number[]
  onNotice: (message: string, level?: 'info' | 'success' | 'warning' | 'error') => void
  onSelectionConsumed?: () => void
  compact?: boolean
}

/** Shared Phase 5 playlist CRUD UI used by the Playlists page and the wizard. */
export default function PlaylistManager({
  selectedMediaIds, onNotice, onSelectionConsumed, compact = false,
}: Props) {
  const available = isPlaylistAvailable()
  const { mutate: mutateGlobal } = useSWRConfig()
  const titleId = useId()
  const [playlists, setPlaylists] = useState<PlaylistSummary[]>([])
  const [selectedPlaylist, setSelectedPlaylist] = useState<number | null>(null)
  const selectedPlaylistRef = useRef<number | null>(null)
  const preferredRefreshRef = useRef<number | null>(null)
  const [items, setItems] = useState<PlaylistItem[]>([])
  const [newName, setNewName] = useState('')
  const [renaming, setRenaming] = useState<number | null>(null)
  const [renameValue, setRenameValue] = useState('')
  const [dragItem, setDragItem] = useState<number | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => { selectedPlaylistRef.current = selectedPlaylist }, [selectedPlaylist])

  const refresh = useCallback(async () => {
    if (!available) return
    try {
      const [all, state] = await Promise.all([listPlaylists(), getPlaybackState()])
      setPlaylists(all)
      // A freshly created/selected playlist wins over possibly stale playback
      // state; otherwise the new playlist looked like it was never created.
      const preferred = preferredRefreshRef.current
        ?? state.last_selected_playlist_id
        ?? all.find((playlist) => playlist.id === selectedPlaylistRef.current)?.id
        ?? all[0]?.id
        ?? null
      setSelectedPlaylist(preferred)
      if (preferred == null) setItems([])
      debugLog('playlists', 'refresh', { count: all.length, selectedPlaylist: preferred })
      // Keep the sidebar count and queue state in sync across every surface.
      await Promise.all([
        mutateGlobal('phase5-playlists'),
        mutateGlobal('phase5-playback-state'),
      ])
    } catch (error) {
      onNotice(friendlyError(error), 'error')
    }
  }, [available, mutateGlobal, onNotice])

  useEffect(() => { void refresh() }, [refresh])

  useEffect(() => {
    if (selectedPlaylist == null || !available) { setItems([]); return }
    let active = true
    listPlaylistItems(selectedPlaylist).then((next) => {
      if (active) setItems(next)
    }).catch((error) => {
      if (active) onNotice(friendlyError(error), 'error')
    })
    return () => { active = false }
  }, [available, selectedPlaylist, onNotice])

  const guard = useCallback(async (action: () => Promise<unknown>) => {
    setBusy(true)
    try {
      await action()
      await refresh()
    } catch (error) {
      debugLog('playlists', 'operation_failed', { message: friendlyError(error) })
      onNotice(friendlyError(error), 'error')
    } finally {
      setBusy(false)
      preferredRefreshRef.current = null
    }
  }, [onNotice, refresh])

  function requireSelection(): number[] {
    if (!selectedMediaIds.length) {
      onNotice('Select one or more indexed videos first.', 'warning')
      return []
    }
    return selectedMediaIds
  }

  function openPlaylist(playlist: PlaylistSummary) {
    debugLog('playlists', 'button_click', { control: 'select_playlist', playlistId: playlist.id })
    void guard(async () => {
      preferredRefreshRef.current = playlist.id
      setSelectedPlaylist(playlist.id)
      await selectPlaylist(playlist.id)
      onNotice(`Playlist “${playlist.name}” selected.`, 'info')
    })
  }

  function playWholePlaylist(playlist: PlaylistSummary) {
    if (!playlist.item_count) {
      onNotice(`“${playlist.name}” is empty. Add videos first.`, 'warning')
      return
    }
    debugLog('playlists', 'button_click', { control: 'play_playlist', playlistId: playlist.id })
    void guard(async () => {
      preferredRefreshRef.current = playlist.id
      setSelectedPlaylist(playlist.id)
      await playPlaylist(playlist.id)
      onNotice(`Playing “${playlist.name}” on the Cast device. Each video advances when the TV reports it finished — no durations needed.`, 'success')
    })
  }

  return (
    <section className={`queue-section playlist-manager ${compact ? 'playlist-manager-compact' : ''}`} aria-labelledby={titleId}>
      <div className="queue-section-head">
        <div>
          <div className="section-eyebrow">SAVED</div>
          <h2 id={titleId}>Playlists</h2>
          <p>Deleting a playlist never deletes your media files.</p>
        </div>
      </div>

      <form className="playlist-create" onSubmit={(event) => {
        event.preventDefault()
        const name = newName.trim()
        if (!name) return
        debugLog('playlists', 'button_click', { control: 'create_playlist', name })
        void guard(async () => {
          const id = await createPlaylist(name)
          preferredRefreshRef.current = id
          setNewName('')
          await selectPlaylist(id)
          setSelectedPlaylist(id)
          onNotice(`Playlist “${name}” created.`, 'success')
        })
      }}>
        <input value={newName} onChange={(event) => setNewName(event.target.value)} placeholder="New playlist name" aria-label="New playlist name" maxLength={120} disabled={busy || !available} />
        <button type="submit" className="queue-primary" disabled={busy || !newName.trim() || !available}><Plus size={15} /> Create</button>
        <button
          type="button"
          className="queue-primary playlist-create-with-selection"
          disabled={busy || !newName.trim() || !selectedMediaIds.length || !available}
          title="Create the playlist and add the currently selected videos"
          onClick={() => {
            const name = newName.trim()
            if (!name || !selectedMediaIds.length) return
            debugLog('playlists', 'button_click', { control: 'create_playlist_with_selection', name, count: selectedMediaIds.length })
            void guard(async () => {
              const id = await createPlaylist(name)
              preferredRefreshRef.current = id
              await addPlaylistItems(id, selectedMediaIds)
              await selectPlaylist(id)
              setNewName('')
              setSelectedPlaylist(id)
              onSelectionConsumed?.()
              onNotice(`Playlist “${name}” created with ${selectedMediaIds.length} video${selectedMediaIds.length === 1 ? '' : 's'}.`, 'success')
            })
          }}
        ><Plus size={15} /> Create + add selected ({selectedMediaIds.length})</button>
      </form>

      <div className="playlist-grid">
        {playlists.map((playlist) => (
          <article key={playlist.id} className={`playlist-card ${selectedPlaylist === playlist.id ? 'selected' : ''}`}>
            <button className="playlist-open" onClick={() => openPlaylist(playlist)}>
              <ListMusic size={17} />
              <span><strong>{playlist.name}</strong><small>{playlist.item_count} items · {formatDuration(playlist.duration_seconds)}</small></span>
              <ChevronRight size={16} />
            </button>
            <div className="playlist-card-actions">
              <button className="playlist-play" onClick={() => playWholePlaylist(playlist)} disabled={busy || !playlist.item_count || !available} title="Play whole playlist" aria-label={`Play ${playlist.name}`}><Play size={14} fill="currentColor" /></button>
              <button onClick={() => { setRenaming(playlist.id); setRenameValue(playlist.name) }} disabled={busy} title="Rename" aria-label={`Rename ${playlist.name}`}><RefreshCw size={14} /></button>
              <button onClick={() => void guard(async () => {
                await queueAddPlaylist(playlist.id, 'append')
                onNotice(`“${playlist.name}” added to the queue.`, 'success')
              })} disabled={busy} title="Add playlist to queue" aria-label={`Add ${playlist.name} to queue`}><Plus size={14} /></button>
              <button onClick={() => {
                debugLog('playlists', 'button_click', { control: 'delete_playlist', playlistId: playlist.id })
                void guard(async () => {
                  await deletePlaylist(playlist.id)
                  if (selectedPlaylist === playlist.id) {
                    preferredRefreshRef.current = null
                    setSelectedPlaylist(null)
                  }
                  onNotice(`Playlist “${playlist.name}” deleted. Your files were not changed.`, 'info')
                })
              }} disabled={busy} title="Delete playlist" aria-label={`Delete ${playlist.name}`}><Trash2 size={14} /></button>
            </div>
            {renaming === playlist.id && <form className="playlist-rename" onSubmit={(event) => {
              event.preventDefault()
              const name = renameValue.trim()
              if (!name) return
              void guard(async () => {
                await renamePlaylist(playlist.id, name)
                setRenaming(null)
                onNotice('Playlist renamed.', 'success')
              })
            }}><input value={renameValue} onChange={(event) => setRenameValue(event.target.value)} maxLength={120} autoFocus /><button type="submit" aria-label="Save name"><Check size={14} /></button><button type="button" onClick={() => setRenaming(null)} aria-label="Cancel rename"><X size={14} /></button></form>}
          </article>
        ))}
        {!playlists.length && <div className="playlist-empty">No playlists yet. Create one above, then add indexed videos.</div>}
      </div>

      {selectedPlaylist != null && <div className="playlist-detail">
        <div className="playlist-detail-head">
          <h3>{playlists.find((playlist) => playlist.id === selectedPlaylist)?.name ?? 'Playlist'}</h3>
          <button className="playlist-detail-play" onClick={() => {
            const playlist = playlists.find((item) => item.id === selectedPlaylist)
            if (playlist) playWholePlaylist(playlist)
          }} disabled={busy || !items.length || !available}><Play size={14} fill="currentColor" /> Play playlist</button>
          <button className="queue-primary" onClick={() => {
            const ids = requireSelection()
            if (!ids.length) return
            debugLog('playlists', 'button_click', { control: 'add_videos', playlistId: selectedPlaylist, count: ids.length })
            void guard(async () => {
              await addPlaylistItems(selectedPlaylist, ids)
              onSelectionConsumed?.()
              onNotice(`${ids.length} video${ids.length === 1 ? '' : 's'} added to the playlist.`, 'success')
            })
          }} disabled={busy || !selectedMediaIds.length}>Add selected videos</button>
        </div>
        {items.length ? <div className="playlist-items">{items.map((item, index) => (
          <div key={item.id}
            className={`playlist-item ${item.availability_state !== 'available' ? 'missing' : ''} ${dragItem === item.id ? 'dragging' : ''}`}
            draggable
            onDragStart={() => setDragItem(item.id)}
            onDragEnd={() => setDragItem(null)}
            onDragOver={(event) => event.preventDefault()}
            onDrop={(event) => {
              event.preventDefault()
              const from = dragItem
              setDragItem(null)
              if (from && from !== item.id) {
                debugLog('playlists', 'reorder_item', { from, targetIndex: index })
                void guard(async () => reorderPlaylistItem(from, index))
              }
            }}
          >
            <GripVertical size={14} />
            <span className="playlist-item-index">{index + 1}</span>
            <span className="playlist-item-copy" title={item.filename}>{item.filename}{item.availability_state !== 'available' && <em> · missing</em>}</span>
            <span>{formatDuration(item.duration_seconds)}</span>
            <button onClick={() => void guard(async () => playMediaNow(item.media_file_id))} disabled={busy} title="Play now" aria-label={`Play ${item.filename}`}><Play size={13} /></button>
            <button onClick={() => void guard(async () => queueAdd([item.media_file_id], 'play_next'))} disabled={busy} title="Play next in queue" aria-label={`Play ${item.filename} next`}><SkipForward size={13} /></button>
            <button onClick={() => void guard(async () => removePlaylistItem(item.id))} disabled={busy} title="Remove from playlist" aria-label={`Remove ${item.filename} from playlist`}><X size={13} /></button>
          </div>
        ))}</div> : <div className="playlist-empty">This playlist is empty. Select indexed videos above and choose “Add selected videos”.</div>}
      </div>}

      {!available && <div className="queue-error"><AlertCircle size={15} />Playlist controls require the installed Windows desktop app.</div>}
      {busy && <div className="playlist-manager-busy"><LoaderCircle size={15} className="spin-icon" /> Updating playlists…</div>}
    </section>
  )
}
