import type { PendingActionView } from '../lib/types.js'

interface ActionTrayProps {
  actions: PendingActionView[]
  onChoose: (action: PendingActionView, choiceId: string) => void
}

export function ActionTray({ actions, onChoose }: ActionTrayProps) {
  if (actions.length === 0) {
    return null
  }

  return (
    <div className="action-tray">
      <div className="section-label">Pending actions</div>
      {actions.map((action) => (
        <div key={action.action_id} className="action-card">
          <div className="action-kind">{action.kind}</div>
          <div className="action-title">{action.title}</div>
          <p className="action-body">{action.body}</p>
          <div className="action-buttons">
            {action.choices.map((choice) => (
              <button
                key={choice.id}
                type="button"
                className="action-button"
                onClick={() => onChoose(action, choice.id)}
              >
                {choice.label}
              </button>
            ))}
          </div>
        </div>
      ))}
    </div>
  )
}
