import { useEffect, useMemo, useState } from 'react'
import useSWR from 'swr'
import { AlertCircle, Check, Film, LoaderCircle, Search, ShieldCheck } from 'lucide-react'
import { folderForPath, formatBytes, getMediaFiles, isDesktopApp, type MediaFile } from '../media'
import { debugLog } from '../diagnostics'

type Props = {
  selectedMediaIds: number[]
  onSelectionChange: (ids: number[]) => void
}

const PAGE_SIZE = 120

/** Compact indexed-video selector for the playlist step of first-run setup. */
export default function MediaPicker({ selectedMediaIds, onSelectionChange }: Props) {
  const isDesktop = isDesktopApp()
  const [search, setSearch] = useState('')
  const [folderFilter, setFolderFilter] = useState('')
  const [showAll, setShowAll] = useState(false)
  const key = useMemo(() => ['media-files', search, '', 'name'] as const, [search])
  const { data: files = [], isLoading, error } = useSWR(isDesktop ? key : null, getMediaFiles)

  const folderOptions = useMemo(
    () => Array.from(new Set(files.map((file) => folderForPath(file.path)))).sort((a, b) => a.localeCompare(b, undefined, { sensitivity: 'base' })),
    [files],
  )
  const visibleFiles = useMemo(
    () => folderFilter ? files.filter((file) => folderForPath(file.path) === folderFilter) : files,
    [files, folderFilter],
  )
  const renderedFiles = showAll ? visibleFiles : visibleFiles.slice(0, PAGE_SIZE)
  const allVisibleSelected = visibleFiles.length > 0 && visibleFiles.every((file) => selectedMediaIds.includes(file.id))

  useEffect(() => {
    if (folderFilter && !folderOptions.includes(folderFilter)) setFolderFilter('')
  }, [folderFilter, folderOptions])

  function toggle(file: MediaFile) {
    const next = selectedMediaIds.includes(file.id)
      ? selectedMediaIds.filter((id) => id !== file.id)
      : [...selectedMediaIds, file.id]
    debugLog('wizard', 'button_click', { control: 'toggle_indexed_video', mediaFileId: file.id, selected: next.includes(file.id) })
    onSelectionChange(next)
  }

  function selectAllVisible() {
    if (!visibleFiles.length) return
    if (allVisibleSelected) {
      const ids = new Set(visibleFiles.map((file) => file.id))
      onSelectionChange(selectedMediaIds.filter((id) => !ids.has(id)))
    } else {
      onSelectionChange(Array.from(new Set([...selectedMediaIds, ...visibleFiles.map((file) => file.id)])))
    }
    debugLog('wizard', 'button_click', {
      control: allVisibleSelected ? 'unselect_all_indexed_videos' : 'select_all_indexed_videos',
      folder: folderFilter || null,
      count: visibleFiles.length,
    })
  }

  if (!isDesktop) {
    return <div className="setup-note"><ShieldCheck size={17} /><span>Indexed videos are available in the installed Windows desktop app.</span></div>
  }

  return (
    <div className="wizard-media-picker">
      <div className="wizard-picker-head">
        <label className="wizard-picker-search">
          <Search size={14} />
          <input value={search} onChange={(event) => setSearch(event.target.value)} placeholder="Search indexed videos" aria-label="Search indexed videos" />
        </label>
        <span>{selectedMediaIds.length ? `${selectedMediaIds.length} selected` : 'Select videos to add to a playlist'}</span>
      </div>
      <div className="wizard-picker-actions">
        <select value={folderFilter} onChange={(event) => { setFolderFilter(event.target.value); setShowAll(false) }} aria-label="Limit selection to one folder" disabled={!folderOptions.length}>
          <option value="">All folders</option>
          {folderOptions.map((folder) => <option key={folder} value={folder} title={folder}>{folder}</option>)}
        </select>
        <button type="button" onClick={selectAllVisible} disabled={!visibleFiles.length}>
          <Check size={13} /> {allVisibleSelected ? 'Unselect all' : `Select all (${visibleFiles.length})`}
        </button>
      </div>
      {error && <div className="library-message error" role="alert"><AlertCircle size={16} />Could not load indexed videos: {String(error)}</div>}
      {isLoading ? <div className="library-loading"><LoaderCircle size={17} className="spin-icon" /> Loading indexed videos…</div> : visibleFiles.length ? (
        <div className="wizard-media-list">
          {renderedFiles.map((file) => {
            const selected = selectedMediaIds.includes(file.id)
            return (
              <label key={file.id} className={`wizard-media-row ${selected ? 'selected' : ''}`}>
                <input type="checkbox" checked={selected} onChange={() => toggle(file)} />
                <span className="wizard-media-check">{selected ? <Check size={12} /> : null}</span>
                <Film size={14} />
                <span className="wizard-media-name" title={file.filename}>{file.filename}</span>
                <small>{file.extension.toUpperCase()} · {formatBytes(file.size_bytes)}</small>
              </label>
            )
          })}
          {visibleFiles.length > PAGE_SIZE && <button type="button" className="wizard-picker-more" onClick={() => setShowAll((value) => !value)}>
            {showAll ? `Show first ${PAGE_SIZE}` : `Show all ${visibleFiles.length} videos`}
          </button>}
        </div>
      ) : (
        <div className="playlist-empty"><Film size={16} />{search || folderFilter ? 'No indexed videos match this folder or search.' : 'No indexed videos yet. Add and scan a media folder first.'}</div>
      )}
    </div>
  )
}
