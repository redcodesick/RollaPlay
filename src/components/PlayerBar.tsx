import {
  Cast, LoaderCircle, Pause, Play, SkipBack, SkipForward, Volume2, VolumeX,
} from 'lucide-react'
import { formatSeconds, playerStateLabel } from '../cast'

export type PlayerBarProps = {
  castReady: boolean
  playlistReady: boolean
  castTransport?: 'cast' | 'dlna'
  castConnected: boolean
  castHasMedia: boolean
  isPlaying: boolean
  castTitle: string | null
  castDevice: string | null
  castState: string | null
  displayedPosition: number
  castDuration: number
  progressPercent: number
  previousTitle: string
  queueAvailable: boolean
  nextAvailable?: boolean
  muted: boolean
  volume: number
  onPrevious: () => void
  onNext: () => void
  onTogglePlay: () => void
  onSeekPreview: (value: number) => void
  onCommitSeek: () => void
  onCancelSeek: () => void
  onOpenConnect: () => void
  onMute: () => void
  onVolume: (value: number) => void
}

export default function PlayerBar({
  castReady,
  playlistReady,
  castTransport = 'cast',
  castConnected,
  castHasMedia,
  isPlaying,
  castTitle,
  castDevice,
  castState,
  displayedPosition,
  castDuration,
  progressPercent,
  previousTitle,
  queueAvailable,
  nextAvailable,
  muted,
  volume,
  onPrevious,
  onNext,
  onTogglePlay,
  onSeekPreview,
  onCommitSeek,
  onCancelSeek,
  onOpenConnect,
  onMute,
  onVolume,
}: PlayerBarProps) {
  const canControlPlayback = castReady && castConnected && castHasMedia
  const canUseQueue = playlistReady && queueAvailable
  const canUseNext = playlistReady && (nextAvailable ?? queueAvailable)
  const canSeek = canControlPlayback && castDuration > 0
  const canUseVolume = castReady && castConnected
  const isBuffering = castConnected && castHasMedia && castState === 'BUFFERING'
  const transportLabel = castTransport === 'dlna' ? 'DLNA / UPnP' : 'Google Cast'

  return (
    <div className="player-bar" aria-label="Playback controls">
      <div className="player-track">
        <div className={`player-art ${castConnected && castHasMedia ? 'casting' : ''} ${isBuffering ? 'is-buffering' : ''}`} aria-busy={isBuffering}>
          {isBuffering ? <LoaderCircle size={20} className="spin-icon" /> : <Cast size={20} />}
        </div>
        <div className="player-meta">
          <strong>{castTitle ?? 'Nothing is playing'}</strong>
          <span>{castConnected && castHasMedia
            ? `${playerStateLabel(castState)} on ${castDevice ?? 'your receiver'}`
            : 'Pick a video and cast it to your TV'}</span>
        </div>
      </div>

      <div className="player-center">
        <div className="player-controls">
          <button disabled={!canUseQueue} onClick={onPrevious} aria-label="Previous video" title={previousTitle}><SkipBack size={18} /></button>
          <button
            className={`player-play ${isBuffering ? 'is-buffering' : ''}`}
            disabled={!canControlPlayback}
            onClick={onTogglePlay}
            aria-label={isPlaying ? 'Pause' : 'Play'}
          >
            {isBuffering ? <LoaderCircle size={17} className="spin-icon" /> : isPlaying ? <Pause size={17} fill="currentColor" /> : <Play size={17} fill="currentColor" />}
          </button>
          <button disabled={!canUseNext} onClick={onNext} aria-label="Next video" title={canUseNext ? 'Next video' : 'End of queue'}><SkipForward size={18} /></button>
        </div>
        <div className="player-progress">
          <span>{formatSeconds(displayedPosition)}</span>
          <div className={`progress-track ${canSeek ? '' : 'is-disabled'}`}>
            <i style={{ width: `${progressPercent}%` }} />
            <b className="seek-thumb" style={{ left: `${progressPercent}%` }} />
            <input
              className="seek-range"
              type="range"
              min={0}
              max={Math.max(1, Math.ceil(castDuration))}
              step={1}
              aria-label="Seek playback"
              disabled={!canSeek}
              value={Math.round(Math.min(displayedPosition, castDuration || 0))}
              onChange={(event) => onSeekPreview(Number(event.target.value))}
              onPointerUp={onCommitSeek}
              onPointerCancel={onCancelSeek}
              onKeyUp={onCommitSeek}
              onBlur={onCommitSeek}
            />
          </div>
          <span>{formatSeconds(castDuration)}</span>
        </div>
      </div>

      <div className="player-device">
        <button className="player-destination" onClick={() => void onOpenConnect()} title="Open Connect" aria-label={`${transportLabel} destination: ${castDevice ?? 'no device connected'}. Open Connect.`}>
          <Cast size={18} className={castConnected ? 'casting' : ''} />
          <span>{castDevice ?? 'No Cast device'}</span>
        </button>
        <div className="player-volume">
          <button
            aria-label={muted ? 'Unmute' : 'Mute'}
            disabled={!canUseVolume}
            onClick={onMute}
          >{muted ? <VolumeX size={16} /> : <Volume2 size={16} />}</button>
          <input
            type="range"
            min={0}
            max={1}
            step={0.01}
            aria-label="Receiver volume"
            disabled={!canUseVolume}
            value={volume}
            onChange={(event) => onVolume(Number(event.target.value))}
          />
        </div>
      </div>

      <div className="player-disclaimer"><span /> {castConnected && castHasMedia
        ? `PLAYBACK CONTROLLED THROUGH ${transportLabel.toUpperCase()}`
        : `PLAYER READY WHEN ${transportLabel.toUpperCase()} IS CONNECTED`}</div>
    </div>
  )
}
