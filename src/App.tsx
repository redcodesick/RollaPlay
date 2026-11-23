import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import useSWR from 'swr'
import { listen } from '@tauri-apps/api/event'
import { convertFileSrc } from '@tauri-apps/api/core'
import MediaLibrary from './components/MediaLibrary'
import MediaServerSettings from './components/MediaServerSettings'
import CastPanel from './components/CastPanel'
import ConnectPanel from './components/ConnectPanel'
import QueuePanel from './components/QueuePanel'
import { FolderList, useMediaFolders } from './components/LibraryFolders'
import MediaPicker from './components/MediaPicker'
import PlaylistManager from './components/PlaylistManager'
import { CastCheck, ChromeCheck, NetworkCheck } from './components/SetupChecks'
import {
  castDeviceName, castSendControl, embedCastSender, formatSeconds, getCastPlaybackState,
  hideCastSender, isCastAvailable, launchCastSender, moveCastSender, onCastPlaybackEvent,
  playerStateLabel, showCastSender, type SenderHostRect,
} from './cast'
import {
  getPlaybackState, isPlaylistAvailable, listPlaylists, playMediaNow, playNext, playPrevious,
  type Phase5Notification, type PlaybackSnapshot,
} from './playlist'
import { formatBytes, getLocalStatus, getMediaDirectories, getMediaFiles, isDesktopApp } from './media'
import { debugLog } from './diagnostics'
import {
  Aperture, AlertCircle, Cast, Check, ChevronDown, CircleHelp, Clapperboard, Clock3,
  Film, FolderPlus, Gauge, HardDrive, History, House, Library,
  ListMusic, MoreHorizontal, Pause, Play, Plus, Search, Settings2, ShieldCheck,
  SkipBack, SkipForward, SlidersHorizontal, Sparkles, Volume2, VolumeX, Wifi, X,
} from 'lucide-react'

type View = 'Home' | 'Library' | 'Playlists' | 'Recently added' | 'History' | 'Settings'
type Notice = (message: string, level?: Phase5Notification['level']) => void

const navItems: { label: View; icon: typeof House }[] = [
  { label: 'Home', icon: House },
  { label: 'Library', icon: Library },
  { label: 'Playlists', icon: ListMusic },
  { label: 'Recently added', icon: Clock3 },
  { label: 'History', icon: History },
]

