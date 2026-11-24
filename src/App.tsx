import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import useSWR from 'swr'
import { listen } from '@tauri-apps/api/event'
import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import { getName, getTauriVersion, getVersion } from '@tauri-apps/api/app'
import { getCurrentWindow } from '@tauri-apps/api/window'
import MediaLibrary from './components/MediaLibrary'
import MediaServerSettings from './components/MediaServerSettings'
import Downloader from './components/Downloader'
import CastPanel from './components/CastPanel'
import ConnectPanel from './components/ConnectPanel'
import QueuePanel from './components/QueuePanel'
import { FolderList, useMediaFolders } from './components/LibraryFolders'
import MediaPicker from './components/MediaPicker'
import PlaylistManager from './components/PlaylistManager'
import PlayerBar from './components/PlayerBar'
import { CastCheck, ChromeCheck, NetworkCheck } from './components/SetupChecks'
import {
  castDeviceName, castSendControl, clickCastSender, embedCastSender, formatSeconds,
  getCastPlaybackState, hideCastSender, isCastAvailable, launchCastSender, onCastPlaybackEvent,
  showCastSender, type SenderHostRect,
} from './cast'
import {
  clearWatchHistory, formatDuration, getPlaybackState, isPlaylistAvailable, listPlaylists,
  playMediaNow, playNext, playPlaylist, playPrevious, type Phase5Notification,
  type PlaybackSnapshot, type PlaylistSummary,
} from './playlist'
import { formatBytes, getLocalStatus, getMediaDirectories, getMediaFiles, isDesktopApp, type MediaFile } from './media'
import { debugLog } from './diagnostics'
import {
  AlertCircle, Cast, Check, ChevronDown, ChevronsLeft, ChevronsRight, CircleHelp,
  Clapperboard, Clock3, Download, Film, FolderPlus, Gauge, HardDrive, History, House, Info, Library,
  ListMusic, Play, Plus, Search, Settings2, ShieldCheck,
  Sparkles, Trash2, Wifi, X,
} from 'lucide-react'
import { BrandMark, NoticeIcon } from './ui'

type View = 'Home' | 'Library' | 'Playlists' | 'Downloader' | 'Recently added' | 'History' | 'Settings'
type Notice = (message: string, level?: Phase5Notification['level']) => void

const navItems: { label: View; icon: typeof House }[] = [
  { label: 'Home', icon: House },
  { label: 'Library', icon: Library },
  { label: 'Playlists', icon: ListMusic },
  { label: 'Recently added', icon: Clock3 },
  { label: 'History', icon: History },
  { label: 'Downloader', icon: Download },
]

function Sidebar({
  active, playlistCount, collapsed, onToggle, onNavigate, onSetup, onAbout,
}: {
  active: View
  playlistCount: number
  collapsed: boolean
  onToggle: () => void
  onNavigate: (view: View) => void
  onSetup: () => void
  onAbout: () => void
}) {
  return (
    <aside className={`sidebar ${collapsed ? 'collapsed' : ''}`}>
      <a className="brand" href="#home" aria-label="RollaPlay home" title="RollaPlay home" onClick={(event) => { event.preventDefault(); onNavigate('Home') }}>
        <BrandMark size={38} />
        <span className="brand-name"><b>ROLLA</b>PLAY<span>ROCKNROLLA MEDIA STATION</span></span>
      </a>
      <div className="nav-label">YOUR SPACE</div>
      <nav className="primary-nav" aria-label="Main navigation">
        {navItems.map(({ label, icon: Icon }) => (
          <button key={label} className={`nav-item ${active === label ? 'active' : ''}`} onClick={() => onNavigate(label)} title={label} aria-current={active === label ? 'page' : undefined}>
            <Icon size={19} strokeWidth={1.8} /><span>{label}</span>{label === 'Playlists' && <span className="nav-count">{playlistCount}</span>}
          </button>
        ))}
      </nav>
      <div className="sidebar-divider" />
      <button className={`nav-item ${active === 'Settings' ? 'active' : ''}`} onClick={() => onNavigate('Settings')} title="Settings" aria-current={active === 'Settings' ? 'page' : undefined}><Settings2 size={19} strokeWidth={1.8} /><span>Settings</span></button>
      <div className="sidebar-spacer" />
      <div className="local-card">
        <div className="local-card-icon"><HardDrive size={17} /></div>
        <div className="local-copy"><strong>Everything stays local</strong><span>Your files never leave this PC.</span></div>
        <ShieldCheck size={16} className="shield-icon" />
      </div>
      <button className="help-link" onClick={onSetup} title="Setup guide"><CircleHelp size={17} /> <span>Setup guide</span></button>
      <button className="help-link" onClick={onAbout} title="About RollaPlay"><Info size={17} /> <span>About RollaPlay</span></button>
      <button className="sidebar-collapse" onClick={onToggle} title={collapsed ? 'Expand sidebar' : 'Collapse sidebar'} aria-label={collapsed ? 'Expand sidebar' : 'Collapse sidebar'} aria-expanded={!collapsed}>
        {collapsed ? <ChevronsRight size={18} /> : <><ChevronsLeft size={18} /><span>Collapse</span></>}
      </button>
      <div className="profile-row"><span className="profile-avatar"><BrandMark size={20} /></span><div><strong>RollaPlay Station</strong><span>Local device</span></div></div>
    </aside>
  )
}

