export interface MonitorNotification {
  title: string
  message: string
  kind: string
  key: string
}

export interface ActionChoiceView {
  id: string
  label: string
}

export interface JumpTarget {
  source: string
  session_id: string
  repo_path?: string
}

export interface PendingActionView {
  action_id: string
  source: string
  session_id: string
  kind: string
  title: string
  body: string
  choices: ActionChoiceView[]
  expires_at?: number
  jump_target: JumpTarget
}

export interface MonitorEventView {
  ts_ms: number
  type: string
  state_hint: string
  text?: string
  files_touched: string[]
}

export interface IslandAgentView {
  key: string
  source: string
  session_id: string
  agent_id: string
  display_name: string
  state: 'idle' | 'thinking' | 'running' | 'waiting' | 'done' | 'error' | string
  last_ts_ms: number
  last_text?: string
  last_activity_text: string
  repo_path?: string
  files_touched: string[]
  alerts: Array<{ kind: string; message: string; ts_ms: number }>
  recent_events: MonitorEventView[]
  read_only: boolean
  can_reply: boolean
  jump_target: JumpTarget
}

export interface IslandSnapshot {
  summary: {
    total: number
    active: number
    waiting: number
    done: number
    error: number
    pr_pending: number
    alerts: number
  }
  agents: IslandAgentView[]
  pending_actions: PendingActionView[]
  now_ms: number
}

export interface MonitorSettings {
  enabled: boolean
  enableClaude: boolean
  enableOpencode: boolean
  enableCodex: boolean
  enableGit: boolean
  enablePr: boolean
  flushIntervalMs: number
  sourcePollIntervalMs: number
  gitPollIntervalMs: number
  prPollIntervalMs: number
  agentLabelFontPx: number
  maxIdleAgents: number
}

export interface IslandBootstrap {
  socketPath: string
  claudeAvailable: boolean
  monitorSettings: MonitorSettings
  windowMode: 'collapsed' | 'expanded'
}

export interface IslandTickPayload {
  snapshot: IslandSnapshot
  notifications: MonitorNotification[]
}

export type SessionTitleMap = Record<string, string>
