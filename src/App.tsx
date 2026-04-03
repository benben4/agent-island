import { getCurrentWindow } from '@tauri-apps/api/window'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { IslandShell } from './components/IslandShell.js'
import { MarqueeText } from './components/MarqueeText.js'
import { RichText } from './components/RichText.js'
import { SessionDetail } from './components/SessionDetail.js'
import { SessionStack } from './components/SessionStack.js'
import { useAudioNotifications } from './hooks/useAudioNotifications.js'
import {
  islandBootstrap,
  islandJumpToSession,
  islandLaunchAgent,
  islandPerformAction,
  islandSetWindowMode,
  islandTitleMap,
  islandTick,
} from './lib/bridge.js'
import type {
  IslandBootstrap,
  IslandSnapshot,
  IslandAgentView,
  MonitorNotification,
  PendingActionView,
  SessionTitleMap,
} from './lib/types.js'
import { previewText, sessionTitle } from './lib/sessionText.js'
import { animateWindowMode } from './lib/windowMotion.js'

const EMPTY_SNAPSHOT: IslandSnapshot = {
  summary: { total: 0, active: 0, waiting: 0, done: 0, error: 0, pr_pending: 0, alerts: 0 },
  agents: [],
  pending_actions: [],
  now_ms: 0,
}

type LayoutMode = 'regular' | 'condensed' | 'compact'

function headerFromSnapshot(
  snapshot: IslandSnapshot,
  activeAgent: IslandAgentView | null,
  titleMap: SessionTitleMap,
  titleRunning: boolean,
) {
  const titleText = activeAgent ? sessionTitle(activeAgent, titleMap) : ''
  return (
    <>
      <div className="headline">
        <span className="headline-count">{snapshot.summary.active} active</span>
        {snapshot.pending_actions.length > 0 ? <span className="headline-alert">{snapshot.pending_actions.length} waiting</span> : null}
        {activeAgent ? <MarqueeText text={titleText} running={titleRunning} className="headline-session" /> : null}
      </div>
      <div className="headline-preview" title={previewText(activeAgent, titleMap)}>
        <RichText text={previewText(activeAgent, titleMap)} />
      </div>
    </>
  )
}