function EmptyLibrary({ onSetup, onBrowse }: { onSetup: () => void; onBrowse: () => void }) {
  return (
    <section className="empty-library" aria-labelledby="empty-title">
      <div className="empty-illustration" aria-hidden="true"><div className="empty-disc"><Film size={30} /></div><span className="orbit orbit-one" /><span className="orbit orbit-two" /><span className="spark spark-a">✦</span><span className="spark spark-b">✧</span></div>
      <div className="empty-kicker"><span /> YOUR LIBRARY IS READY</div>
      <h2 id="empty-title">Your next movie night<br />starts <em>right here.</em></h2>
      <p>Add a folder with your videos to build a private library on this device. Your files stay exactly where they are.</p>
      <div className="empty-actions"><button className="button-primary" onClick={onBrowse}><FolderPlus size={18} /> Add media folder</button><button className="button-text" onClick={onSetup}>Walk me through setup <span>→</span></button></div>
      <div className="format-note"><span>SUPPORTS</span><b>MP4</b><i /> <b>MKV</b><i /> <b>MOV</b><i /> <b>AVI</b><i /> <b>AND MORE</b></div>
    </section>
  )
}

function RecentGrid({ files, limit = 6, onPlay }: { files: MediaFile[]; limit?: number; onPlay: (id: number) => void }) {
  return (
    <div className="recent-grid">
      {files.slice(0, limit).map((media) => <button
        key={media.id}
        className="recent-card"
        title={`Play ${media.filename}`}
        onClick={() => onPlay(media.id)}
      >
        <span className="recent-art">{media.thumbnail_status === 'ready' && media.thumbnail_path
          ? <img src={convertFileSrc(media.thumbnail_path)} alt="" />
          : <Film size={18} />}</span>
        <strong>{media.filename}</strong>
        <small>{formatBytes(media.size_bytes)}</small>
      </button>)}
    </div>
  )
}

function HomeSpotlight({
  playlists, recentFiles, onPlayPlaylist, onPlayMedia, onOpenLibrary, onOpenDownloader,
}: {
  playlists: PlaylistSummary[] | undefined
  recentFiles: MediaFile[] | undefined
  onPlayPlaylist: (id: number) => void
  onPlayMedia: (id: number) => void
  onOpenLibrary: () => void
  onOpenDownloader: () => void
}) {
  const playlist = playlists?.[0]
  const latest = recentFiles?.[0]
  const continueTitle = playlist?.name ?? latest?.filename ?? null
  const detail = playlist
    ? `${playlist.item_count} video${playlist.item_count === 1 ? '' : 's'} · ${formatDuration(playlist.duration_seconds)} · ready for your TV.`
    : latest
      ? `${formatBytes(latest.size_bytes)} · the newest addition to your collection.`
      : 'Add a folder with your local videos and RollaPlay will index it here.'
  const continuePlaying = () => {
    if (playlist) onPlayPlaylist(playlist.id)
    else if (latest) onPlayMedia(latest.id)
  }
  return (
    <section className="featured-card">
      <img src="/media/featured-still.png" alt="" />
      <div className="feature-overlay" />
      <div className="feature-content">
        <div className="feature-tag"><span /> YOUR MUSIC. YOUR RULES.</div>
        <h2>YOUR MUSIC.<br />YOUR RULES.</h2>
        <p>{detail}</p>
        <div className="feature-actions">
          <button className="feature-button" onClick={onOpenLibrary}><Library size={17} /> Browse Library <span>→</span></button>
          <button className="feature-secondary" onClick={onOpenDownloader}><Download size={16} /> Open Downloader</button>
          {continueTitle && <button className="feature-secondary" onClick={continuePlaying}><Play size={15} /> Continue playing · {continueTitle}</button>}
        </div>
      </div>
    </section>
  )
}

const homePlaylistTones = ['tone-red', 'tone-amber', 'tone-paper', 'tone-steel']

function HomePlaylistShelf({
  playlists, onPlay, onOpenAll,
}: {
  playlists: PlaylistSummary[] | undefined
  onPlay: (id: number) => void
  onOpenAll: () => void
}) {
  if (!playlists?.length) return null
  return (
    <div className="library-summary home-playlists">
      <div className="summary-title">
        <div><div className="section-eyebrow">YOUR COLLECTION</div><h2>Your playlists</h2></div>
        <button className="text-link" onClick={onOpenAll}>See all <span>→</span></button>
      </div>
      <div className="home-playlist-grid">
        {playlists.slice(0, 4).map((playlist, index) => <div className="home-playlist-card" key={playlist.id}>
          <button
            className={`playlist-cover ${homePlaylistTones[index % homePlaylistTones.length]}`}
            onClick={() => onPlay(playlist.id)}
            aria-label={`Play ${playlist.name}`}
            title={`Play ${playlist.name}`}
          ><Play size={18} fill="currentColor" /></button>
          <button className="home-playlist-name" onClick={onOpenAll}>
            <strong title={playlist.name}>{playlist.name}</strong>
            <small>{playlist.item_count} video{playlist.item_count === 1 ? '' : 's'}</small>
          </button>
        </div>)}
      </div>
    </div>
  )
}

type SetupDialogProps = {
  step: number
  setStep: (n: number) => void
  onClose: () => void
  onOpenLibrary: () => void
  onNotice: Notice
  selectedMediaIds: number[]
  onSelectionChange: (ids: number[]) => void
  snapshot: PlaybackSnapshot | null
}

