import { describe, expect, it } from 'vitest'
import { hasPlayableQueueEntry, type EntryStatus, type PlaybackSnapshot, type QueueEntry } from './playlist'

function snapshot(entries: Array<Pick<QueueEntry, 'availability_state' | 'status'>>): PlaybackSnapshot {
  const queue: QueueEntry[] = entries.map((entry, index) => ({
    entry_id: `entry-${index}`,
    media_file_id: index,
    filename: `video-${index}`,
    availability_state: entry.availability_state,
    duration_seconds: null,
    status: entry.status as EntryStatus,
    position: index,
  }))
  return {
    state: 'idle',
    mode: 'manual',
    shuffle: false,
    repeat: 'off',
    queue,
    current_entry_id: null,
    next_entry_id: null,
    history: [],
    current_media_file_id: null,
    position_seconds: 0,
    duration_seconds: null,
    playback_token: null,
    cast_session_id: null,
    media_session_id: null,
    last_completion_token: null,
    last_selected_playlist_id: null,
    previous_restart_threshold_seconds: 5,
    message: null,
    last_error: null,
  }
}

describe('hasPlayableQueueEntry', () => {
  it('is false for an empty queue', () => {
    expect(hasPlayableQueueEntry(null)).toBe(false)
    expect(hasPlayableQueueEntry(snapshot([]))).toBe(false)
  })

  it('ignores unavailable entries', () => {
    expect(hasPlayableQueueEntry(snapshot([
      { availability_state: 'removed_from_library', status: 'unavailable' },
      { availability_state: 'temporarily_unavailable', status: 'pending' },
    ]))).toBe(false)
  })

  it('accepts available entries even when already completed', () => {
    expect(hasPlayableQueueEntry(snapshot([
      { availability_state: 'available', status: 'completed' },
    ]))).toBe(true)
  })
})
