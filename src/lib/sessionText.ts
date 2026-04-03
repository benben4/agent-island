import type { IslandAgentView, SessionTitleMap } from './types.js'

const GENERIC_ACTIVITY = new Set([
  'Thinking',
  'Assistant message',
  'Tool output',
  'Turn completed',
  'Waiting for input',
  'Task started',
  'Step started',
  'Session activity',
  'No recent activity',
  'Codex error',
  'Idle',
])

function prettifySource(source: string): string {
  if (source === 'opencode') {
    return 'OpenCode'
  }
  if (source === 'codex') {
    return 'Codex'
  }
  if (source === 'claude') {
    return 'Claude'
  }
  return source.charAt(0).toUpperCase() + source.slice(1)
}

function sourceFallback(agent: IslandAgentView): string {
  return `${prettifySource(agent.source)} app`
}

function normalizeWhitespace(text: string): string {
  return text.replace(/\s+/g, ' ').trim()
}

function titleLine(text: string, maxLength = 28): string {
  const firstLine = text
    .split('\n')
    .map((line) => line.trim())
    .find(Boolean)
    ?? text.trim()
  const cleaned = normalizeWhitespace(firstLine.replace(/^[-*•]\s*/, '').replace(/^\d+\.\s*/, ''))
  if (cleaned.length <= maxLength) {
    return cleaned
  }
  return `${cleaned.slice(0, maxLength - 1).trimEnd()}…`
}

function explicitDisplayTitle(agent: IslandAgentView, titleMap?: SessionTitleMap): string | null {
  const mappedTitle = titleMap?.[agent.session_id]?.trim()
  if (mappedTitle) {
    return mappedTitle
  }
  const trimmed = agent.display_name.trim()
  const sourcePrefix = `${agent.source}: `
  if (!trimmed.toLowerCase().startsWith(sourcePrefix)) {
    return trimmed || null
  }
  const rest = trimmed.slice(sourcePrefix.length).trim()
  if (!rest || /^[0-9a-f-]{6,}$/i.test(rest)) {
    return null
  }
  return rest
}

function meaningfulText(text: string | undefined): string | null {
  if (!text) {
    return null
  }

  const trimmed = text.trim()
  if (!trimmed) {
    return null
  }
  if (GENERIC_ACTIVITY.has(trimmed)) {
    return null
  }

  return normalizeToolStatus(trimmed)
}

function normalizeToolStatus(text: string): string {
  const toolStatus = text.match(/^([^:]+):\s*(running|completed|error)$/i)
  if (toolStatus) {
    return toolStatus[1].trim()
  }
  return text
}

function latestMeaningfulText(agent: IslandAgentView): string | null {
  const direct = meaningfulText(agent.last_activity_text) ?? meaningfulText(agent.last_text)
  if (direct) {
    return direct
  }
  for (const event of agent.recent_events) {
    const candidate = meaningfulText(event.text ?? event.type)
    if (candidate) {
      return candidate
    }
  }
  return null
}

function earliestUserTaskText(agent: IslandAgentView): string | null {
  for (let index = agent.recent_events.length - 1; index >= 0; index -= 1) {
    const event = agent.recent_events[index]
    if (event.state_hint !== 'waiting') {
      continue
    }
    const candidate = meaningfulText(event.text ?? event.type)
    if (!candidate) {
      continue
    }
    return candidate
  }
  return null
}

export function sessionTitle(agent: IslandAgentView | null, titleMap?: SessionTitleMap): string {
  if (!agent) {
    return 'No session'
  }
  const explicit = explicitDisplayTitle(agent, titleMap)
  if (explicit) {
    return explicit
  }
  const userTask = earliestUserTaskText(agent)
  if (userTask) {
    return titleLine(userTask)
  }
  return sourceFallback(agent)
}

export function sessionActivityText(agent: IslandAgentView | null, text?: string, titleMap?: SessionTitleMap): string {
  if (!agent) {
    return 'No recent activity.'
  }
  const candidate = meaningfulText(text)
  if (candidate) {
    return candidate
  }
  return latestMeaningfulText(agent) ?? explicitDisplayTitle(agent, titleMap) ?? sourceFallback(agent)
}

export function previewText(agent: IslandAgentView | null, titleMap?: SessionTitleMap): string {
  if (!agent) {
    return 'No recent activity.'
  }
  return normalizeWhitespace(sessionActivityText(agent, agent.last_activity_text, titleMap)) || 'No recent activity.'
}

export function sessionEventText(text: string | undefined): string | null {
  if (!text) {
    return null
  }
  const trimmed = text.trim()
  if (!trimmed) {
    return null
  }
  if (GENERIC_ACTIVITY.has(trimmed)) {
    return null
  }
  return normalizeToolStatus(trimmed)
}