function SetupDialog({
  step, setStep, onClose, onOpenLibrary, onNotice, selectedMediaIds, onSelectionChange, snapshot,
}: SetupDialogProps) {
  const steps = ['Welcome', 'Media folders', 'Scan', 'Cast devices', 'All set']
  const isDesktop = isDesktopApp()
  const folders = useMediaFolders()
  const [skippedFolders, setSkippedFolders] = useState(false)
  const [showAdvanced, setShowAdvanced] = useState(false)
  const lastStep = steps.length - 1
  const optionalSteps = new Set([2, 3])

  useEffect(() => {
    debugLog('wizard', 'step_change', { step, label: steps[step] })
  }, [step])
  useEffect(() => {
    if (folders.directories.length) setSkippedFolders(false)
  }, [folders.directories.length])

  const click = useCallback((control: string, details?: Record<string, unknown>) => {
    debugLog('wizard', 'button_click', { control, step, label: steps[step], ...details })
  }, [step])

  const hasFolders = folders.directories.length > 0
  const canLeaveFolders = hasFolders || skippedFolders

  function advance() {
    if (step === lastStep) {
      click('open_media_library')
      onOpenLibrary()
      return
    }
    if (step === 1 && !canLeaveFolders) {
      click('continue_blocked', { reason: 'no_media_folders' })
      onNotice('Add at least one media folder or choose “Skip folders for now” first.', 'warning')
      return
    }
    click('continue')
    setStep(step + 1)
  }

  return (
    <div className="dialog-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget) { click('close_backdrop'); onClose() } }}>
      <section className={`setup-dialog ${step > 0 && step < lastStep ? 'setup-dialog-wide' : ''}`} role="dialog" aria-modal="true" aria-labelledby="setup-title">
        <button className="dialog-close" onClick={() => { click('close'); onClose() }} aria-label="Close setup">×</button>
        <div className="setup-eyebrow"><Sparkles size={15} /> FIRST-RUN SETUP <span className="setup-step-count">STEP {step + 1} OF {steps.length}</span></div>
        <div className="step-progress">{steps.map((item, index) => <span key={item} className={index <= step ? 'complete' : ''} />)}</div>

        {step === 0 && <>
          <div className="setup-icon"><Clapperboard size={25} /></div>
          <h2 id="setup-title">Welcome to your<br /><em>personal media room.</em></h2>
          <p>Your Windows PC will keep your video library and stream original files directly to your TV over your home network. Nothing is uploaded to the cloud.</p>
          <div className="setup-note"><ShieldCheck size={17} /><span><strong>Private by design</strong> — your library and playlists stay on this device.</span></div>
        </>}

        {step === 1 && <>
          <div className="setup-icon"><FolderPlus size={25} /></div>
          <h2 id="setup-title">Choose your<br /><em>media folders.</em></h2>
          <p>Select one or more local or removable folders. Each folder is added to the existing media library and indexed recursively. Removing a folder never deletes your files.</p>
          <button className="folder-picker" onClick={() => { click('select_media_folder'); void folders.addFolders() }} disabled={!isDesktop || folders.isAdding}>
            <FolderPlus size={20} />
            <span><strong>{folders.isAdding ? 'Adding folders…' : 'Select media folders'}</strong><small>{isDesktop ? 'Open the native Windows folder picker. You can choose multiple folders.' : 'Folder access is available in the installed Windows app.'}</small></span>
            <span className="picker-arrow">→</span>
          </button>
          {folders.messages}
          <FolderList
            directories={folders.directories}
            isLoading={folders.isLoadingDirectories}
            scanProgress={folders.scanProgress}
            onScan={(directory) => { click('rescan_folder', { directoryId: directory.id }); void folders.scanDirectory(directory) }}
            onCancel={(directory) => { click('cancel_scan', { directoryId: directory.id }); void folders.cancelScan(directory) }}
            onRemove={(directory) => { click('remove_folder', { directoryId: directory.id }); void folders.removeFolder(directory) }}
            onAdd={() => { click('select_media_folder_empty'); void folders.addFolders() }}
            addDisabled={!isDesktop || folders.isAdding}
          />
          {!hasFolders && !skippedFolders && <div className="setup-note warning"><AlertCircle size={17} /><span>Add at least one folder to continue, or skip this step and add folders later from the Media library.</span></div>}
          {skippedFolders && <div className="setup-note"><ShieldCheck size={17} /><span>Folders skipped. You can add them later from the Media library or Settings.</span></div>}
          {!hasFolders && <button className="button-text wizard-skip-inline" onClick={() => { click('skip_folders'); setSkippedFolders(true) }}>Skip folders for now</button>}
        </>}

        {step === 2 && <>
          <div className="setup-icon"><HardDrive size={25} /></div>
          <h2 id="setup-title">Scan your<br /><em>library.</em></h2>
          <p>RollaPlay indexes selected folders in the background and generates local previews. Nothing is uploaded.</p>
          <FolderList
            directories={folders.directories}
            isLoading={folders.isLoadingDirectories}
            scanProgress={folders.scanProgress}
            onScan={(directory) => void folders.scanDirectory(directory)}
            onCancel={(directory) => void folders.cancelScan(directory)}
            onRemove={(directory) => void folders.removeFolder(directory)}
            onAdd={() => void folders.addFolders()}
            addDisabled={!isDesktop || folders.isAdding}
          />
          <button className="button-text wizard-advanced-toggle" onClick={() => { click('toggle_advanced'); setShowAdvanced(!showAdvanced) }} aria-expanded={showAdvanced}>
            {showAdvanced ? 'Hide optional checks' : 'Show optional checks'} <span>→</span>
          </button>
          {showAdvanced && <div className="wizard-advanced">
            <div className="setup-note"><ShieldCheck size={17} /><span>These checks are optional. You can return to them any time from the Setup guide.</span></div>
            <div className="wizard-advanced-block"><div className="download-label">Playlists</div><MediaPicker selectedMediaIds={selectedMediaIds} onSelectionChange={onSelectionChange} /><PlaylistManager snapshot={snapshot} selectedMediaIds={selectedMediaIds} onNotice={onNotice} onSelectionConsumed={() => onSelectionChange([])} compact /></div>
            <div className="wizard-advanced-block"><div className="download-label">Google Chrome</div><ChromeCheck onNotice={onNotice} /></div>
            <div className="wizard-advanced-block"><div className="download-label">Local network</div><NetworkCheck onNotice={onNotice} /></div>
          </div>}
        </>}

        {step === 3 && <>
          <div className="setup-icon"><Cast size={25} /></div>
          <h2 id="setup-title">Find your<br /><em>Cast device.</em></h2>
          <p>Real receiver discovery requires Chrome and a Cast device on the same LAN. Starting discovery opens the managed sender panel in this window.</p>
          <CastCheck onNotice={onNotice} />
        </>}

        {step === lastStep && <>
          <div className="setup-icon"><Check size={25} /></div>
          <h2 id="setup-title">Your media room is<br /><em>ready to explore.</em></h2>
          <p>Your folders, indexed videos, playlists and queue are stored locally. You can return to the Setup guide at any time from the sidebar.</p>
          <div className="setup-note"><ShieldCheck size={17} /><span><strong>Nothing is uploaded.</strong> Cast playback streams original files from this PC over your private LAN.</span></div>
        </>}

        <div className="setup-footer">
          <button className="button-text" onClick={() => {
            if (step === 0) { click('not_now'); onClose(); return }
            click('back')
            setStep(step - 1)
          }}>{step === 0 ? 'Not now' : 'Back'}</button>
          <div className="setup-footer-right">
            {optionalSteps.has(step) && <button className="button-text" onClick={() => { click('skip_step'); setStep(step + 1) }}>Skip for now</button>}
            <button className="button-primary" onClick={advance}>{step === lastStep ? 'Open media library' : 'Continue'} <span>→</span></button>
          </div>
        </div>
      </section>
    </div>
  )
}

