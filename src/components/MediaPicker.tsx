import { useMemo, useState } from 'react'
import useSWR from 'swr'
import { AlertCircle, Check, Film, LoaderCircle, Search, ShieldCheck } from 'lucide-react'
import { formatBytes, getMediaFiles, isDesktopApp, type MediaFile } from '../media'
import { debugLog } from '../diagnostics'

type Props = {
  selectedMediaIds: number[]
  onSelectionChange: (ids: number[]) => void
}

/** Compact indexed-video selector for the playlist step of first-run setup. */
export default function MediaPicker({ selectedMediaIds, onSelectionChange }: Props) {
  const isDesktop = isDesktopApp()
  const [search, setSearch] = useState('')
  const key = useMemo(() => ['media-files', search, '', 'name'] as const, [search])
  const { data: files = [], isLoading, error } = useSWR(isDesktop ? key : null, getMediaFiles)

  function toggle(file: MediaFile) {
    const next = selectedMediaIds.includes(file.id)
      ? selectedMediaIds.filter((id) => id !== file.id)
      : [...selectedMediaIds, file.id]
    debugLog('wizard', 'button_click', { control: 'toggle_indexed_video', mediaFileId: file.id, selected: next.includes(file.id) })
    onSelectionChange(next)
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
      {error && <div className="library-message error" role="alert"><AlertCircle size={16} />Could not load indexed videos: {String(error)}</div>}
      {isLoading ? <div className="library-loading"><LoaderCircle size={17} className="spin-icon" /> Loading indexed videos…</div> : files.length ? (
        <div className="wizard-media-list">
          {files.slice(0, 120).map((file) => {
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
          {files.length > 120 && <div className="playlist-empty">Showing the first 120 videos. Use search to narrow the list.</div>}
        </div>
      ) : (
        <div className="playlist-empty"><Film size={16} />{search ? 'No indexed videos match your search.' : 'No indexed videos yet. Add and scan a media folder first.'}</div>
      )}
    </div>
  )
}
