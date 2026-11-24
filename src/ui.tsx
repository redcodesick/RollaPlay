import { AlertCircle, Check, Info, TriangleAlert } from 'lucide-react'
import type { Phase5Notification } from './playlist'

export function BrandMark({ size = 32, title = false }: { size?: number; title?: boolean }) {
  return (
    <span className={`rolla-mark${title ? ' rolla-mark-title' : ''}`} style={{ width: size, height: size }} aria-hidden="true">
      <svg viewBox="0 0 32 32" role="presentation" focusable="false">
        <path d="M7 24V8h9.1c4.4 0 7 2.4 7 6.1 0 2.7-1.4 4.6-3.8 5.5L24 24h-5.2l-3.5-4.2h-3.2V24H7Zm4.3-7.5h4.4c1.8 0 2.9-.9 2.9-2.4 0-1.5-1.1-2.3-2.9-2.3h-4.4v4.7Z" fill="currentColor" />
        <path d="M21.8 17.4 25 21.9h-3.9l-2.5-3.3" fill="var(--orange)" />
      </svg>
    </span>
  )
}

export function NoticeIcon({ level }: { level: Phase5Notification['level'] }) {
  if (level === 'error') return <AlertCircle size={16} />
  if (level === 'warning') return <TriangleAlert size={16} />
  if (level === 'success') return <Check size={16} />
  return <Info size={16} />
}