function AboutDialog({
  onClose, onOpenSetup, folderCount, videoCount, playlistCount,
}: {
  onClose: () => void
  onOpenSetup: () => void
  folderCount: number
  videoCount: number
  playlistCount: number
}) {
  const [info, setInfo] = useState({ name: 'RollaPlay', version: '0.1.0', tauri: '—' })
  useEffect(() => {
    if (!isDesktopApp()) return
    let active = true
    void Promise.all([getName(), getVersion(), getTauriVersion()])
      .then(([name, version, tauri]) => { if (active) setInfo({ name, version, tauri }) })
      .catch(() => { /* Keep the packaged defaults when the host API is unavailable. */ })
    return () => { active = false }
  }, [])
  return (
    <div className="dialog-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose() }}>
      <section className="about-dialog" role="dialog" aria-modal="true" aria-labelledby="about-title">
        <button className="dialog-close" onClick={onClose} aria-label="Close about">×</button>
        <div className="setup-eyebrow"><BrandMark size={16} /> ABOUT THIS APP</div>
        <div className="about-brand">
          <span className="about-mark"><BrandMark size={38} title /></span>
          <div><h2 id="about-title">{info.name}</h2><p>Local media, loud attitude.</p></div>
        </div>
        <p className="about-lede">RollaPlay keeps your video library on this PC and streams original files straight to your TV. No account, no uploads, no subscriptions.</p>
        <dl className="about-facts">
          <div><dt>Version</dt><dd>{info.version}</dd></div>
          <div><dt>Tauri runtime</dt><dd>{info.tauri}</dd></div>
          <div><dt>Platform</dt><dd>Windows desktop</dd></div>
          <div><dt>Media folders</dt><dd>{folderCount}</dd></div>
          <div><dt>Indexed videos</dt><dd>{videoCount}</dd></div>
          <div><dt>Saved playlists</dt><dd>{playlistCount}</dd></div>
        </dl>
        <div className="about-note"><ShieldCheck size={16} /><span>Metadata, playlists and settings stay on this device. Casting uses Google Chrome and your home network only.</span></div>
        <div className="setup-footer"><button className="button-text" onClick={onOpenSetup}>Open setup guide <span>→</span></button><button className="button-primary" onClick={onClose}>Done</button></div>
      </section>
    </div>
  )
}

function ExitConfirmDialog({ onCancel, onConfirm }: { onCancel: () => void; onConfirm: () => void }) {
  return (
    <div className="dialog-backdrop" role="presentation">
      <section className="confirm-dialog" role="alertdialog" aria-modal="true" aria-labelledby="exit-title" aria-describedby="exit-copy">
        <div className="setup-icon"><AlertCircle size={25} /></div>
        <h2 id="exit-title">Are you sure you want to exit?</h2>
        <p id="exit-copy">Receiver playback will stop and the Cast connection will close. Your library, playlists and settings are saved on this device.</p>
        <div className="setup-footer">
          <button className="button-text" onClick={onCancel}>Cancel</button>
          <button className="button-primary" onClick={onConfirm}>Exit RollaPlay <span>→</span></button>
        </div>
      </section>
    </div>
  )
}

function SettingsView({
  onManage, onPlayback, onDownloader, onAbout,
}: {
  onManage: () => void
  onPlayback: () => void
  onDownloader: () => void
  onAbout: () => void
}) {
  const scrollTo = (id: string) => document.getElementById(id)?.scrollIntoView({ behavior: 'smooth', block: 'start' })
  return (
    <section className="settings-view">
      <div className="section-eyebrow">PREFERENCES</div>
      <h1>Settings</h1>
      <p className="settings-lede">Configure how RollaPlay works on this device.</p>
      <div className="settings-grid">
        <article className="settings-card" id="settings-general">
          <div className="settings-card-icon"><Info size={19} /></div>
          <div><h3>General</h3><p>Setup guide and information about this app.</p></div>
          <button className="settings-action" onClick={onAbout}>About <span>→</span></button>
        </article>
        <article className="settings-card" id="settings-library">
          <div className="settings-card-icon"><FolderPlus size={19} /></div>
          <div><h3>Media library</h3><p>Choose which folders are included in your library.</p></div>
          <button className="settings-action" onClick={onManage}>Manage <span>→</span></button>
        </article>
        <article className="settings-card" id="settings-playback">
          <div className="settings-card-icon"><Gauge size={19} /></div>
          <div><h3>Playback</h3><p>Autoplay, repeat, and shuffle preferences.</p></div>
          <button className="settings-action" onClick={onPlayback}>Configure <span>→</span></button>
        </article>
        <article className="settings-card" id="settings-downloader">
          <div className="settings-card-icon"><Download size={19} /></div>
          <div><h3>Downloader</h3><p>Public YouTube links, tool status, and saved MP4 files.</p></div>
          <button className="settings-action" onClick={onDownloader}>Open <span>→</span></button>
        </article>
        <article className="settings-card" id="settings-network">
          <div className="settings-card-icon"><Wifi size={19} /></div>
          <div><h3>Network and Cast</h3><p>LAN streaming, receiver discovery, and sender diagnostics.</p></div>
          <button className="settings-action" onClick={() => scrollTo('settings-network-panel')}>Open <span>→</span></button>
        </article>
        <article className="settings-card" id="settings-diagnostics">
          <div className="settings-card-icon"><ShieldCheck size={19} /></div>
          <div><h3>Diagnostics</h3><p>Redacted local checks and streaming diagnostics.</p></div>
          <button className="settings-action" onClick={() => scrollTo('settings-cast')}>Review <span>→</span></button>
        </article>
      </div>
      <div className="privacy-banner"><ShieldCheck size={19} /><div><strong>Your privacy matters</strong><p>Media metadata and playlists are stored locally on this PC. No cloud account required.</p></div></div>
      <div id="settings-network-panel"><MediaServerSettings /></div>
      <div id="settings-cast"><CastPanel /></div>
    </section>
  )
}

