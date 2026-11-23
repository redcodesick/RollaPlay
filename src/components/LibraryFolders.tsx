import { useCallback, useEffect, useState } from 'react'
import useSWR, { useSWRConfig } from 'swr'
import { listen } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-dialog'
import { AlertCircle, Check, FolderOpen, FolderPlus, LoaderCircle, RefreshCw, Trash2, X } from 'lucide-react'
import {
  getMediaDirectories, isDesktopApp,
  type MediaDirectory, type ScanProgress, type ScanResult,
} from '../media'
import { debugLog, diagnosticError, friendlyError, invokeTracked } from '../diagnostics'

export function FolderRow({
  directory, progress, onScan, onCancel, onRemove,
}: {
  directory: MediaDirectory
  progress?: ScanProgress
  onScan: (directory: MediaDirectory) => void
  onCancel: (directory: MediaDirectory) => void
  onRemove: (directory: MediaDirectory) => void
}) {
  const scanning = progress?.phase === 'scanning'
  const folderStatus = directory.availability_state === 'temporarily_unavailable'
    ? 'Drive or folder unavailable · indexed items and playlists are retained'
    : directory.availability_state === 'partially_unavailable'
      ? 'Some locations inaccessible · previous entries were preserved'
      : directory.last_scanned_at
        ? `Last scanned ${new Date(`${directory.last_scanned_at.replace(' ', 'T')}Z`).toLocaleString()}`
        : 'Not scanned yet'
  return (
    <div className={`library-folder-row ${directory.availability_state !== 'available' ? 'folder-offline' : ''}`}>
      <span className="folder-row-icon"><FolderOpen size={17} /></span>
      <span className="folder-row-info"><strong title={directory.path}>{directory.path}</strong><small>{scanning ? `Scanning · ${progress.files_found} videos found · ${progress.inaccessible_entries} inaccessible items` : folderStatus}</small></span>
      {scanning ? <><span className="folder-scanning"><LoaderCircle size={15} /> Scanning</span><button className="folder-cancel" onClick={() => onCancel(directory)} aria-label={`Cancel scan of ${directory.path}`}><X size={15} /> Cancel</button></> : <button className="folder-action" onClick={() => onScan(directory)} aria-label={`${directory.availability_state !== 'available' ? 'Retry scan' : 'Rescan'} ${directory.path}`}><RefreshCw size={15} /> {directory.availability_state !== 'available' ? 'Retry' : 'Scan'}</button>}
      <button className="folder-remove" onClick={() => onRemove(directory)} disabled={scanning} aria-label={`Remove ${directory.path} from library`}><Trash2 size={15} /></button>
    </div>
  )
}

export function FolderList({
  directories, isLoading, scanProgress, onScan, onCancel, onRemove, onAdd, addDisabled,
}: {
  directories: MediaDirectory[]
  isLoading: boolean
  scanProgress: Record<number, ScanProgress>
  onScan: (directory: MediaDirectory) => void
  onCancel: (directory: MediaDirectory) => void
  onRemove: (directory: MediaDirectory) => void
  onAdd?: () => void
  addDisabled?: boolean
}) {
  if (isLoading) return <div className="library-loading"><LoaderCircle size={17} className="spin-icon" /> Loading folders…</div>
  if (!directories.length) return <div className="library-empty-folders"><FolderPlus size={19} /><span>No media folders yet.</span>{onAdd && <button onClick={onAdd} disabled={addDisabled}>Add a folder to begin</button>}</div>
  return (
    <div className="library-folder-list">
      {directories.map((directory) => (
        <FolderRow
          key={directory.id}
          directory={directory}
          progress={scanProgress[directory.id]}
          onScan={onScan}
          onCancel={onCancel}
          onRemove={onRemove}
        />
      ))}
    </div>
  )
}

/**
 * Shared media-folder state and actions. Used by both the Media library page
 * and the first-run wizard so there is only one folder/indexing implementation.
 */
