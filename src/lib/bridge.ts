import { invoke } from '@tauri-apps/api/core'
import type { IslandBootstrap, IslandTickPayload, JumpTarget, MonitorSettings, SessionTitleMap } from './types.js'

export function islandBootstrap(): Promise<IslandBootstrap> {
  return invoke<IslandBootstrap>('island_bootstrap')
}

export function islandTick(): Promise<IslandTickPayload> {
  return invoke<IslandTickPayload>('island_tick')
}

export function islandTitleMap(): Promise<SessionTitleMap> {
  return invoke<SessionTitleMap>('island_title_map')
}

export function islandPerformAction(actionId: string, choiceId: string): Promise<void> {
  return invoke('island_perform_action', { actionId, choiceId })
}

export function islandJumpToSession(target: JumpTarget): Promise<void> {
  return invoke('island_jump_to_session', {
    source: target.source,
    sessionId: target.session_id,
    repoPath: target.repo_path ?? null,
  })
}

export function islandLaunchAgent(source: 'claude' | 'codex' | 'opencode', cwd?: string): Promise<void> {
  return invoke('island_launch_agent', { source, cwd: cwd ?? null })
}

export function islandSetWindowMode(mode: 'collapsed' | 'expanded'): Promise<void> {
  return invoke('island_set_window_mode', { mode })
}

export function islandSetMonitorSettings(settings: MonitorSettings): Promise<void> {
  return invoke('island_set_monitor_settings', { settings })
}

export function islandOpenPath(path: string): Promise<void> {
  return invoke('island_open_path', { path })
}