function App() {
  const [active, setActive] = useState<View>('Home')
  const [query, setQuery] = useState('')
  const [setupOpen, setSetupOpen] = useState(false)
  const [setupStep, setSetupStep] = useState(0)
  const [setupDone, setSetupDone] = useState(() => localStorage.getItem('wms.setup.completed') === '1')
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => localStorage.getItem('wms.sidebar.collapsed') === '1')
  const [aboutOpen, setAboutOpen] = useState(false)
  const [exitConfirmOpen, setExitConfirmOpen] = useState(false)
  const [selectedMediaIds, setSelectedMediaIds] = useState<number[]>([])
  const [notice, setNotice] = useState<{ message: string; level: Phase5Notification['level'] } | null>(null)
  const [connectOpen, setConnectOpen] = useState(false)
  const [playingOverride, setPlayingOverride] = useState<boolean | null>(null)
  const [volumePreview, setVolumePreview] = useState<number | null>(null)
  const senderHostRef = useRef<HTMLDivElement | null>(null)
  const senderEmbeddedRef = useRef(false)
  const playlistReady = isPlaylistAvailable()
  const { data: phase5Snapshot, mutate: mutatePhase5 } = useSWR(
    playlistReady ? 'phase5-playback-state' : null,
    getPlaybackState,
    { refreshInterval: 1500 },
  )
  const { data: playlists } = useSWR(
    playlistReady ? 'phase5-playlists' : null,
    listPlaylists,
    { refreshInterval: 5000 },
  )
  const { data: recentFiles } = useSWR(
    isDesktopApp() ? ['recent-media', '', '', 'date'] as const : null,
    getMediaFiles,
    { refreshInterval: 10000 },
  )
  const { data: mediaDirectories } = useSWR(
    isDesktopApp() ? 'about-media-directories' : null,
    getMediaDirectories,
    { refreshInterval: 30000 },
  )
  const showNotice = useCallback<Notice>((message, level = 'info') => {
    setNotice({ message, level })
  }, [])
  const completeSetup = useCallback(() => {
    localStorage.setItem('wms.setup.completed', '1')
    setSetupDone(true)
    debugLog('wizard', 'setup_completed')
  }, [])
  const playSelectedMedia = useCallback((id: number) => {
    void playMediaNow(id).then(() => mutatePhase5()).catch((error) => showNotice(String(error), 'error'))
  }, [mutatePhase5, showNotice])
  const playSelectedPlaylist = useCallback((id: number) => {
    void playPlaylist(id).then(() => mutatePhase5()).catch((error) => showNotice(String(error), 'error'))
  }, [mutatePhase5, showNotice])
  const toggleSidebar = useCallback(() => {
    setSidebarCollapsed((previous) => {
      const next = !previous
      localStorage.setItem('wms.sidebar.collapsed', next ? '1' : '0')
      return next
    })
  }, [])
  const confirmExit = useCallback(async () => {
    try {
      await invoke('approve_app_exit')
      await getCurrentWindow().close()
    } catch (error) {
      setExitConfirmOpen(false)
      showNotice(String(error), 'error')
    }
  }, [showNotice])
  const clearHistory = useCallback(() => {
    if (!phase5Snapshot?.history.length) return
    void clearWatchHistory()
      .then(() => mutatePhase5())
      .then(() => showNotice('Watch history cleared.', 'success'))
      .catch((error) => showNotice(String(error), 'error'))
  }, [phase5Snapshot?.history.length, mutatePhase5, showNotice])
  useEffect(() => {
    if (!isDesktopApp()) return
    let active = true
    let unlisten: (() => void) | undefined
    void listen('app-exit-requested', () => { if (active) setExitConfirmOpen(true) })
      .then((stop) => { if (active) unlisten = stop; else stop() })
      .catch(() => { /* The native close still works without the styled dialog. */ })
    return () => { active = false; unlisten?.() }
  }, [])
  useEffect(() => {
    if (!isDesktopApp()) return
    let disposed = false
    void (async () => {
      try {
        const [status, directories] = await Promise.all([getLocalStatus(), getMediaDirectories()])
        if (disposed) return
        debugLog('wizard', 'startup_check', {
          databaseReady: status.database_ready,
          directoryCount: directories.length,
        })
        if (!status.database_ready) {
          showNotice('The local database is not ready. Restart the app before setting up your library.', 'error')
          return
        }
        if (localStorage.getItem('wms.setup.completed') !== '1') {
          setSetupStep(0)
          setSetupOpen(true)
        }
      } catch (error) {
        if (!disposed) debugLog('wizard', 'startup_check_failed', { message: String(error) })
      }
    })()
    return () => { disposed = true }
  }, [showNotice])
  useEffect(() => {
    if (!playlistReady) return
    let active = true
    const unlisten = listen<PlaybackSnapshot>('phase5-snapshot', (event) => {
      if (active) void mutatePhase5(event.payload, { revalidate: false })
    })
    return () => { active = false; void unlisten.then((off) => off()) }
  }, [playlistReady, mutatePhase5])
  useEffect(() => {
    if (!playlistReady) return
    let active = true
    const unlisten = listen<Phase5Notification>('phase5-notification', (event) => {
      if (active) showNotice(event.payload.message, event.payload.level)
    })
    return () => { active = false; void unlisten.then((off) => off()) }
  }, [playlistReady, showNotice])
  useEffect(() => {
    if (!notice) return
    const timer = window.setTimeout(() => setNotice(null), 5000)
    return () => window.clearTimeout(timer)
  }, [notice])
  // Wizard, top-bar and Cast-panel buttons all open the same in-app Connect
  // overlay instead of a separate floating Chrome window.
  useEffect(() => {
    let active = true
    const unlisten = listen('cast-sender-launch-requested', () => {
      if (active) setConnectOpen(true)
    })
    return () => { active = false; void unlisten.then((off) => off()) }
  }, [])
  const castReady = isCastAvailable()
  // The hidden sender acknowledges when the desktop app has armed a receiver.
  // A trusted OS click is then required because Chrome only opens its official
  // Cast picker from a real user gesture in the sender page.
  useEffect(() => {
    if (!castReady) return
    let active = true
    const unlisten = onCastPlaybackEvent((event) => {
      if (!active) return
      if (event.kind === 'picker_clicked') {
        debugLog('wizard', 'Cast picker click reached the hidden sender')
        return
      }
      if (event.kind !== 'picker_armed') return
      debugLog('wizard', 'Cast receiver armed; sending trusted picker click')
      void clickCastSender()
        .then(() => debugLog('wizard', 'Cast picker click delivered'))
        .catch((error) => showNotice(String(error), 'error'))
    })
    return () => { active = false; void unlisten.then((off) => off()) }
  }, [castReady, showNotice])
  const { data: castPlayback } = useSWR(castReady ? 'cast-playback-state' : null, getCastPlaybackState, { refreshInterval: 1500 })
  const castDevice = castDeviceName(castPlayback)
  const castTitle = castPlayback?.filename ?? null
  const castState = castPlayback?.player_state ?? null
  // The receiver status is polled, so flip the button immediately on a click
  // and drop the override once the receiver confirms the new state.
  const isPlaying = playingOverride ?? castState === 'PLAYING'
  const castConnected = !!castDevice
  const castHasMedia = castTitle !== null && castPlayback?.media_file_id != null
  const castPosition = castPlayback?.position_seconds ?? 0
  const castDuration = castPlayback?.duration_seconds ?? 0
  // Smooth the receiver's polled position between updates so the bar walks
  // while playing. This is display-only: playlist advancement still waits for
  // a real IdleReason.FINISHED event from the receiver.
  const [progressNow, setProgressNow] = useState(castPosition)
  const [seekPreview, setSeekPreview] = useState<number | null>(null)
  const progressAnchor = useRef({
    position: castPosition,
    duration: castDuration,
    at: Date.now(),
    state: castState,
  })
  useEffect(() => {
    progressAnchor.current = {
      position: castPosition,
      duration: castDuration,
      at: castPlayback?.updated_at_epoch_ms ?? Date.now(),
      state: castState,
    }
    setProgressNow(castPosition)
  }, [castPosition, castDuration, castState, castPlayback?.updated_at_epoch_ms])
  useEffect(() => {
    if (castState !== 'PLAYING' || castDuration <= 0) return
    const timer = window.setInterval(() => {
      const anchor = progressAnchor.current
      const elapsed = Math.max(0, Date.now() - anchor.at) / 1000
      setProgressNow(Math.min(anchor.duration, anchor.position + elapsed))
    }, 250)
    return () => window.clearInterval(timer)
  }, [castState, castDuration])
  const displayedPosition = seekPreview ?? progressNow
  const progressPercent = castDuration > 0 ? Math.min(100, Math.max(0, (displayedPosition / castDuration) * 100)) : 0
  const commitSeek = () => {
    if (seekPreview === null) return
    const target = Math.max(0, Math.min(seekPreview, castDuration || 0))
    progressAnchor.current = { position: target, duration: castDuration, at: Date.now(), state: castState }
    setProgressNow(target)
    setSeekPreview(null)
    void castSendControl('seek', { positionSeconds: target }).catch((error) => showNotice(String(error), 'error'))
  }
  useEffect(() => {
    if (playingOverride !== null && castState && (castState === 'PLAYING') === playingOverride) {
      setPlayingOverride(null)
    }
  }, [castState, playingOverride])
  useEffect(() => {
    const receiverVolume = castPlayback?.volume_level
    if (volumePreview !== null && receiverVolume !== null && receiverVolume !== undefined && Math.abs(receiverVolume - volumePreview) < 0.02) {
      setVolumePreview(null)
    }
  }, [castPlayback?.volume_level, volumePreview])
  const pageTitle = useMemo(() => active === 'Home' ? 'Good evening' : active, [active])
  const closeSetup = () => { completeSetup(); setSetupOpen(false) }
  const openSetup = () => { setSetupStep(0); setSetupOpen(true) }
  const browse = () => { setSetupStep(1); setSetupOpen(true) }
  const openNetworkSetup = () => { setSetupStep(2); setSetupOpen(true) }
  const senderRect = useCallback((): SenderHostRect => {
    const element = senderHostRef.current
    if (!element) return { x: 24, y: 24, width: 380, height: 260 }
    const box = element.getBoundingClientRect()
    return {
      x: Math.round(box.left),
      y: Math.round(box.top),
      width: Math.round(box.width),
      height: Math.round(box.height),
    }
  }, [])

  const attachSenderWindow = useCallback(async () => {
    if (!isCastAvailable()) return
    try {
      const rect = senderRect()
      // A relaunched Chrome process gets a new HWND; the result tells us whether
      // it is still embedded so the next attempt can re-embed it.
      const embedded = senderEmbeddedRef.current
        ? await showCastSender(rect)
        : await embedCastSender(rect)
      senderEmbeddedRef.current = embedded
      // The sender page must never be visible. Concealing the window keeps the
      // SDK alive and lets Chrome open its own picker next to the Cast pill.
      await hideCastSender()
    } catch (error) {
      showNotice(`Could not prepare the Cast sender: ${String(error)}`, 'error')
    }
  }, [senderRect, showNotice])

  const closeConnect = useCallback(() => {
    // Concealing the child window leaves the Chrome process and Cast session
    // alive; only the app-owned device list is visible to the user.
    void hideCastSender().catch((error) => showNotice(String(error), 'error'))
    setConnectOpen(false)
  }, [showNotice])

  // The device menu is dismissed by ConnectPanel only after a receiver is
  // selected in response to this open menu. A restored session or a periodic
  // CONNECTED state must never close it before discovery can render devices.
  useEffect(() => {
    if (!connectOpen) return
    let disposed = false
    const frame = requestAnimationFrame(() => { if (!disposed) void attachSenderWindow() })
    return () => {
      disposed = true
      cancelAnimationFrame(frame)
    }
  }, [connectOpen, attachSenderWindow])

  // Cast device selection stays in the managed Chrome sender (the official
  // SDK requires it). The player bar opens that sender inside this window as
  // a panel instead of leaving a second floating window open.
  const openConnect = useCallback(async () => {
    debugLog('wizard', 'button_click', { control: 'open_connect', connected: !!castDevice })
    try {
      // Render the host first so the Chrome window is embedded as soon as it
      // appears instead of flashing as a separate top-level window.
      setConnectOpen(true)
      await launchCastSender()
      showNotice(
        castDevice
          ? `Connected to ${castDevice}. Choose another receiver any time from Connect.`
          : 'Connect is ready. Pick a discovered receiver to open the Cast dialog.',
        castDevice ? 'success' : 'info',
      )
    } catch (error) {
      showNotice(typeof error === 'string' ? error : error instanceof Error ? error.message : String(error), 'error')
    }
  }, [castDevice, showNotice])
  const toggleCastPlay = () => {
    if (!castReady || !castConnected || !castHasMedia) return
    setPlayingOverride(!isPlaying)
    castSendControl('toggle_play').catch((error) => {
      setPlayingOverride(null)
      showNotice(String(error), 'error')
    })
  }
  const queueNext = () => {
    if (!playlistReady || !phase5Snapshot?.current_entry_id) return
    playNext().then(() => mutatePhase5()).catch((error) => showNotice(String(error), 'error'))
  }
  const queuePrevious = () => {
    if (!playlistReady || !phase5Snapshot?.current_entry_id) return
    playPrevious().then(() => mutatePhase5()).catch((error) => showNotice(String(error), 'error'))
  }
  return (
    <div className={`app-shell ${sidebarCollapsed ? 'sidebar-collapsed' : ''}`}>
      <Sidebar
        active={active}
        playlistCount={playlists?.length ?? 0}
        collapsed={sidebarCollapsed}
        onToggle={toggleSidebar}
        onNavigate={setActive}
        onSetup={openSetup}
        onAbout={() => setAboutOpen(true)}
      />
      <main className="main-area">
        <header className="topbar">
          <div className="breadcrumb"><span>MY MEDIA</span><span className="crumb-slash">/</span><strong>{active.toUpperCase()}</strong></div>
          <div className="topbar-actions">
            {(active === 'Library' || active === 'Recently added') && <label className="search-box"><Search size={17} /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search indexed videos" aria-label="Search indexed videos" /></label>}
            <div className="cast-menu">
              <button className={`device-pill ${castConnected ? 'connected' : ''}`} onClick={() => void openConnect()} aria-label={`Cast status: ${castDevice ?? 'no device connected'}. Open Connect.`}><span className="status-dot" /><Cast size={17} /><span>{castDevice ?? 'No device'}</span><ChevronDown size={14} /></button>
              {/* The official Web Sender SDK lives here, fully transparent. */}
              <div ref={senderHostRef} className="sender-hidden-host" aria-hidden="true" />
              <ConnectPanel
                open={connectOpen}
                onClose={closeConnect}
                onNotice={showNotice}
                playback={castPlayback ?? null}
              />
            </div>
          </div>
        </header>
        {active === 'Settings' ? <SettingsView onManage={browse} onPlayback={() => setActive('Playlists')} onDownloader={() => setActive('Downloader')} onAbout={() => setAboutOpen(true)} />
          : active === 'Downloader' ? <Downloader />
          : active === 'Library' ? <MediaLibrary searchQuery={query} selectedMediaIds={selectedMediaIds} onSelectionChange={setSelectedMediaIds} />
          : active === 'Playlists' ? <section className="section-page queue-page"><div className="section-eyebrow">YOUR COLLECTION</div><h1>Playlists &amp; queue</h1><p>Saved playlists first, then the active playback queue and receiver-driven playback controls.</p><QueuePanel snapshot={phase5Snapshot ?? null} castConnected={castConnected} selectedMediaIds={selectedMediaIds} onNotice={showNotice} onSelectionConsumed={() => setSelectedMediaIds([])} onTogglePlay={toggleCastPlay} isPlaying={castState === 'PLAYING'} onHistoryCleared={() => void mutatePhase5()} /></section>
          : active === 'Recently added' ? <section className="section-page">
            <div className="section-eyebrow">YOUR COLLECTION</div>
            <h1>{pageTitle}</h1>
            <p>The newest videos indexed from your local folders.</p>
            {recentFiles && recentFiles.length > 0
              ? <RecentGrid files={recentFiles} limit={24} onPlay={playSelectedMedia} />
              : <EmptyLibrary onSetup={openSetup} onBrowse={browse} />}
          </section>
          : active === 'History' ? <section className="section-page">
            <div className="section-eyebrow">ACTIVITY</div>
            <div className="section-heading-row">
              <h1>{pageTitle}</h1>
              <button className="clear-button" onClick={clearHistory} disabled={!phase5Snapshot?.history.length} title="Clear all watch history"><Trash2 size={15} /> Clear history</button>
            </div>
            <p>Recorded from real receiver playback only.</p>
            <div className="history-list history-page-list">
              {phase5Snapshot?.history.length ? phase5Snapshot.history.map((entry) => (
                <div key={entry.id} className="history-row"><Clock3 size={14} /><span title={entry.filename}>{entry.filename}</span><small>{entry.completion_state ?? 'started'}</small><small>{formatSeconds(entry.watched_seconds)}</small></div>
              )) : <div className="playlist-empty">No playback recorded yet.</div>}
            </div>
          </section>
          : <>
          <section className="welcome-row"><div><div className="section-eyebrow"><span className="eyebrow-line" /> YOUR PERSONAL MEDIA SPACE</div><h1>Good evening<span className="title-period">.</span></h1><p>Your movies, your space. All stored locally.</p></div><button className="quick-add" onClick={browse}><Plus size={17} /> Add media</button></section>
          <HomeSpotlight
            playlists={playlists}
            recentFiles={recentFiles}
            onPlayPlaylist={playSelectedPlaylist}
            onPlayMedia={playSelectedMedia}
            onOpenLibrary={() => setActive('Library')}
            onOpenDownloader={() => setActive('Downloader')}
          />
          <div className="library-summary"><div className="summary-title"><div><div className="section-eyebrow">YOUR COLLECTION</div><h2>Recently added</h2></div><button className="text-link" onClick={() => setActive('Recently added')}>View all <span>→</span></button></div>
             {recentFiles && recentFiles.length > 0
               ? <RecentGrid files={recentFiles} onPlay={playSelectedMedia} />
               : <EmptyLibrary onSetup={openSetup} onBrowse={browse} />}
           </div>
          {!setupDone && <>
            <div className="content-heading first-steps-heading"><div><div className="section-eyebrow">START HERE</div><h2>Make it yours</h2></div><button className="text-link" onClick={openSetup}>Setup guide <span>→</span></button></div>
            <div className="quick-cards"><button className="quick-card" onClick={browse}><span className="quick-card-icon purple"><FolderPlus size={19} /></span><span className="quick-card-copy"><strong>Add your folders</strong><small>Build your local library</small></span><span className="card-arrow">↗</span></button><button className="quick-card" onClick={() => void openConnect()}><span className="quick-card-icon blue"><Cast size={19} /></span><span className="quick-card-copy"><strong>Connect your TV</strong><small>{castDevice ? `Casting to ${castDevice}` : 'Choose a device'}</small></span><span className="card-arrow">↗</span></button><button className="quick-card" onClick={openNetworkSetup}><span className="quick-card-icon amber"><HardDrive size={19} /></span><span className="quick-card-copy"><strong>Check your setup</strong><small>Network and preferences</small></span><span className="card-arrow">↗</span></button></div>
          </>}
          <HomePlaylistShelf playlists={playlists} onPlay={playSelectedPlaylist} onOpenAll={() => setActive('Playlists')} />
        </>}
        <footer className="app-footer"><span>ROLLAPLAY <b>•</b> LOCAL MEDIA. LOUD ATTITUDE.</span><span><ShieldCheck size={13} /> LOCAL FIRST</span></footer>
      </main>
      {notice && <div className={`phase5-toast toast-${notice.level}`} role="status"><NoticeIcon level={notice.level} /><span>{notice.message}</span><button onClick={() => setNotice(null)} aria-label="Dismiss notification"><X size={14} /></button></div>}
      <PlayerBar
        castReady={castReady}
        playlistReady={playlistReady}
        castConnected={castConnected}
        castHasMedia={castHasMedia}
        isPlaying={isPlaying}
        castTitle={castTitle}
        castDevice={castDevice}
        castState={castState}
        displayedPosition={displayedPosition}
        castDuration={castDuration}
        progressPercent={progressPercent}
        previousTitle={phase5Snapshot ? `Previous · restarts after ${phase5Snapshot.previous_restart_threshold_seconds}s; with Shuffle follows history` : 'Previous'}
        queueAvailable={!!phase5Snapshot?.current_entry_id}
        muted={!!castPlayback?.muted}
        volume={volumePreview ?? castPlayback?.volume_level ?? 1}
        onPrevious={queuePrevious}
        onNext={queueNext}
        onTogglePlay={toggleCastPlay}
        onSeekPreview={setSeekPreview}
        onCommitSeek={commitSeek}
        onOpenConnect={() => void openConnect()}
        onMute={() => void castSendControl('set_mute', { muted: !castPlayback?.muted }).catch((error) => showNotice(String(error), 'error'))}
        onVolume={(value) => {
          setVolumePreview(value)
          void castSendControl('set_volume', { volume: value }).catch((error) => showNotice(String(error), 'error'))
        }}
      />
      {setupOpen && <SetupDialog
        step={setupStep}
        setStep={setSetupStep}
        onClose={closeSetup}
        onOpenLibrary={() => {
          completeSetup()
          setActive('Library')
          setSetupOpen(false)
        }}
        onNotice={showNotice}
        selectedMediaIds={selectedMediaIds}
        onSelectionChange={setSelectedMediaIds}
        snapshot={phase5Snapshot ?? null}
      />}
      {aboutOpen && <AboutDialog
        onClose={() => setAboutOpen(false)}
        onOpenSetup={() => { setAboutOpen(false); openSetup() }}
        folderCount={mediaDirectories?.length ?? 0}
        videoCount={recentFiles?.length ?? 0}
        playlistCount={playlists?.length ?? 0}
      />}
      {exitConfirmOpen && <ExitConfirmDialog onCancel={() => setExitConfirmOpen(false)} onConfirm={() => void confirmExit()} />}
    </div>
  )
}

export default App