function Sidebar({ active, playlistCount, onNavigate, onSetup }: { active: View; playlistCount: number; onNavigate: (view: View) => void; onSetup: () => void }) {
  return (
    <aside className="sidebar">
      <a className="brand" href="#home" aria-label="RollaPlay home" onClick={(event) => { event.preventDefault(); onNavigate('Home') }}>
        <span className="brand-mark"><Aperture size={19} strokeWidth={2.4} /></span>
        <span className="brand-name"><b>ROLLA</b>PLAY<span>ROCKNROLLA MEDIA STATION</span></span>
      </a>
      <div className="nav-label">YOUR SPACE</div>
      <nav className="primary-nav" aria-label="Main navigation">
        {navItems.map(({ label, icon: Icon }) => (
          <button key={label} className={`nav-item ${active === label ? 'active' : ''}`} onClick={() => onNavigate(label)} aria-current={active === label ? 'page' : undefined}>
            <Icon size={19} strokeWidth={1.8} /><span>{label}</span>{label === 'Playlists' && <span className="nav-count">{playlistCount}</span>}
          </button>
        ))}
      </nav>
      <div className="sidebar-divider" />
      <button className={`nav-item ${active === 'Settings' ? 'active' : ''}`} onClick={() => onNavigate('Settings')} aria-current={active === 'Settings' ? 'page' : undefined}><Settings2 size={19} strokeWidth={1.8} /><span>Settings</span></button>
      <div className="sidebar-spacer" />
      <div className="local-card">
        <div className="local-card-icon"><HardDrive size={17} /></div>
        <div className="local-copy"><strong>Everything stays local</strong><span>Your files never leave this PC.</span></div>
        <ShieldCheck size={16} className="shield-icon" />
      </div>
      <button className="help-link" onClick={onSetup}><CircleHelp size={17} /> Setup guide</button>
      <div className="profile-row"><div className="profile-avatar">M</div><div><strong>RockNRolla Station</strong><span>Local device</span></div><MoreHorizontal size={18} /></div>
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
  const steps = ['Welcome', 'Media folders', 'Playlists', 'Google Chrome', 'Network', 'Cast devices', 'All set']
  const isDesktop = isDesktopApp()
  const folders = useMediaFolders()
  const [skippedFolders, setSkippedFolders] = useState(false)
  const lastStep = steps.length - 1
  const optionalSteps = new Set([2, 3, 4, 5])

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
        <div className="setup-eyebrow"><Sparkles size={15} /> FIRST-RUN SETUP <span>STEP {step + 1} OF {steps.length}</span></div>
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
          <div className="setup-icon"><ListMusic size={25} /></div>
          <h2 id="setup-title">Make it yours<br /><em>with playlists.</em></h2>
          <p>Select indexed videos, then create a playlist or choose an existing one. This step is optional; existing playlists and selections are stored in SQLite.</p>
          <MediaPicker selectedMediaIds={selectedMediaIds} onSelectionChange={onSelectionChange} />
          <PlaylistManager
            snapshot={snapshot}
            selectedMediaIds={selectedMediaIds}
            onNotice={onNotice}
            onSelectionConsumed={() => onSelectionChange([])}
            compact
          />
        </>}

        {step === 3 && <>
          <div className="setup-icon"><Film size={25} /></div>
          <h2 id="setup-title">Check<br /><em>Google Chrome.</em></h2>
          <p>The official Google Cast Web Sender SDK runs inside the app's sender panel using the installed Chrome browser. This step checks the browser and can start the sender.</p>
          <ChromeCheck onNotice={onNotice} />
        </>}

        {step === 4 && <>
          <div className="setup-icon"><Wifi size={25} /></div>
          <h2 id="setup-title">Check your<br /><em>local network.</em></h2>
          <p>Streaming binds to a private LAN IPv4 address. These checks read the active interface and server state from the desktop app.</p>
          <NetworkCheck onNotice={onNotice} />
        </>}

        {step === 5 && <>
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

function SettingsView({ onManage }: { onManage: () => void }) {
  return (
    <section className="settings-view">
      <div className="section-eyebrow">PREFERENCES</div>
      <h1>Settings</h1>
      <p className="settings-lede">Configure how RollaPlay works on this device.</p>
      <div className="settings-grid">
        <article className="settings-card">
          <div className="settings-card-icon"><FolderPlus size={19} /></div>
          <div><h3>Media folders</h3><p>Choose which folders are included in your library.</p></div>
          <button className="settings-action" onClick={onManage}>Manage <span>→</span></button>
        </article>
        <article className="settings-card">
          <div className="settings-card-icon"><Wifi size={19} /></div>
          <div><h3>Local network</h3><p>LAN streaming and connectivity diagnostics.</p></div>
          <span className="not-configured">PHASE 3</span>
        </article>
        <article className="settings-card">
          <div className="settings-card-icon"><Cast size={19} /></div>
          <div><h3>Cast connection</h3><p>Official Google Cast sender via Google Chrome.</p></div>
          <span className="not-configured">PHASE 4</span>
        </article>
        <article className="settings-card">
          <div className="settings-card-icon"><Gauge size={19} /></div>
          <div><h3>Playback behavior</h3><p>Autoplay, repeat, and shuffle preferences.</p></div>
          <span className="not-configured">DEFAULTS</span>
        </article>
      </div>
      <div className="privacy-banner"><ShieldCheck size={19} /><div><strong>Your privacy matters</strong><p>Media metadata and playlists are stored locally on this PC. No cloud account required.</p></div></div>
      <MediaServerSettings />
      <CastPanel />
    </section>
  )
}

function App() {
  const [active, setActive] = useState<View>('Home')
  const [query, setQuery] = useState('')
  const [setupOpen, setSetupOpen] = useState(false)
  const [setupStep, setSetupStep] = useState(0)
  const [selectedMediaIds, setSelectedMediaIds] = useState<number[]>([])
  const [notice, setNotice] = useState<{ message: string; level: Phase5Notification['level'] } | null>(null)
  const [connectOpen, setConnectOpen] = useState(false)
  const [senderEmbedded, setSenderEmbedded] = useState(false)
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
  const showNotice = useCallback<Notice>((message, level = 'info') => {
    setNotice({ message, level })
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
  const progressPercent = castDuration > 0 ? Math.min(100, Math.max(0, (castPosition / castDuration) * 100)) : 0
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
  const openSetup = () => { setSetupStep(0); setSetupOpen(true) }
  const browse = () => { setSetupStep(1); setSetupOpen(true) }
  const openNetworkSetup = () => { setSetupStep(4); setSetupOpen(true) }
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
      setSenderEmbedded(embedded)
    } catch (error) {
      showNotice(`Could not place the Cast sender in this window: ${String(error)}`, 'error')
    }
  }, [senderRect, showNotice])

  const closeConnect = useCallback(() => {
    // Hiding the child window leaves the Chrome process and Cast session alive.
    void hideCastSender().catch((error) => showNotice(String(error), 'error'))
    setConnectOpen(false)
  }, [showNotice])

  // Spotify-style device picker: the sender panel is only needed while choosing
  // a receiver. Once a receiver is selected it hides itself and playback stays
  // controlled from the app's player bar.
  useEffect(() => {
    let active = true
    const unlisten = onCastPlaybackEvent((event) => {
      if (!active) return
      const deviceChosen =
        event.kind === 'session_started' || event.kind === 'session_resumed' ||
        (event.kind === 'cast_state' && event.cast_state === 'CONNECTED' && !!event.device_name)
      // Wait until the Chrome window is actually embedded. Hiding it while it
      // is still a top-level window drops the bridge connection.
      if (deviceChosen && senderEmbedded) closeConnect()
    })
    return () => { active = false; void unlisten.then((off) => off()) }
  }, [closeConnect, senderEmbedded])

  const previousCastDeviceRef = useRef<string | null>(null)
  useEffect(() => {
    const previous = previousCastDeviceRef.current
    previousCastDeviceRef.current = castDevice
    if (!castDevice || previous || !connectOpen || !senderEmbedded) return
    const timer = window.setTimeout(() => closeConnect(), 700)
    return () => window.clearTimeout(timer)
  }, [castDevice, connectOpen, senderEmbedded, closeConnect])

  // A receiver may be selected while the sender window is still being embedded.
  // As soon as embedding succeeds, close the picker panel without killing the
  // Cast session.
  useEffect(() => {
    if (!senderEmbedded || !connectOpen || !castDevice) return
    const timer = window.setTimeout(() => closeConnect(), 300)
    return () => window.clearTimeout(timer)
  }, [senderEmbedded, connectOpen, castDevice, closeConnect])

  useEffect(() => {
    if (!connectOpen) return
    let disposed = false
    const frame = requestAnimationFrame(() => { if (!disposed) void attachSenderWindow() })
    const reposition = () => {
      if (!disposed && senderEmbeddedRef.current) void moveCastSender(senderRect()).catch(() => {})
    }
    const observer = new ResizeObserver(reposition)
    if (senderHostRef.current) observer.observe(senderHostRef.current)
    window.addEventListener('resize', reposition)
    return () => {
      disposed = true
      cancelAnimationFrame(frame)
      observer.disconnect()
      window.removeEventListener('resize', reposition)
    }
  }, [connectOpen, attachSenderWindow, senderRect])

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
    <div className="app-shell">
      <Sidebar active={active} playlistCount={playlists?.length ?? 0} onNavigate={setActive} onSetup={openSetup} />
      <main className="main-area">
        <header className="topbar"><div className="breadcrumb"><span>MY MEDIA</span><span className="crumb-slash">/</span><strong>{active.toUpperCase()}</strong></div><div className="topbar-actions"><label className="search-box"><Search size={17} /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search your library" aria-label="Search your library" /><kbd>⌘ K</kbd></label><button className="icon-button" aria-label="Sort library"><SlidersHorizontal size={19} /></button><button className={`device-pill ${castConnected ? 'connected' : ''}`} onClick={() => void openConnect()} aria-label={`Cast status: ${castDevice ?? 'no device connected'}. Open Connect.`}><span className="status-dot" /><Cast size={17} /><span>{castDevice ?? 'No device'}</span><ChevronDown size={14} /></button></div></header>
        {active === 'Settings' ? <SettingsView onManage={browse} /> : active === 'Library' ? <MediaLibrary searchQuery={query} selectedMediaIds={selectedMediaIds} onSelectionChange={setSelectedMediaIds} /> : active === 'Playlists' ? <section className="section-page queue-page"><div className="section-eyebrow">YOUR COLLECTION</div><h1>Playlists &amp; queue</h1><p>Saved playlists, the active playback queue and receiver-driven playback controls.</p><QueuePanel snapshot={phase5Snapshot ?? null} castConnected={castConnected} selectedMediaIds={selectedMediaIds} onNotice={showNotice} onSelectionConsumed={() => setSelectedMediaIds([])} onTogglePlay={toggleCastPlay} isPlaying={castState === 'PLAYING'} onHistoryCleared={() => void mutatePhase5()} /></section> : active === 'History' || active === 'Recently added' ? <section className="section-page"><div className="section-eyebrow">YOUR COLLECTION</div><h1>{pageTitle}</h1><p>{active === 'History' ? 'Your playback history appears in the Playlists & queue panel.' : 'New additions to your library will appear here.'}</p><EmptyLibrary onSetup={openSetup} onBrowse={browse} /></section> : <>
          <section className="welcome-row"><div><div className="section-eyebrow"><span className="eyebrow-line" /> YOUR PERSONAL MEDIA SPACE</div><h1>Good evening<span className="title-period">.</span></h1><p>Your movies, your space. All stored locally.</p></div><button className="quick-add" onClick={browse}><Plus size={17} /> Add media</button></section>
          <section className="featured-card"><img src="/media/featured-still.png" alt="Cinematic scene of a figure in a rain-lit city at night" /><div className="feature-overlay" /><div className="feature-content"><div className="feature-tag"><span /> MADE FOR YOUR LIVING ROOM</div><h2>Your library.<br /><em>Your rules.</em></h2><p>Bring your local video collection together and stream directly to your TV. No subscriptions. No uploads. Just your media.</p><button className="feature-button" onClick={browse}><FolderPlus size={17} /> Add your first folder <span>→</span></button></div><div className="feature-side-note"><span>01</span><i /> PRIVATE BY DESIGN</div><div className="feature-pagination"><span className="selected" /><span /><span /></div></section>
          <div className="content-heading"><div><div className="section-eyebrow">START HERE</div><h2>Make it yours</h2></div><button className="text-link" onClick={openSetup}>Setup guide <span>→</span></button></div>
          <div className="quick-cards"><button className="quick-card" onClick={browse}><span className="quick-card-icon purple"><FolderPlus size={19} /></span><span className="quick-card-copy"><strong>Add your folders</strong><small>Build your local library</small></span><span className="card-arrow">↗</span></button><button className="quick-card" onClick={() => void openConnect()}><span className="quick-card-icon blue"><Cast size={19} /></span><span className="quick-card-copy"><strong>Connect your TV</strong><small>{castDevice ? `Casting to ${castDevice}` : 'Choose a device'}</small></span><span className="card-arrow">↗</span></button><button className="quick-card" onClick={openNetworkSetup}><span className="quick-card-icon amber"><HardDrive size={19} /></span><span className="quick-card-copy"><strong>Check your setup</strong><small>Network and preferences</small></span><span className="card-arrow">↗</span></button></div>
          <div className="library-summary"><div className="summary-title"><div><div className="section-eyebrow">YOUR COLLECTION</div><h2>Recently added</h2></div><button className="text-link" onClick={() => setActive('Library')}>View library <span>→</span></button></div>
             {recentFiles && recentFiles.length > 0 ? <div className="recent-grid">
               {recentFiles.slice(0, 6).map((media) => <button
                 key={media.id}
                 className="recent-card"
                 title={`Play ${media.filename}`}
                 onClick={() => { void playMediaNow(media.id).then(() => mutatePhase5()).catch((error) => showNotice(String(error), 'error')) }}
               >
                 <span className="recent-art">{media.thumbnail_status === 'ready' && media.thumbnail_path
                   ? <img src={convertFileSrc(media.thumbnail_path)} alt="" />
                   : <Film size={18} />}</span>
                 <strong>{media.filename}</strong>
                 <small>{formatBytes(media.size_bytes)}</small>
               </button>)}
             </div> : <EmptyLibrary onSetup={openSetup} onBrowse={browse} />}
           </div>
        </>}
        <footer className="app-footer"><span>ROLLAPLAY <b>•</b> LOCAL MEDIA. LOUD ATTITUDE.</span><span><ShieldCheck size={13} /> LOCAL FIRST</span></footer>
      </main>
      {notice && <div className={`phase5-toast toast-${notice.level}`} role="status"><Check size={15} /><span>{notice.message}</span><button onClick={() => setNotice(null)} aria-label="Dismiss notification"><X size={14} /></button></div>}
      <ConnectPanel
        open={connectOpen}
        onClose={closeConnect}
        onNotice={showNotice}
        playback={castPlayback ?? null}
        localPlaying={phase5Snapshot?.state === 'playing' && !castConnected}
        senderHostRef={senderHostRef}
      />
      <div className="player-bar"><div className="player-track"><div className={`player-art ${castConnected && castHasMedia ? 'casting' : ''}`}><Cast size={17} /></div><div className="player-meta"><strong>{castTitle ?? 'Nothing is playing'}</strong><span>{castConnected && castHasMedia ? `${playerStateLabel(castState)} on ${castDevice}` : 'Pick a video and cast it to your TV'}</span></div></div><div className="player-controls"><button disabled={!playlistReady || !phase5Snapshot?.current_entry_id} onClick={queuePrevious} aria-label="Previous video" title={phase5Snapshot ? `Previous · restarts after ${phase5Snapshot.previous_restart_threshold_seconds}s; with Shuffle follows history` : 'Previous'}><SkipBack size={17} /></button><button className="player-play" disabled={!castReady || !castConnected || !castHasMedia} onClick={toggleCastPlay} aria-label={isPlaying ? 'Pause' : 'Play'}>{isPlaying ? <Pause size={15} fill="currentColor" /> : <Play size={15} fill="currentColor" />}</button><button disabled={!playlistReady || !phase5Snapshot?.current_entry_id} onClick={queueNext} aria-label="Next video" title="Next video"><SkipForward size={17} /></button></div><div className="player-progress"><span>{formatSeconds(castPosition)}</span><div className="progress-track"><i style={{ width: `${progressPercent}%` }} /></div><span>{formatSeconds(castDuration)}</span></div><div className="player-device"><Cast size={17} className={castConnected ? 'casting' : ''} /><span>{castDevice ?? 'No Cast device'}</span><div className="player-volume"><button
  aria-label={castPlayback?.muted ? 'Unmute' : 'Mute'}
  disabled={!castReady || !castConnected}
  onClick={() => void castSendControl('set_mute', { muted: !castPlayback?.muted }).catch((error) => showNotice(String(error), 'error'))}
>{castPlayback?.muted ? <VolumeX size={15} /> : <Volume2 size={15} />}</button><input
  type="range" min={0} max={1} step={0.01}
  aria-label="Receiver volume"
  disabled={!castReady || !castConnected}
  value={volumePreview ?? castPlayback?.volume_level ?? 1}
  onChange={(event) => {
    const value = Number(event.target.value)
    setVolumePreview(value)
    void castSendControl('set_volume', { volume: value }).catch((error) => showNotice(String(error), 'error'))
  }}
/></div></div><div className="player-disclaimer"><span /> {castConnected && castHasMedia ? 'PLAYBACK CONTROLLED THROUGH GOOGLE CAST' : 'PLAYER READY WHEN CAST IS CONNECTED'}</div></div>
      {setupOpen && <SetupDialog
        step={setupStep}
        setStep={setSetupStep}
        onClose={() => setSetupOpen(false)}
        onOpenLibrary={() => {
          localStorage.setItem('wms.setup.completed', '1')
          debugLog('wizard', 'setup_completed')
          setActive('Library')
          setSetupOpen(false)
        }}
        onNotice={showNotice}
        selectedMediaIds={selectedMediaIds}
        onSelectionChange={setSelectedMediaIds}
        snapshot={phase5Snapshot ?? null}
      />}
      {query && active !== 'Library' && <div className="search-feedback" role="status">Open Media library to search your indexed videos.</div>}
    </div>
  )
}

export default App
