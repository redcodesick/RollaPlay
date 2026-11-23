import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react'
import useSWR, { useSWRConfig } from 'swr'
import { convertFileSrc } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import {
  AlertCircle, Check, Clock3, Disc3, Download, FolderPlus, GripVertical, LoaderCircle,
  MoreHorizontal, Music2, Play, Plus, RefreshCw, SkipForward, Trash2, Upload, X,
} from 'lucide-react'
import {
  addPlaylistItems, createPlaylist, deletePlaylist, exportPlaylistFile, formatDuration,
  getPlaybackState, importPlaylistFiles, isPlaylistAvailable, listPlaylistItems, listPlaylists,
  playMediaNow, playPlaylist, queueAdd, queueAddPlaylist, removePlaylistItem, renamePlaylist,
  reorderPlaylistItem, selectPlaylist, type PlaybackSnapshot, type PlaylistFileFormat,
  type PlaylistItem, type PlaylistSummary,
} from '../playlist'
import { debugLog, friendlyError, invokeTracked } from '../diagnostics'
import { getMediaFiles, type MediaDirectory, type MediaFile, type ScanResult } from '../media'

const coverTones = ['red', 'amber', 'paper', 'steel']

function coverTone(id: number) {
  return coverTones[id % coverTones.length]
}

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
  const [indexingVideos, setIndexingVideos] = useState(false)

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

  // Cover art is built from thumbnails of indexed videos already in a playlist.
  // It is purely decorative; a failed thumbnail lookup never blocks playlist UI.
  const { data: coverFiles } = useSWR(
    available && !compact ? ['playlist-covers', '', '', 'date'] as const : null,
    getMediaFiles,
    { refreshInterval: 60000 },
  )
  const thumbnailByMediaId = useMemo(() => {
    const map = new Map<number, string>()
    for (const file of coverFiles ?? []) {
      if (file.thumbnail_status === 'ready' && file.thumbnail_path) map.set(file.id, file.thumbnail_path)
    }
    return map
  }, [coverFiles])
  const [covers, setCovers] = useState<Record<number, string | null>>({})

  useEffect(() => {
    if (!available || compact || !playlists.length || !thumbnailByMediaId.size) return
    let active = true
    void Promise.all(playlists.map(async (playlist) => {
      const firstWithArt = (await listPlaylistItems(playlist.id))
        .find((item) => thumbnailByMediaId.has(item.media_file_id))
      const thumbnail = firstWithArt ? thumbnailByMediaId.get(firstWithArt.media_file_id) : null
      return [playlist.id, thumbnail ? convertFileSrc(thumbnail) : null] as const
    })).then((entries) => {
      if (active) setCovers(Object.fromEntries(entries))
    }).catch(() => {
      // Missing or unreadable thumbnails fall back to the gradient cover.
    })
    return () => { active = false }
  }, [available, compact, playlists, thumbnailByMediaId])

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

  // Lets an empty playlist add real videos directly: pick folders, index them,
  // then add the freshly indexed files to the playlist that is open.
  const addVideosFromFolder = useCallback(async () => {
    if (!available) return
    debugLog('playlists', 'button_click', { control: 'add_videos_from_folder' })
    setIndexingVideos(true)
    onNotice('Choose the folders with your videos…', 'info')
    try {
      const selection = await open({ directory: true, multiple: true, title: 'Choose folders containing videos' })
      if (!selection || (Array.isArray(selection) && selection.length === 0)) {
        onNotice('Folder selection cancelled.', 'info')
        return
      }
      const paths = Array.isArray(selection) ? selection : [selection]
      onNotice(`Indexing ${paths.length} folder${paths.length === 1 ? '' : 's'}…`, 'info')
      const directoryIds: number[] = []
      for (const path of paths) {
        const directory = await invokeTracked<MediaDirectory>('add_media_directory', { path }, 'playlists')
        directoryIds.push(directory.id)
        const result = await invokeTracked<ScanResult>('scan_media_directory', { directoryId: directory.id }, 'playlists')
        debugLog('playlists', 'folder_indexed', { directoryId: directory.id, filesIndexed: result.files_indexed })
      }
      await Promise.all([
        mutateGlobal('media-directories'),
        mutateGlobal((key: unknown) => Array.isArray(key) && key[0] === 'media-files'),
        mutateGlobal((key: unknown) => Array.isArray(key) && key[0] === 'playlist-covers'),
      ])
      const files = await invokeTracked<MediaFile[]>('list_media_files', {
        search: null, extension: null, sortBy: 'date',
      }, 'playlists')
      const indexed = files.filter((file) =>
        directoryIds.includes(file.directory_id) && file.availability_state === 'available')
      const existing = new Set(items.map((item) => item.media_file_id))
      const additions = indexed.map((file) => file.id).filter((id) => !existing.has(id))

      if (selectedPlaylist != null && additions.length) {
        preferredRefreshRef.current = selectedPlaylist
        await addPlaylistItems(selectedPlaylist, additions)
        setItems(await listPlaylistItems(selectedPlaylist))
        await refresh()
        onNotice(`${indexed.length} videos indexed · ${additions.length} added to the playlist.`, 'success')
      } else if (selectedPlaylist != null) {
        onNotice(indexed.length
          ? `${indexed.length} videos indexed; they are already in this playlist.`
          : 'No videos were found in the selected folders.', 'info')
      } else {
        onNotice(indexed.length
          ? `${indexed.length} videos indexed. Create a playlist, then add them.`
          : 'No videos were found in the selected folders.', 'info')
      }
    } catch (error) {
      debugLog('playlists', 'add_videos_from_folder_failed', { message: friendlyError(error) })
      onNotice(friendlyError(error), 'error')
    } finally {
      preferredRefreshRef.current = null
      setIndexingVideos(false)
    }
  }, [available, items, mutateGlobal, onNotice, refresh, selectedPlaylist])

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

  function importFiles() {
    debugLog('playlists', 'button_click', { control: 'import_playlist_files' })
    void guard(async () => {
      const result = await importPlaylistFiles()
      if (!result) {
        onNotice('Playlist file selection cancelled.', 'info')
        return
      }
      const parts: string[] = []
      if (result.playlists_created) parts.push(`${result.playlists_created} created`)
      if (result.playlists_updated) parts.push(`${result.playlists_updated} updated`)
      if (result.items_imported) parts.push(`${result.items_imported} videos matched`)
      if (result.skipped) parts.push(`${result.skipped} unchanged`)
      if (result.failed) parts.push(`${result.failed} could not be imported`)
      onNotice(
        parts.length
          ? `M3U/WPL import: ${parts.join(' · ')}.`
          : 'No playlist files were imported.',
        result.failed ? 'warning' : 'success',
      )
    })
  }

  function exportSelectedPlaylist(format: PlaylistFileFormat) {
    const playlist = playlists.find((item) => item.id === selectedPlaylist)
    if (!playlist) return
    debugLog('playlists', 'button_click', { control: 'export_playlist_file', playlistId: playlist.id, format })
    void guard(async () => {
      const result = await exportPlaylistFile(playlist.id, playlist.name, format)
      if (!result) {
        onNotice('Playlist export cancelled.', 'info')
        return
      }
      onNotice(`Exported ${result.items_written} video${result.items_written === 1 ? '' : 's'} as ${result.format.toUpperCase()}.`, 'success')
    })
  }

  function playlistCard(playlist: PlaylistSummary, featured = false) {
    const selected = selectedPlaylist === playlist.id
    const cover = covers[playlist.id]
    return (
      <article key={playlist.id} className={`playlist-card ${featured ? 'playlist-card-featured' : ''} ${selected ? 'selected' : ''}`}>
        <div className={`playlist-cover tone-${coverTone(playlist.id)}`}>
          {cover ? <img src={cover} alt="" /> : <Disc3 size={featured ? 38 : 26} strokeWidth={1.4} />}
          <button
            className="playlist-cover-play"
            onClick={() => playWholePlaylist(playlist)}
            disabled={busy || !playlist.item_count || !available}
            title="Play whole playlist"
            aria-label={`Play ${playlist.name}`}
          ><Play size={featured ? 20 : 15} fill="currentColor" /></button>
        </div>
        <button className="playlist-open" onClick={() => openPlaylist(playlist)}>
          <span><strong>{playlist.name}</strong><small>{playlist.item_count} items · {formatDuration(playlist.duration_seconds)}</small></span>
        </button>
        <div className="playlist-card-actions">
          <span className="playlist-card-type"><Music2 size={11} /> PLAYLIST</span>
          <button onClick={() => { setRenaming(playlist.id); setRenameValue(playlist.name) }} disabled={busy} title="Rename" aria-label={`Rename ${playlist.name}`}><RefreshCw size={13} /></button>
          <button onClick={() => void guard(async () => {
            await queueAddPlaylist(playlist.id, 'append')
            onNotice(`“${playlist.name}” added to the queue.`, 'success')
          })} disabled={busy} title="Add playlist to queue" aria-label={`Add ${playlist.name} to queue`}><Plus size={13} /></button>
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
          }} disabled={busy} title="Delete playlist" aria-label={`Delete ${playlist.name}`}><Trash2 size={13} /></button>
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
    )
  }

  const folderButton = (label: string) => (
    <button
      type="button"
      className="queue-primary playlist-add-folder"
      onClick={() => void addVideosFromFolder()}
      disabled={busy || indexingVideos || !available}
    >
      {indexingVideos ? <LoaderCircle size={15} className="spin-icon" /> : <FolderPlus size={15} />}
      {indexingVideos ? 'Indexing videos…' : label}
    </button>
  )

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
        <button
          type="button"
          className="queue-primary playlist-import"
          disabled={busy || !available}
          title="Create playlists from M3U, M3U8, or WPL files"
          onClick={importFiles}
        ><Upload size={15} /> Import M3U/WPL</button>
      </form>

      <div className="playlist-shelf-label"><span>{compact ? 'YOUR PLAYLISTS' : 'PICK UP WHERE YOU LEFT OFF'}</span>{!compact && playlists.length > 0 && <small>{playlists.length} saved</small>}</div>
      <div className="playlist-grid">
        {playlists.map((playlist, index) => playlistCard(playlist, !compact && index < 3))}
        {!playlists.length && <div className="playlist-empty playlist-empty-action">
          <p>No playlists yet. Create one above, or add a folder of videos to your library to get started.</p>
          {folderButton('Add videos from folder')}
        </div>}
      </div>

      {selectedPlaylist != null && <div className="playlist-detail">
        <div className="playlist-detail-head">
          <div className="playlist-detail-title">
            <span className="playlist-detail-kicker"><MoreHorizontal size={16} /> PLAYLIST</span>
            <h3>{playlists.find((playlist) => playlist.id === selectedPlaylist)?.name ?? 'Playlist'}</h3>
            <small>{items.length} videos · {formatDuration(items.reduce((total, item) => total + (item.duration_seconds ?? 0), 0))}</small>
          </div>
          <div className="playlist-detail-actions">
            <button className="playlist-detail-play" onClick={() => {
              const playlist = playlists.find((item) => item.id === selectedPlaylist)
              if (playlist) playWholePlaylist(playlist)
            }} disabled={busy || !items.length || !available}><Play size={14} fill="currentColor" /> Play playlist</button>
            {folderButton('Add from folder')}
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
            <button className="queue-primary" onClick={() => exportSelectedPlaylist('m3u')} disabled={busy || !items.length || !available} title="Export as M3U"><Download size={14} /> Export M3U</button>
            <button className="queue-primary" onClick={() => exportSelectedPlaylist('wpl')} disabled={busy || !items.length || !available} title="Export as WPL"><Download size={14} /> Export WPL</button>
          </div>
        </div>
        {items.length ? <>
          <div className="playlist-track-head"><span>#</span><span /><span>TITLE</span><span><Clock3 size={12} /> LENGTH</span><span /></div>
          <div className="playlist-items">{items.map((item, index) => (
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
              <span className="playlist-item-index"><GripVertical size={12} className="playlist-grip" />{index + 1}</span>
              <span className="playlist-item-art"><Disc3 size={14} /></span>
              <span className="playlist-item-copy" title={item.filename}>{item.filename}{item.availability_state !== 'available' && <em> · missing</em>}</span>
              <span className="playlist-item-time">{formatDuration(item.duration_seconds)}</span>
              <div className="playlist-item-actions">
                <button onClick={() => void guard(async () => playMediaNow(item.media_file_id))} disabled={busy} title="Play now" aria-label={`Play ${item.filename}`}><Play size={13} /></button>
                <button onClick={() => void guard(async () => queueAdd([item.media_file_id], 'play_next'))} disabled={busy} title="Play next in queue" aria-label={`Play ${item.filename} next`}><SkipForward size={13} /></button>
                <button onClick={() => void guard(async () => removePlaylistItem(item.id))} disabled={busy} title="Remove from playlist" aria-label={`Remove ${item.filename} from playlist`}><X size={13} /></button>
              </div>
            </div>
          ))}</div>
        </> : <div className="playlist-empty playlist-empty-action">
          <p>This playlist is empty. Choose a folder with videos to add them now, or select already-indexed videos above and choose “Add selected videos”.</p>
          {folderButton('Add videos from folder')}
        </div>}
      </div>}

      {!available && <div className="queue-error"><AlertCircle size={15} />Playlist controls require the installed Windows desktop app.</div>}
      {busy && <div className="playlist-manager-busy"><LoaderCircle size={15} className="spin-icon" /> Updating playlists…</div>}
    </section>
  )
}
