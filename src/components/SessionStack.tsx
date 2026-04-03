import type { IslandAgentView, PendingActionView, SessionTitleMap } from '../lib/types.js'
import { sessionActivityText, sessionTitle } from '../lib/sessionText.js'

interface SessionStackProps {
  agents: IslandAgentView[]
  selectedKey: string | null
  pendingActions: PendingActionView[]
  titleMap: SessionTitleMap
  onSelect: (key: string) => void
  onJump: (agent: IslandAgentView) => void
}

export function SessionStack({ agents, selectedKey, pendingActions, titleMap, onSelect, onJump }: SessionStackProps) {
  const actionCountBySession = new Map<string, number>()
  for (const action of pendingActions) {
    const key = `${action.source}:${action.session_id}`
    actionCountBySession.set(key, (actionCountBySession.get(key) ?? 0) + 1)
  }

  return (
    <div className="session-stack">
      {agents.map((agent) => {
        const actionCount = actionCountBySession.get(agent.key) ?? 0
        return (
          <button
            key={agent.key}
            type="button"
            className={`session-card ${selectedKey === agent.key ? 'selected' : ''}`}
            onClick={() => onSelect(agent.key)}
            >
            <div className="session-topline">
              <span className={`status-dot ${agent.state}`} />
              <span className="session-title">{sessionTitle(agent, titleMap)}</span>
              <span className="session-badge">{agent.source}</span>
            </div>
            <div className="session-text">{sessionActivityText(agent, agent.last_activity_text, titleMap)}</div>
            <div className="session-meta">
              <span>{agent.read_only ? 'Read-only' : 'Reply-ready'}</span>
              {actionCount > 0 ? <span>{actionCount} action{actionCount > 1 ? 's' : ''}</span> : null}
              <button
                type="button"
                className="ghost-button"
                onClick={(event) => {
                  event.stopPropagation()
                  onJump(agent)
                }}
              >
                Jump
              </button>
            </div>
          </button>
        )
      })}
    </div>
  )
}