export default function App() {
  const [bootstrap, setBootstrap] = useState<IslandBootstrap | null>(null)
  const [snapshot, setSnapshot] = useState<IslandSnapshot>(EMPTY_SNAPSHOT)
  const [selectedKey, setSelectedKey] = useState<string | null>(null)
  const [expanded, setExpanded] = useState(false)
  const [manualExpanded, setManualExpanded] = useState(false)
  const [notifications, setNotifications] = useState<MonitorNotification[]>([])
  const [titleMap, setTitleMap] = useState<SessionTitleMap>({})
  const [diagnosticsEnabled, setDiagnosticsEnabled] = useState(() => {
    if (typeof window === 'undefined') {
      return false
    }
    return window.location.search.includes('diagnostics=1')
      || window.localStorage.getItem('agent-island-diagnostics') === '1'
  })
  const [diagnosticLines, setDiagnosticLines] = useState<string[]>([])
  const [viewport, setViewport] = useState(() => ({
    width: typeof window === 'undefined' ? 880 : window.innerWidth,
    height: typeof window === 'undefined' ? 620 : window.innerHeight,
  }))
  const collapseTimerRef = useRef<number | null>(null)

  useAudioNotifications(notifications, false)

  useEffect(() => {
    const onResize = () => {
      setViewport({ width: window.innerWidth, height: window.innerHeight })
    }
    onResize()
    void getCurrentWindow().setBackgroundColor([0, 0, 0, 0])
    const onKeyDown = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || !event.shiftKey || event.key.toLowerCase() !== 'd') {
        return
      }
      event.preventDefault()
      setDiagnosticsEnabled((current) => {
        const next = !current
        window.localStorage.setItem('agent-island-diagnostics', next ? '1' : '0')
        return next
      })
    }
    window.addEventListener('resize', onResize)
    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('resize', onResize)
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [])

  const pushDiagnostic = useCallback((message: string) => {
    const line = `${new Date().toLocaleTimeString()} ${message}`
    setDiagnosticLines((current) => [...current.slice(-7), line])
  }, [])

  const layoutMode = useMemo<LayoutMode>(() => {
    if (viewport.width < 640 || viewport.height < 500) {
      return 'compact'
    }
    if (viewport.width < 860 || viewport.height < 620) {
      return 'condensed'
    }
    return 'regular'
  }, [viewport.height, viewport.width])

  const refresh = useCallback(async (): Promise<void> => {
    const [payload, nextTitleMap] = await Promise.all([islandTick(), islandTitleMap()])
    setSnapshot(payload.snapshot)
    setNotifications(payload.notifications)
    setTitleMap(nextTitleMap)
    setSelectedKey((prev) => {
      if (prev && payload.snapshot.agents.some((agent) => agent.key === prev)) {
        return prev
      }
      return payload.snapshot.agents[0]?.key ?? null
    })

    const hasRunningAgents = payload.snapshot.agents.some(
      (agent) => agent.state === 'running' || agent.state === 'thinking',
    )
    const shouldAutoExpand = payload.snapshot.pending_actions.length > 0
      || payload.notifications.some((notification) => notification.kind === 'done' || notification.kind === 'error' || notification.kind === 'actionable')
    const shouldAutoCollapse = !hasRunningAgents && payload.snapshot.pending_actions.length === 0

    if (shouldAutoExpand) {
      if (!expanded) {
        pushDiagnostic('mode:auto-expand')
        setExpanded(true)
        void islandSetWindowMode('expanded')
        void animateWindowMode('expanded')
      }
      if (collapseTimerRef.current !== null) {
        window.clearTimeout(collapseTimerRef.current)
        collapseTimerRef.current = null
      }
      return
    }

    if (!manualExpanded && shouldAutoCollapse) {
      if (collapseTimerRef.current !== null) {
        window.clearTimeout(collapseTimerRef.current)
      }
      collapseTimerRef.current = window.setTimeout(() => {
        if (expanded) {
          pushDiagnostic('mode:auto-collapse')
          setExpanded(false)
          void islandSetWindowMode('collapsed')
          void animateWindowMode('collapsed')
        }
      }, 4500)
      return
    }

    if (collapseTimerRef.current !== null) {
      window.clearTimeout(collapseTimerRef.current)
      collapseTimerRef.current = null
    }
  }, [expanded, manualExpanded, pushDiagnostic])

  useEffect(() => {
    let cancelled = false
    async function start(): Promise<void> {
      const info = await islandBootstrap()
      if (cancelled) {
        return
      }
      setBootstrap(info)
      setExpanded(info.windowMode === 'expanded')
      setManualExpanded(info.windowMode === 'expanded')
      await refresh()
    }
    void start()
    const timer = window.setInterval(() => {
      void refresh()
    }, 2000)
    return () => {
      cancelled = true
      clearInterval(timer)
      if (collapseTimerRef.current !== null) {
        window.clearTimeout(collapseTimerRef.current)
      }
    }
  }, [refresh])

  const selectedAgent = useMemo<IslandAgentView | null>(
    () => snapshot.agents.find((agent) => agent.key === selectedKey) ?? snapshot.agents[0] ?? null,
    [selectedKey, snapshot.agents],
  )
  const selectedActions = useMemo<PendingActionView[]>(
    () => selectedAgent
      ? snapshot.pending_actions.filter((action) => `${action.source}:${action.session_id}` === selectedAgent.key)
      : snapshot.pending_actions,
    [selectedAgent, snapshot.pending_actions],
  )

  async function handleChoose(action: PendingActionView, choiceId: string): Promise<void> {
    await islandPerformAction(action.action_id, choiceId)
    await refresh()
  }

  async function handleToggle(): Promise<void> {
    const next = !expanded
    pushDiagnostic(`mode:toggle ${next ? 'expanded' : 'collapsed'}`)
    setExpanded(next)
    setManualExpanded(next)
    await islandSetWindowMode(next ? 'expanded' : 'collapsed')
    await animateWindowMode(next ? 'expanded' : 'collapsed')
  }

  async function handleCollapse(): Promise<void> {
    if (collapseTimerRef.current !== null) {
      window.clearTimeout(collapseTimerRef.current)
      collapseTimerRef.current = null
    }
    pushDiagnostic('mode:manual-collapse')
    setExpanded(false)
    setManualExpanded(false)
    await islandSetWindowMode('collapsed')
    await animateWindowMode('collapsed')
  }

  const hasAnyAgents = snapshot.agents.length > 0
  const showToolbar = expanded || hasAnyAgents
  const showCollapseButton = expanded
  const focusAgent = selectedAgent ?? snapshot.agents[0] ?? null
  const titleRunning = Boolean(focusAgent && (focusAgent.state === 'running' || focusAgent.state === 'thinking'))

  return (
    <main className="app-shell">
      <IslandShell
        expanded={expanded}
        onToggle={() => void handleToggle()}
        title="Agent Island"
        titleRunning={titleRunning}
        header={headerFromSnapshot(snapshot, focusAgent, titleMap, titleRunning)}
        headerActions={(
          <div className="header-button-row">
            <button
              type="button"
              className="ghost-button"
              onClick={() => {
                setDiagnosticsEnabled((current) => {
                  const next = !current
                  window.localStorage.setItem('agent-island-diagnostics', next ? '1' : '0')
                  return next
                })
              }}
            >
              {diagnosticsEnabled ? 'Hide Diagnostics' : 'Diagnostics'}
            </button>
            {showCollapseButton ? (
              <button type="button" className="collapse-button" onClick={() => void handleCollapse()}>
                Collapse
              </button>
            ) : null}
          </div>
        )}
        layoutMode={layoutMode}
        collapsedContent={(
          <div className="collapsed-preview">
            <div className="collapsed-preview-label">
              {focusAgent ? (
                <MarqueeText text={sessionTitle(focusAgent, titleMap)} running={titleRunning} className="collapsed-title" />
              ) : 'No active session'}
            </div>
            <div className="collapsed-preview-scroll" title={previewText(focusAgent, titleMap)}>
              <RichText text={previewText(focusAgent, titleMap)} />
            </div>
          </div>
        )}
        onDiagnostics={pushDiagnostic}
      >
        {showToolbar ? (
          <div className="toolbar">
            <button type="button" className="toolbar-button" onClick={() => void islandLaunchAgent('codex')}>
              New Codex
            </button>
            <button type="button" className="toolbar-button" onClick={() => void islandLaunchAgent('opencode')}>
              New OpenCode
            </button>
            <button
              type="button"
              className="toolbar-button"
              disabled={!bootstrap?.claudeAvailable}
              onClick={() => void islandLaunchAgent('claude')}
            >
              New Claude
            </button>
          </div>
        ) : null}
        <div className="panel-grid">
          <SessionStack
            agents={snapshot.agents}
            selectedKey={selectedKey}
            pendingActions={snapshot.pending_actions}
            titleMap={titleMap}
            onSelect={setSelectedKey}
            onJump={(agent) => void islandJumpToSession(agent.jump_target)}
          />
          <SessionDetail
            agent={selectedAgent}
            actions={selectedActions}
            titleMap={titleMap}
            onChoose={(action, choiceId) => void handleChoose(action, choiceId)}
          />
        </div>
      </IslandShell>
      {diagnosticsEnabled ? (
        <aside className="diagnostics-panel">
          <div className="diagnostics-title">Diagnostics</div>
          <div className="diagnostics-row">{`mode=${expanded ? 'expanded' : 'collapsed'} manual=${manualExpanded}`}</div>
          <div className="diagnostics-row">{`viewport=${Math.round(viewport.width)}x${Math.round(viewport.height)}`}</div>
          <div className="diagnostics-row">{`agent=${focusAgent?.session_id ?? 'none'} state=${focusAgent?.state ?? 'none'}`}</div>
          <div className="diagnostics-row">{`notifications=${notifications.length} actions=${snapshot.pending_actions.length}`}</div>
          <div className="diagnostics-log">
            {diagnosticLines.map((line) => (
              <div key={line} className="diagnostics-row">{line}</div>
            ))}
          </div>
        </aside>
      ) : null}
    </main>
  )
}