export function useMediaFolders() {
  const isDesktop = isDesktopApp()
  const { mutate: globalMutate } = useSWRConfig()
  const {
    data: directories = [],
    isLoading: isLoadingDirectories,
    mutate: refreshDirectories,
  } = useSWR(isDesktop ? 'media-directories' : null, getMediaDirectories)
  const [scanProgress, setScanProgress] = useState<Record<number, ScanProgress>>({})
  const [isAdding, setIsAdding] = useState(false)
  const [notice, setNotice] = useState('')
  const [error, setError] = useState('')

  const refreshFiles = useCallback(async () => {
    await globalMutate((key: unknown) => Array.isArray(key) && key[0] === 'media-files')
  }, [globalMutate])

  useEffect(() => {
    if (!isDesktop) return
    let disposed = false
    let stopListening: (() => void) | undefined
    listen<ScanProgress>('media-scan-progress', (event) => {
      setScanProgress((current) => ({ ...current, [event.payload.directory_id]: event.payload }))
      if (event.payload.phase !== 'scanning') {
        debugLog('media-folders', 'index_progress', {
          directoryId: event.payload.directory_id,
          phase: event.payload.phase,
          filesFound: event.payload.files_found,
          inaccessibleEntries: event.payload.inaccessible_entries,
        })
        void refreshFiles()
        void refreshDirectories()
      }
    }).then((stop) => {
      if (disposed) stop()
      else stopListening = stop
    }).catch((listenError) => {
      diagnosticError('media-folders', 'progress_listener_failed', listenError)
      setError('Could not listen for folder indexing progress. Rescan the folder to refresh its status.')
    })
    return () => {
      disposed = true
      stopListening?.()
    }
  }, [isDesktop, refreshDirectories, refreshFiles])

  const scanDirectory = useCallback(async (directory: MediaDirectory) => {
    setError('')
    setNotice('')
    setScanProgress((current) => ({
      ...current,
      [directory.id]: { directory_id: directory.id, files_found: 0, inaccessible_entries: 0, phase: 'scanning' },
    }))
    debugLog('media-folders', 'index_start', { directoryId: directory.id })
    try {
      const result = await invokeTracked<ScanResult>('scan_media_directory', { directoryId: directory.id }, 'media-folders')
      await Promise.all([refreshFiles(), refreshDirectories()])
      const summary = result.cancelled
        ? 'Scan cancelled. No partial index changes were saved.'
        : `${result.files_indexed} videos indexed${result.renamed_files ? ` · ${result.renamed_files} renamed files matched` : ''}${result.files_removed ? ` · ${result.files_removed} permanently missing` : ''}${result.temporarily_unavailable_files ? ` · ${result.temporarily_unavailable_files} temporarily unavailable` : ''}${result.inaccessible_entries ? ` · ${result.inaccessible_entries} inaccessible locations` : ''}${result.playlists_imported ? ` · ${result.playlists_imported} M3U/WPL playlist${result.playlists_imported === 1 ? '' : 's'} imported` : ''}.`
      debugLog('media-folders', 'index_complete', {
        directoryId: directory.id,
        filesIndexed: result.files_indexed,
        cancelled: result.cancelled,
        inaccessibleEntries: result.inaccessible_entries,
      })
      setNotice(summary)
      setScanProgress((current) => ({
        ...current,
        [directory.id]: {
          directory_id: directory.id,
          files_found: result.files_indexed,
          inaccessible_entries: result.inaccessible_entries,
          phase: result.cancelled ? 'cancelled' : 'complete',
        },
      }))
    } catch (scanError) {
      await refreshDirectories()
      diagnosticError('media-folders', 'index_failed', scanError, { directoryId: directory.id })
      setScanProgress((current) => ({
        ...current,
        [directory.id]: { directory_id: directory.id, files_found: 0, inaccessible_entries: 0, phase: 'temporarily_unavailable' },
      }))
      setError(friendlyError(scanError))
    }
  }, [refreshDirectories, refreshFiles])

  const addFolders = useCallback(async () => {
    if (!isDesktop) return
    setIsAdding(true)
    setError('')
    setNotice('')
    debugLog('media-folders', 'picker_invoke', { multiple: true, directory: true })
    try {
      const selection = await open({ directory: true, multiple: true, title: 'Choose folders containing videos' })
      if (!selection || (Array.isArray(selection) && selection.length === 0)) {
        debugLog('media-folders', 'picker_cancelled')
        setNotice('Folder selection cancelled.')
        return
      }
      const paths = Array.isArray(selection) ? selection : [selection]
      debugLog('media-folders', 'picker_response', { count: paths.length, paths })
      let added = 0
      for (const path of paths) {
        const directory = await invokeTracked<MediaDirectory>('add_media_directory', { path }, 'media-folders')
        added += 1
        await refreshDirectories()
        await scanDirectory(directory)
      }
      await refreshFiles()
      setNotice(`${added} folder${added === 1 ? '' : 's'} added${added === 1 ? '' : ' and indexing started'}.`)
    } catch (pickerError) {
      diagnosticError('media-folders', 'add_folders_failed', pickerError)
      setError(friendlyError(pickerError))
    } finally {
      setIsAdding(false)
    }
  }, [isDesktop, refreshDirectories, refreshFiles, scanDirectory])

  const cancelScan = useCallback(async (directory: MediaDirectory) => {
    debugLog('media-folders', 'index_cancel_request', { directoryId: directory.id })
    try {
      const cancelled = await invokeTracked<boolean>('cancel_media_scan', { directoryId: directory.id }, 'media-folders')
      setNotice(cancelled ? 'Cancelling scan. No partial index changes will be saved.' : 'The scan had already finished.')
    } catch (cancelError) {
      diagnosticError('media-folders', 'index_cancel_failed', cancelError, { directoryId: directory.id })
      setError(friendlyError(cancelError))
    }
  }, [])

  const removeFolder = useCallback(async (directory: MediaDirectory) => {
    const confirmed = window.confirm(`Remove this folder from your library?\n\n${directory.path}\n\nYour video files will not be deleted.`)
    if (!confirmed) {
      debugLog('media-folders', 'remove_cancelled', { directoryId: directory.id })
      return
    }
    setError('')
    debugLog('media-folders', 'remove_request', { directoryId: directory.id })
    try {
      await invokeTracked<void>('remove_media_directory', { directoryId: directory.id }, 'media-folders')
      setScanProgress((current) => {
        const next = { ...current }
        delete next[directory.id]
        return next
      })
      await Promise.all([refreshDirectories(), refreshFiles()])
      debugLog('media-folders', 'remove_complete', { directoryId: directory.id })
      setNotice('Folder removed from the library. Your files were not changed.')
    } catch (removeError) {
      diagnosticError('media-folders', 'remove_failed', removeError, { directoryId: directory.id })
      setError(friendlyError(removeError))
    }
  }, [refreshDirectories, refreshFiles])

  const clearMessages = useCallback(() => {
    setNotice('')
    setError('')
  }, [])

  return {
    isDesktop,
    directories,
    isLoadingDirectories,
    refreshDirectories,
    refreshFiles,
    scanProgress,
    isAdding,
    notice,
    error,
    addFolders,
    scanDirectory,
    cancelScan,
    removeFolder,
    clearMessages,
    messages: (
      <>
        {error && <div className="library-message error" role="alert"><AlertCircle size={16} />{error}</div>}
        {notice && <div className="library-message success" role="status"><Check size={16} />{notice}</div>}
      </>
    ),
  }
}
