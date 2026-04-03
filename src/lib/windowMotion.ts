import { LogicalPosition, LogicalSize } from '@tauri-apps/api/dpi'
import { getCurrentWindow } from '@tauri-apps/api/window'

export const WINDOW_SIZES = {
  collapsed: { width: 580, height: 164 },
  expanded: { width: 1180, height: 780 },
} as const

const WINDOW_ANIMATION_MS = 500

function easeInOutCubic(t: number): number {
  return t < 0.5 ? 4 * t * t * t : 1 - ((-2 * t + 2) ** 3) / 2
}

async function currentWindowFrame() {
  const appWindow = getCurrentWindow()
  const scale = await appWindow.scaleFactor()
  const position = (await appWindow.outerPosition()).toLogical(scale)
  const size = (await appWindow.innerSize()).toLogical(scale)
  return {
    appWindow,
    x: position.x,
    y: position.y,
    width: size.width,
    height: size.height,
  }
}

export async function animateWindowMode(mode: keyof typeof WINDOW_SIZES): Promise<void> {
  const frame = await currentWindowFrame()
  const target = WINDOW_SIZES[mode]
  const start = {
    x: frame.x,
    y: frame.y,
    width: frame.width,
    height: frame.height,
  }
  const targetFrame = {
    x: start.x + (start.width - target.width) / 2,
    y: start.y,
    width: target.width,
    height: target.height,
  }

  const startedAt = performance.now()

  await new Promise<void>((resolve) => {
    const tick = () => {
      const elapsed = performance.now() - startedAt
      const progress = Math.min(1, elapsed / WINDOW_ANIMATION_MS)
      const eased = easeInOutCubic(progress)
      const width = start.width + (targetFrame.width - start.width) * eased
      const height = start.height + (targetFrame.height - start.height) * eased
      const x = start.x + (targetFrame.x - start.x) * eased
      const y = start.y + (targetFrame.y - start.y) * eased

      void frame.appWindow.setSize(new LogicalSize(width, height))
      void frame.appWindow.setPosition(new LogicalPosition(x, y))

      if (progress < 1) {
        requestAnimationFrame(tick)
        return
      }
      resolve()
    }

    requestAnimationFrame(tick)
  })
}

interface DragStartOptions {
  onClick?: () => void
  onDiagnostics?: (message: string) => void
}

export function beginWindowDrag(
  originScreenX: number,
  originScreenY: number,
  options: DragStartOptions = {},
): () => void {
  const appWindow = getCurrentWindow()
  let basePosition: { x: number; y: number } | null = null
  let scaleFactor = 1
  let moved = false
  let lastApplied = { x: originScreenX, y: originScreenY }
  let active = true

  options.onDiagnostics?.(`drag:start ${originScreenX},${originScreenY}`)

  void (async () => {
    scaleFactor = await appWindow.scaleFactor()
    const position = (await appWindow.outerPosition()).toLogical(scaleFactor)
    if (!active) {
      return
    }
    basePosition = { x: position.x, y: position.y }
    if (moved) {
      void appWindow.setPosition(
        new LogicalPosition(
          basePosition.x + ((lastApplied.x - originScreenX) / scaleFactor),
          basePosition.y + ((lastApplied.y - originScreenY) / scaleFactor),
        ),
      )
    }
  })()

  const onPointerMove = (event: PointerEvent) => {
    const deltaX = event.screenX - originScreenX
    const deltaY = event.screenY - originScreenY
    if (!moved && Math.hypot(deltaX, deltaY) >= 4) {
      moved = true
      options.onDiagnostics?.(`drag:move ${Math.round(deltaX)},${Math.round(deltaY)}`)
    }
    if (!moved) {
      return
    }
    event.preventDefault()
    lastApplied = { x: event.screenX, y: event.screenY }
    if (!basePosition) {
      return
    }
    void appWindow.setPosition(
      new LogicalPosition(
        basePosition.x + (deltaX / scaleFactor),
        basePosition.y + (deltaY / scaleFactor),
      ),
    )
  }

  const finish = () => {
    active = false
    window.removeEventListener('pointermove', onPointerMove)
    window.removeEventListener('pointerup', onPointerUp)
    window.removeEventListener('pointercancel', onPointerUp)
    if (!moved) {
      options.onDiagnostics?.('drag:click')
      options.onClick?.()
      return
    }
    options.onDiagnostics?.('drag:end')
  }

  const onPointerUp = () => {
    finish()
  }

  window.addEventListener('pointermove', onPointerMove, { passive: false })
  window.addEventListener('pointerup', onPointerUp, { passive: true })
  window.addEventListener('pointercancel', onPointerUp, { passive: true })

  return finish
}
