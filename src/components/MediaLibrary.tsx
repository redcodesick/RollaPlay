import { useEffect, useMemo, useState } from 'react'
import useSWR, { useSWRConfig } from 'swr'
import { listen } from '@tauri-apps/api/event'
import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import {
  AlertCircle, Cast, Check, Film, FolderPlus, HardDrive, ListMusic, LoaderCircle,
  Play, RefreshCw, ShieldCheck, X,
} from 'lucide-react'
import {
  folderForPath, getMediaFiles, getThumbnailSettings, isDesktopApp,
  type CacheCleanupResult, type MediaFile, type ThumbnailProgress, type ThumbnailSettings,
} from '../media'
import { FolderList, useMediaFolders } from './LibraryFolders'
import {
  castDeviceName, getCastPlaybackState, isCastAvailable, isCastSupportedExtension,
} from '../cast'
import { addPlaylistItems, createPlaylist, playMediaNow, queueAdd, selectPlaylist } from '../playlist'

const extensions = ['mp4', 'mkv', 'avi', 'mov', 'wmv', 'webm', 'mpeg', 'mpg', 'm4v', '3gp']
type SortBy = 'name' | 'date' | 'size'

function formatBytes(value: number) {
  if (value < 1024) return `${value} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let amount = value / 1024
  let unit = 0
  while (amount >= 1024 && unit < units.length - 1) {
    amount /= 1024
    unit += 1
  }
  return `${amount.toFixed(amount >= 100 ? 0 : 1)} ${units[unit]}`
}

function MediaCard({
  media, onRefresh, onCast, castReady, isCasting, selected, onToggleSelect,
}: {
  media: MediaFile
  onRefresh: (media: MediaFile) => Promise<void>
  onCast: (media: MediaFile) => void
  castReady: boolean
  isCasting: boolean
  selected: boolean
  onToggleSelect: (media: MediaFile) => void
}) {
  const [imageFailed, setImageFailed] = useState(false)
  const [isRefreshing, setIsRefreshing] = useState(false)
  const thumbnailUrl = media.thumbnail_path ? convertFileSrc(media.thumbnail_path) : null
  const showThumbnail = media.thumbnail_status === 'ready' && thumbnailUrl && !imageFailed
  const castSupported = isCastSupportedExtension(media.extension)
  const castTitle = castReady
    ? castSupported
      ? `Cast ${media.filename} to a Cast device`
      : `${media.extension.toUpperCase()} is not playable by the Cast Default Media Receiver`
    : 'Casting is available in the installed Windows desktop app'

  useEffect(() => setImageFailed(false), [thumbnailUrl])

  async function refresh() {
    setIsRefreshing(true)
    try {
      await onRefresh(media)
    } finally {
      setIsRefreshing(false)
    }
  }

  return (
    <article className={`media-card poster-${media.extension} ${selected ? 'selected' : ''}`}>
      <div className="media-artwork">
        <label className="media-select" title={selected ? 'Remove from selection' : 'Add to selection'}>
          <input type="checkbox" checked={selected} onChange={() => onToggleSelect(media)} aria-label={`Select ${media.filename}`} />
          <span>{selected ? <Check size={13} /> : null}</span>
        </label>
        {showThumbnail ? <img className="media-thumbnail" src={thumbnailUrl} alt="" onError={() => setImageFailed(true)} /> : <div className="media-placeholder" aria-label="Generic video thumbnail">
          <span className="artwork-orbit" />
          <span className="artwork-glow" />
          <Film size={27} strokeWidth={1.35} />
          {media.thumbnail_status === 'pending' && <span className="thumbnail-state" role="status"><LoaderCircle size={13} className="spin-icon" />Generating preview</span>}
          {(media.thumbnail_status === 'failed' || imageFailed) && <span className="thumbnail-state failed" title={media.thumbnail_error ?? 'Thumbnail extraction failed'} role="status">Preview unavailable</span>}
        </div>}
        <span className="media-extension">{media.extension.toUpperCase()}</span>
        <button className="thumbnail-refresh" onClick={refresh} disabled={isRefreshing} aria-label={`Regenerate thumbnail for ${media.filename}`} title={media.thumbnail_error ?? 'Regenerate thumbnail'}>
          {isRefreshing ? <LoaderCircle size={15} className="spin-icon" /> : <RefreshCw size={15} />}
        </button>
        <button
          className={`media-cast-button ${isCasting ? 'casting' : ''}`}
          onClick={() => onCast(media)}
          disabled={!castReady || !castSupported}
          aria-label={`Cast ${media.filename}`}
          title={castTitle}
        >
          <Cast size={16} />
          <span>Cast</span>
        </button>
      </div>
      <div className="media-card-info">
        <h3 title={media.filename}>{media.filename}</h3>
        <div className="media-card-meta"><span>{media.extension.toUpperCase()}</span><i />{formatBytes(media.size_bytes)}</div>
        <p title={folderForPath(media.path)}>{folderForPath(media.path)}</p>
        {media.thumbnail_status === 'failed' && media.thumbnail_error && <small className="thumbnail-error-text" title={media.thumbnail_error}>{media.thumbnail_error}</small>}
      </div>
    </article>
  )
}

export default function MediaLibrary({
  searchQuery, selectedMediaIds, onSelectionChange,
}: {
  searchQuery: string
  selectedMediaIds: number[]
  onSelectionChange: (ids: number[]) => void
}) {
  const [extension, setExtension] = useState('')
  const [sortBy, setSortBy] = useState<SortBy>('name')
  const [folderFilter, setFolderFilter] = useState('')
  const [playlistName, setPlaylistName] = useState('')
  const [savingPlaylist, setSavingPlaylist] = useState(false)
  const [notice, setNotice] = useState('')
  const [errorMessage, setErrorMessage] = useState('')
  const [castingMediaId, setCastingMediaId] = useState<number | null>(null)
  const isDesktop = isDesktopApp()
  const { mutate: mutateGlobal } = useSWRConfig()
  const castReady = isCastAvailable()
  const folders = useMediaFolders()
  const search = searchQuery.trim()
  const fileKey = useMemo(() => ['media-files', search, extension, sortBy] as const, [search, extension, sortBy])
  const { data: files = [], isLoading: isLoadingFiles, error: filesError, mutate: refreshFiles } = useSWR(fileKey, getMediaFiles)
  const { data: thumbnailSettings = { width: 320, height: 180, quality: 82 }, mutate: refreshThumbnailSettings } = useSWR('thumbnail-settings', getThumbnailSettings)
  const totalBytes = files.reduce((total, file) => total + file.size_bytes, 0)
  const folderOptions = useMemo(
    () => Array.from(new Set(files.map((file) => folderForPath(file.path)))).sort((a, b) => a.localeCompare(b, undefined, { sensitivity: 'base' })),
    [files],
  )
  const visibleFiles = useMemo(
    () => folderFilter ? files.filter((file) => folderForPath(file.path) === folderFilter) : files,
    [files, folderFilter],
  )
  const allVisibleSelected = visibleFiles.length > 0 && visibleFiles.every((file) => selectedMediaIds.includes(file.id))

  useEffect(() => {
    if (folderFilter && !folderOptions.includes(folderFilter)) setFolderFilter('')
  }, [folderFilter, folderOptions])

  useEffect(() => {
    if (!isDesktop) return
    let disposed = false
    let stopListening: (() => void) | undefined
    listen<ThumbnailProgress>('thumbnail-generation-progress', () => {
      void refreshFiles()
    }).then((stop) => {
      if (disposed) stop()
      else {
        stopListening = stop
        void invoke('prepare_thumbnail_cache').catch((error) => setErrorMessage(String(error)))
      }
    }).catch(() => setErrorMessage('Could not listen for thumbnail generation progress.'))
    return () => {
      disposed = true
      stopListening?.()
    }
  }, [isDesktop, refreshFiles])

  async function refreshThumbnail(media: MediaFile) {
    setErrorMessage('')
    try {
      await invoke('refresh_thumbnail', { mediaFileId: media.id })
      setNotice(`Refreshing thumbnail for ${media.filename}.`)
      await refreshFiles()
    } catch (error) {
      setErrorMessage(typeof error === 'string' ? error : error instanceof Error ? error.message : String(error))
    }
  }

  async function updateThumbnailSettings(update: Partial<ThumbnailSettings>) {
    const next = { ...thumbnailSettings, ...update }
    try {
      await invoke('set_thumbnail_settings', next)
      await Promise.all([refreshThumbnailSettings(), refreshFiles()])
      setNotice('Thumbnail settings saved. The local cache is regenerating in the background.')
    } catch (error) {
      setErrorMessage(typeof error === 'string' ? error : error instanceof Error ? error.message : String(error))
    }
  }

  async function cleanThumbnailCache() {
    try {
      const result = await invoke<CacheCleanupResult>('cleanup_thumbnail_cache')
      setNotice(`Cache cleanup removed ${result.files_removed} files (${formatBytes(result.bytes_removed)}).`)
    } catch (error) {
      setErrorMessage(typeof error === 'string' ? error : error instanceof Error ? error.message : String(error))
    }
  }

  function castVideo(media: MediaFile) {
    setErrorMessage('')
    setNotice('')
    setCastingMediaId(media.id)
    playMediaNow(media.id)
      .then(async () => {
        const device = castDeviceName(await getCastPlaybackState().catch(() => null))
        setNotice(device
          ? `Casting ${media.filename} to ${device}.`
          : `Casting ${media.filename}. Choose your TV in the sender panel if it opens.`)
      })
      .catch((error) => {
        setErrorMessage(typeof error === 'string' ? error : error instanceof Error ? error.message : String(error))
      })
      .finally(() => setCastingMediaId(null))
  }

  function toggleSelect(media: MediaFile) {
    onSelectionChange(
      selectedMediaIds.includes(media.id)
        ? selectedMediaIds.filter((id) => id !== media.id)
        : [...selectedMediaIds, media.id],
    )
  }

  function selectAllVisible() {
    if (!visibleFiles.length) return
    if (allVisibleSelected) {
      const visibleIds = new Set(visibleFiles.map((file) => file.id))
      onSelectionChange(selectedMediaIds.filter((id) => !visibleIds.has(id)))
      setNotice(`${visibleFiles.length} video${visibleFiles.length === 1 ? '' : 's'} removed from the selection.`)
      return
    }
    const ids = Array.from(new Set([...selectedMediaIds, ...visibleFiles.map((file) => file.id)]))
    onSelectionChange(ids)
    setNotice(`${visibleFiles.length} video${visibleFiles.length === 1 ? '' : 's'} from ${folderFilter || 'all folders'} selected.`)
  }

  async function saveSelectionAsPlaylist() {
    const name = playlistName.trim()
    if (!name) {
      setErrorMessage('Enter a playlist name first.')
      return
    }
    if (!selectedMediaIds.length) {
      setErrorMessage('Select at least one video first.')
      return
    }
    setErrorMessage('')
    setNotice('')
    setSavingPlaylist(true)
    try {
      const id = await createPlaylist(name)
      await addPlaylistItems(id, selectedMediaIds)
      await selectPlaylist(id)
      await Promise.all([mutateGlobal('phase5-playlists'), mutateGlobal('phase5-playback-state')])
      setPlaylistName('')
      setNotice(`Playlist “${name}” created with ${selectedMediaIds.length} video${selectedMediaIds.length === 1 ? '' : 's'}.`)
      onSelectionChange([])
    } catch (error) {
      setErrorMessage(typeof error === 'string' ? error : error instanceof Error ? error.message : String(error))
    } finally {
      setSavingPlaylist(false)
    }
  }

  async function addSelection(mode: 'append' | 'play_next' | 'play_now') {
    if (!selectedMediaIds.length) return
    setErrorMessage('')
    try {
      await queueAdd(selectedMediaIds, mode)
      setNotice(`${selectedMediaIds.length} video${selectedMediaIds.length === 1 ? '' : 's'} sent to the playback queue.`)
      onSelectionChange([])
    } catch (error) {
      setErrorMessage(typeof error === 'string' ? error : error instanceof Error ? error.message : String(error))
    }
  }

  return (
    <section className="media-library-page" aria-labelledby="library-title">
      <header className="library-page-heading">
        <div>
          <div className="section-eyebrow"><span className="eyebrow-line" /> YOUR COLLECTION</div>
          <h1 id="library-title">Media library<span className="title-period">.</span></h1>
          <p>Browse the videos stored on this device.</p>
        </div>
        <button className="button-primary add-folder-button" onClick={() => void folders.addFolders()} disabled={!isDesktop || folders.isAdding} title={!isDesktop ? 'Available in the installed Windows desktop app' : undefined}>
          {folders.isAdding ? <LoaderCircle size={17} className="spin-icon" /> : <FolderPlus size={17} />}
          {folders.isAdding ? 'Adding folders…' : 'Add folder'}
        </button>
      </header>

      {!isDesktop && <div className="library-runtime-note"><ShieldCheck size={16} /><span>Folder access and indexing are available in the installed Windows desktop app. This browser preview does not scan or simulate local files.</span></div>}
      {errorMessage && <div className="library-message error" role="alert"><AlertCircle size={16} />{errorMessage}</div>}
      {notice && <div className="library-message success" role="status"><Check size={16} />{notice}</div>}

      <div className="library-stats" aria-label="Library statistics">
        <div><span>VIDEOS</span><strong>{files.length}</strong></div>
        <div><span>FOLDERS</span><strong>{folders.directories.length}</strong></div>
        <div><span>INDEXED SIZE</span><strong>{formatBytes(totalBytes)}</strong></div>
        <div className="scan-state"><span>INDEX STATUS</span><strong><i className={Object.values(folders.scanProgress).some((progress) => progress.phase === 'scanning') ? 'scanning' : ''} />{Object.values(folders.scanProgress).some((progress) => progress.phase === 'scanning') ? 'Scanning folders' : 'Ready'}</strong></div>
      </div>

      <section className="folder-section" aria-labelledby="folders-title">
        <div className="library-subheading"><div><h2 id="folders-title">Library folders</h2><p>Choose which folders are indexed. Removing one never deletes your files.</p></div><span>{folders.directories.length} {folders.directories.length === 1 ? 'FOLDER' : 'FOLDERS'}</span></div>
        {folders.messages}
        <FolderList
          directories={folders.directories}
          isLoading={folders.isLoadingDirectories}
          scanProgress={folders.scanProgress}
          onScan={(directory) => void folders.scanDirectory(directory)}
          onCancel={(directory) => void folders.cancelScan(directory)}
          onRemove={(directory) => void folders.removeFolder(directory)}
          onAdd={() => void folders.addFolders()}
          addDisabled={!isDesktop}
        />
      </section>

      <section className="thumbnail-settings" aria-labelledby="thumbnail-settings-title">
        <div className="library-subheading">
          <div><h2 id="thumbnail-settings-title">Thumbnail cache</h2><p>Windows Shell previews are generated in the background and stored separately in LocalAppData.</p></div>
          <button className="folder-action cache-cleanup" onClick={cleanThumbnailCache} disabled={!isDesktop}>Clean unused cache</button>
        </div>
        <div className="thumbnail-settings-controls">
          <label htmlFor="thumbnail-size">Preview size</label>
          <select id="thumbnail-size" value={`${thumbnailSettings.width}x${thumbnailSettings.height}`} onChange={(event) => {
            const [width, height] = event.target.value.split('x').map(Number)
            void updateThumbnailSettings({ width, height })
          }} disabled={!isDesktop}>
            <option value="240x135">Small · 240 × 135</option>
            <option value="320x180">Medium · 320 × 180</option>
            <option value="480x270">Large · 480 × 270</option>
          </select>
          <label htmlFor="thumbnail-quality">JPEG quality</label>
          <select id="thumbnail-quality" value={thumbnailSettings.quality} onChange={(event) => void updateThumbnailSettings({ quality: Number(event.target.value) })} disabled={!isDesktop}>
            <option value="65">Compact · 65%</option>
            <option value="82">Balanced · 82%</option>
            <option value="92">High · 92%</option>
          </select>
        </div>
      </section>

      <section className="video-library-section" aria-labelledby="videos-title">
        <div className="video-library-heading"><div><h2 id="videos-title">All videos</h2><p>{search ? `Search results for “${search}”` : 'Your indexed video collection'}</p></div><div className="library-sort"><label htmlFor="extension-filter">Format</label><select id="extension-filter" value={extension} onChange={(event) => setExtension(event.target.value)}><option value="">All formats</option>{extensions.map((item) => <option key={item} value={item}>{item.toUpperCase()}</option>)}</select><label htmlFor="sort-order">Sort</label><select id="sort-order" value={sortBy} onChange={(event) => setSortBy(event.target.value as SortBy)}><option value="name">Name</option><option value="date">Date added</option><option value="size">File size</option></select></div></div>
        <div className="library-selection-bar" role="status">
          <span>{selectedMediaIds.length ? `${selectedMediaIds.length} selected` : 'Select videos using the checkbox on each card, or select a whole folder'}</span>
          <select className="selection-folder" value={folderFilter} onChange={(event) => setFolderFilter(event.target.value)} aria-label="Limit selection to one folder" disabled={!folderOptions.length}>
            <option value="">All folders</option>
            {folderOptions.map((folder) => <option key={folder} value={folder} title={folder}>{folder}</option>)}
          </select>
          <button onClick={selectAllVisible} disabled={!visibleFiles.length} title="Select every indexed video in the chosen folder">
            <Check size={14} /> {allVisibleSelected ? 'Unselect all' : `Select all (${visibleFiles.length})`}
          </button>
          <button onClick={() => void addSelection('play_now')} disabled={!selectedMediaIds.length || !castReady}><Play size={14} /> Play now</button>
          <button onClick={() => void addSelection('play_next')} disabled={!selectedMediaIds.length}>Play next</button>
          <button onClick={() => void addSelection('append')} disabled={!selectedMediaIds.length}>Add to queue</button>
          <input className="selection-playlist-name" value={playlistName} onChange={(event) => setPlaylistName(event.target.value)} placeholder="New playlist name" aria-label="New playlist name" maxLength={120} disabled={!isDesktop || savingPlaylist} />
          <button onClick={() => void saveSelectionAsPlaylist()} disabled={!selectedMediaIds.length || !playlistName.trim() || !isDesktop || savingPlaylist} title="Create a playlist from the selected videos"><ListMusic size={14} /> Save playlist</button>
          <span className="selection-hint">Playlists are stored locally; you can rename and play them in the Playlists panel.</span>
          {selectedMediaIds.length > 0 && <button className="selection-clear" onClick={() => onSelectionChange([])}><X size={14} /> Clear</button>}
        </div>
        {filesError ? <div className="library-empty-results"><AlertCircle size={20} /><strong>Could not load the library</strong><span>Try reopening the application. Your indexed files are stored on this device.</span></div> : isLoadingFiles ? <div className="library-loading"><LoaderCircle size={17} className="spin-icon" /> Loading videos…</div> : files.length ? <div className="media-grid">{files.map((media) => <MediaCard key={media.id} media={media} onRefresh={refreshThumbnail} onCast={castVideo} castReady={castReady} isCasting={castingMediaId === media.id} selected={selectedMediaIds.includes(media.id)} onToggleSelect={toggleSelect} />)}</div> : <div className="library-empty-results"><span className="empty-results-icon"><HardDrive size={22} /></span><strong>{search || extension ? 'No matching videos' : 'Your video collection is empty'}</strong><span>{search || extension ? 'Try a different search or format filter.' : 'Add a folder and scan it to see your videos here.'}</span>{!folders.directories.length && <button className="button-primary" onClick={() => void folders.addFolders()} disabled={!isDesktop}><FolderPlus size={16} /> Add media folder</button>}</div>}
      </section>
    </section>
  )
}
