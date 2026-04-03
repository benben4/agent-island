import { useEffect, useLayoutEffect, useMemo, useRef } from 'react'
import type { IslandAgentView, PendingActionView, SessionTitleMap } from '../lib/types.js'
import { sessionActivityText, sessionEventText, sessionTitle } from '../lib/sessionText.js'
import { ActionTray } from './ActionTray.js'
import { RichText } from './RichText.js'

interface SessionDetailProps {
  agent: IslandAgentView | null
  actions: PendingActionView[]
  titleMap: SessionTitleMap
  onChoose: (action: PendingActionView, choiceId: string) => void
}

export function SessionDetail({ agent, actions, titleMap, onChoose }: SessionDetailProps) {
  const scrollRef = useRef<HTMLDivElement | null>(null)
  const shouldStickToBottomRef = useRef(true)
  const lastTailKeyRef = useRef<string | null>(null)

  const visibleEvents = useMemo(() => {
    if (!agent) {
      return []
    }
    const chronological = [...agent.recent_events].reverse()
    const deduped = new Map<string, { key: string; stateHint: string; text: string }>()

    for (const event of chronological) {
      const text = sessionEventText(event.text ?? event.type)
      if (!text) {
        continue
      }
      const dedupeKey = text.toLowerCase()
      deduped.set(dedupeKey, {
        key: `${event.ts_ms}:${event.type}:${event.text ?? ''}`,
        stateHint: event.state_hint,
        text,
      })
    }

    return Array.from(deduped.values()).slice(-12)
  }, [agent])

  useEffect(() => {
    shouldStickToBottomRef.current = true
    lastTailKeyRef.current = null
  }, [agent?.key])

  useEffect(() => {
    const node = scrollRef.current
    if (!node) {
      return
    }
    const onScroll = () => {
      const distanceFromBottom = node.scrollHeight - node.scrollTop - node.clientHeight
      shouldStickToBottomRef.current = distanceFromBottom < 24
    }
    onScroll()
    node.addEventListener('scroll', onScroll, { passive: true })
    return () => {
      node.removeEventListener('scroll', onScroll)
    }
  }, [agent?.key])

  useLayoutEffect(() => {
    const node = scrollRef.current
    if (!node) {
      return
    }
    const tailKey = visibleEvents.at(-1)?.key ?? null
    const tailChanged = tailKey !== null && tailKey !== lastTailKeyRef.current
    if (tailChanged && shouldStickToBottomRef.current) {
      node.scrollTop = node.scrollHeight
    }
    lastTailKeyRef.current = tailKey
  }, [visibleEvents])

  if (!agent) {
    return (
      <div className="session-detail empty">
        <div className="section-label">Session detail</div>
        <p>Select a session to inspect recent activity and reply options.</p>
      </div>
    )
  }

  return (
    <div className="session-detail">
        <div className="section-label">Current session</div>
        <div ref={scrollRef} className="detail-scroll-region">
          <div className="detail-heading">
          <h2>{sessionTitle(agent, titleMap)}</h2>
            <span className={`status-chip ${agent.state}`}>{agent.state}</span>
          </div>
          <p className="detail-summary">
          <RichText text={sessionActivityText(agent, agent.last_activity_text, titleMap)} />
          </p>
        <div className="detail-meta">
          <span>{agent.repo_path ?? 'Repo not bound'}</span>
          <span>{agent.read_only ? 'Read-only channel' : 'Interactive channel'}</span>
        </div>
        <div className="detail-events">
          {visibleEvents.map((event) => (
            <div key={event.key} className="event-row">
              <span className={`event-state ${event.stateHint}`}>{event.stateHint}</span>
              <span className="event-text">
                <RichText text={event.text} />
              </span>
            </div>
          ))}
        </div>
        <ActionTray actions={actions} onChoose={onChoose} />
      </div>
    </div>
  )
}
