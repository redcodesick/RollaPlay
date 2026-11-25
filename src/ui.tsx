import { AlertCircle, Check, Info, TriangleAlert } from 'lucide-react'
import type { Phase5Notification } from './playlist'

export function BrandMark({ size = 32, title = false }: { size?: number; title?: boolean }) {
  return (
    <span className={`rolla-mark${title ? ' rolla-mark-title' : ''}`} style={{ width: size, height: size }} aria-hidden="true">
      <img src="/rolla-mark.png" alt="" draggable={false} />
    </span>
  )
}

export function NoticeIcon({ level }: { level: Phase5Notification['level'] }) {
  if (level === 'error') return <AlertCircle size={16} />
  if (level === 'warning') return <TriangleAlert size={16} />
  if (level === 'success') return <Check size={16} />
  return <Info size={16} />
}
