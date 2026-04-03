import { useEffect, useRef } from 'react'
import type { PointerEvent, ReactNode } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { MarqueeText } from './MarqueeText.js'

interface IslandShellProps {
  expanded: boolean
  children: ReactNode
  onToggle: () => void
  title: string
  titleRunning?: boolean
  header: ReactNode
  headerActions?: ReactNode
  layoutMode?: 'regular' | 'condensed' | 'compact'
  collapsedContent?: ReactNode
  onDiagnostics?: (message: string) => void
}

function isInteractiveTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) {
    return false
  }
  return Boolean(
    target.closest('button, a, input, textarea, select, option, summary, .inline-link'),
  )
}

export function IslandShell({
  expanded,
  children,
  onToggle,
  title,
  titleRunning = false,
  header,
  headerActions,
  layoutMode = 'regular',
  collapsedContent,
  onDiagnostics,
}: IslandShellProps) {
  const dragStateRef = useRef<{
    pointerId: number
    startX: number
    startY: number
    dragging: boolean
    onClick?: () => void
  } | null>(null)

  useEffect(() => () => {
    dragStateRef.current = null
  }, [])

  const startDrag = (event: PointerEvent<HTMLElement>, onClick?: () => void) => {
    if (event.button !== 0 || isInteractiveTarget(event.target)) {
      return
    }
    event.preventDefault()
    event.stopPropagation()
    event.currentTarget.setPointerCapture(event.pointerId)
    dragStateRef.current = {
      pointerId: event.pointerId,
      startX: event.screenX,
      startY: event.screenY,
      dragging: false,
      onClick,
    }
    onDiagnostics?.(`drag:down ${event.screenX},${event.screenY}`)
  }

  const continueDrag = (event: PointerEvent<HTMLElement>) => {
    const state = dragStateRef.current
    if (!state || state.pointerId !== event.pointerId) {
      return
    }
    const deltaX = event.screenX - state.startX
    const deltaY = event.screenY - state.startY
    if (!state.dragging && Math.hypot(deltaX, deltaY) >= 6) {
      state.dragging = true
      onDiagnostics?.(`drag:start ${Math.round(deltaX)},${Math.round(deltaY)}`)
      void getCurrentWindow().startDragging().catch((error: unknown) => {
        onDiagnostics?.(`drag:error ${String(error)}`)
      })
    }
  }

  const finishDrag = (event: PointerEvent<HTMLElement>) => {
    const state = dragStateRef.current
    if (!state || state.pointerId !== event.pointerId) {
      return
    }
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId)
    }
    dragStateRef.current = null
    if (!state.dragging) {
      onDiagnostics?.('drag:click')
      state.onClick?.()
      return
    }
    onDiagnostics?.('drag:end')
  }

  return (
    <div className={`island-shell ${expanded ? 'expanded' : 'collapsed'} layout-${layoutMode}`}>
      <div className="island-top-slot">
      <div
        className="island-hitbox"
        role="button"
        tabIndex={0}
        aria-label="Toggle Agent Island"
        onPointerDown={(event) => {
          startDrag(event, onToggle)
        }}
        onPointerMove={continueDrag}
        onPointerUp={finishDrag}
        onPointerCancel={finishDrag}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault()
            onToggle()
          }
        }}
      >
        <span className="notch-pill">
          <MarqueeText text={title} running={titleRunning} className="notch-title" />
        </span>
      </div>
      </div>
      <div className="island-surface">
        <div className="island-header">
          <div
            className="island-header-main window-drag-handle"
            onPointerDown={(event) => {
              startDrag(event)
            }}
            onPointerMove={continueDrag}
            onPointerUp={finishDrag}
            onPointerCancel={finishDrag}
          >
            {header}
          </div>
          {headerActions ? <div className="island-header-actions">{headerActions}</div> : null}
        </div>
        <div className={`island-body ${expanded ? 'island-body-visible' : 'island-body-hidden'}`}>{children}</div>
        {collapsedContent ? (
          <div
            className={`island-collapsed-body window-drag-handle ${expanded ? 'island-collapsed-hidden' : 'island-collapsed-visible'}`}
            onPointerDown={(event) => {
              startDrag(event)
            }}
            onPointerMove={continueDrag}
            onPointerUp={finishDrag}
            onPointerCancel={finishDrag}
          >
            {collapsedContent}
          </div>
        ) : null}
      </div>
    </div>
  )
}
