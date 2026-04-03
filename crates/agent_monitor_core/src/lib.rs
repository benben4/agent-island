use dirs::home_dir;
use regex::Regex;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

const IDLE_AFTER_MS: i64 = 20_000;
const DONE_AFTER_MS: i64 = 90_000;
const CODEX_TAIL_BYTES: usize = 65_536;
const MAX_CODEX_FILES: usize = 120;
const MAX_OPENCODE_FILES: usize = 800;
const MAX_OPENCODE_PART_FILES: usize = 900;
const MAX_OPENCODE_DB_SESSIONS: usize = 800;
const MAX_OPENCODE_DB_PARTS: usize = 1500;
const MAX_MONITOR_TEXT_CHARS: usize = 180;
const MAX_RECENT_EVENTS: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MonitorSettings {
    pub enabled: bool,
    #[serde(rename = "enableClaude", default = "default_enable_true")]
    pub enable_claude: bool,
    #[serde(rename = "enableOpencode", default = "default_enable_true")]
    pub enable_opencode: bool,
    #[serde(rename = "enableCodex", default = "default_enable_true")]
    pub enable_codex: bool,
    #[serde(rename = "enableGit", default = "default_enable_true")]
    pub enable_git: bool,
    #[serde(rename = "enablePr", default = "default_enable_true")]
    pub enable_pr: bool,
    #[serde(rename = "flushIntervalMs", default = "default_flush_interval_ms")]
    pub flush_interval_ms: i64,
    #[serde(rename = "sourcePollIntervalMs", default = "default_source_poll_interval_ms")]
    pub source_poll_interval_ms: i64,
    #[serde(rename = "gitPollIntervalMs", default = "default_git_poll_interval_ms")]
    pub git_poll_interval_ms: i64,
    #[serde(rename = "prPollIntervalMs", default = "default_pr_poll_interval_ms")]
    pub pr_poll_interval_ms: i64,
    #[serde(rename = "agentLabelFontPx", default = "default_agent_label_font_px")]
    pub agent_label_font_px: i64,
    #[serde(rename = "maxIdleAgents", default = "default_max_idle_agents")]
    pub max_idle_agents: i64,
}

impl Default for MonitorSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            enable_claude: true,
            enable_opencode: true,
            enable_codex: true,
            enable_git: true,
            enable_pr: true,
            flush_interval_ms: default_flush_interval_ms(),
            source_poll_interval_ms: default_source_poll_interval_ms(),
            git_poll_interval_ms: default_git_poll_interval_ms(),
            pr_poll_interval_ms: default_pr_poll_interval_ms(),
            agent_label_font_px: default_agent_label_font_px(),
            max_idle_agents: default_max_idle_agents(),
        }
    }
}

fn default_enable_true() -> bool {
    true
}

fn default_flush_interval_ms() -> i64 {
    1000
}

fn default_source_poll_interval_ms() -> i64 {
    2000
}

fn default_git_poll_interval_ms() -> i64 {
    20_000
}

fn default_pr_poll_interval_ms() -> i64 {
    90_000
}

fn default_agent_label_font_px() -> i64 {
    24
}

fn default_max_idle_agents() -> i64 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MonitorAlert {
    pub kind: String,
    pub message: String,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MonitorEventView {
    pub ts_ms: i64,
    #[serde(rename = "type")]
    pub event_type: String,
    pub state_hint: String,
    pub text: Option<String>,
    pub files_touched: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JumpTarget {
    pub source: String,
    pub session_id: String,
    pub repo_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionChoiceView {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingActionView {
    pub action_id: String,
    pub source: String,
    pub session_id: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub choices: Vec<ActionChoiceView>,
    pub expires_at: Option<i64>,
    pub jump_target: JumpTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IslandAgentView {
    pub key: String,
    pub source: String,
    pub session_id: String,
    pub agent_id: String,
    pub display_name: String,
    pub state: String,
    pub last_ts_ms: i64,
    pub last_text: Option<String>,
    pub last_activity_text: String,
    pub repo_path: Option<String>,
    pub files_touched: Vec<String>,
    pub alerts: Vec<MonitorAlert>,
    pub recent_events: Vec<MonitorEventView>,
    pub read_only: bool,
    pub can_reply: bool,
    pub jump_target: JumpTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IslandSummary {
    pub total: usize,
    pub active: usize,
    pub waiting: usize,
    pub done: usize,
    pub error: usize,
    pub pr_pending: usize,
    pub alerts: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IslandSnapshot {
    pub summary: IslandSummary,
    pub agents: Vec<IslandAgentView>,
    pub pending_actions: Vec<PendingActionView>,
    pub now_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MonitorNotification {
    pub title: String,
    pub message: String,
    pub kind: String,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TickPayload {
    pub snapshot: IslandSnapshot,
    pub notifications: Vec<MonitorNotification>,
}

#[derive(Debug, Clone, Default)]
pub struct MonitorMemory {
    pub previous_states: HashMap<String, String>,
    pub previous_action_ids: HashSet<String>,
}

#[derive(Debug, Clone)]
pub struct MonitorPaths {
    pub repo_bindings_file: PathBuf,
    pub codex_home: Option<PathBuf>,
    pub opencode_data_dir: Option<PathBuf>,
}

impl MonitorPaths {
    pub fn from_env(repo_bindings_file: PathBuf) -> Self {
        Self {
            repo_bindings_file,
            codex_home: std::env::var("CODEX_HOME").ok().map(PathBuf::from),
            opencode_data_dir: std::env::var("OPENCODE_DATA_DIR").ok().map(PathBuf::from),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TickOptions {
    pub actionable_notifications: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ActionBridgeSnapshot {
    pub sessions: Vec<InteractiveSession>,
    pub pending_actions: Vec<PendingActionView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InteractiveSession {
    pub source: String,
    pub session_id: String,
    pub display_name: Option<String>,
    pub repo_path: Option<String>,
    pub state: Option<String>,
    pub last_text: Option<String>,
    pub updated_at: i64,
    pub jump_target: Option<JumpTarget>,
}

#[cfg(unix)]
struct ClientState {
    writer: Arc<Mutex<UnixStream>>,
    session_keys: HashSet<String>,
    action_ids: HashSet<String>,
}

#[cfg(unix)]
#[derive(Default)]
struct BridgeState {
    next_client_id: u64,
    clients: HashMap<u64, ClientState>,
    sessions: HashMap<String, (InteractiveSession, u64)>,
    actions: HashMap<String, (PendingActionView, u64)>,
}

pub struct ActionBridge {
    #[cfg(unix)]
    socket_path: PathBuf,
    #[cfg(unix)]
    state: Arc<Mutex<BridgeState>>,
}

impl ActionBridge {
    #[cfg(unix)]
    pub fn start(socket_path: PathBuf) -> Result<Self, String> {
        if socket_path.exists() {
            fs::remove_file(&socket_path).map_err(|e| e.to_string())?;
        }
        ensure_parent(&socket_path)?;
        let listener = UnixListener::bind(&socket_path).map_err(|e| e.to_string())?;
        let state = Arc::new(Mutex::new(BridgeState::default()));
        let listener_state = Arc::clone(&state);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    continue;
                };
                let Ok(writer_stream) = stream.try_clone() else {
                    continue;
                };
                let client_state = Arc::clone(&listener_state);
                thread::spawn(move || {
                    handle_bridge_client(stream, writer_stream, client_state);
                });
            }
        });
        Ok(Self { socket_path, state })
    }

    #[cfg(not(unix))]
    pub fn start(_socket_path: PathBuf) -> Result<Self, String> {
        Ok(Self {})
    }

    #[cfg(unix)]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    #[cfg(not(unix))]
    pub fn socket_path(&self) -> &Path {
        Path::new("")
    }

    #[cfg(unix)]
    pub fn snapshot(&self) -> ActionBridgeSnapshot {
        let now = now_ms();
        let mut lock = match self.state.lock() {
            Ok(lock) => lock,
            Err(_) => return ActionBridgeSnapshot::default(),
        };
        let expired_ids: Vec<String> = lock
            .actions
            .iter()
            .filter_map(|(action_id, (action, _))| match action.expires_at {
                Some(expires_at) if expires_at <= now => Some(action_id.clone()),
                _ => None,
            })
            .collect();
        for action_id in expired_ids {
            remove_action_locked(&mut lock, &action_id);
        }
        let sessions = lock
            .sessions
            .values()
            .map(|(session, _)| session.clone())
            .collect();
        let pending_actions = lock
            .actions
            .values()
            .map(|(action, _)| action.clone())
            .collect();
        ActionBridgeSnapshot {
            sessions,
            pending_actions,
        }
    }

    #[cfg(not(unix))]
    pub fn snapshot(&self) -> ActionBridgeSnapshot {
        ActionBridgeSnapshot::default()
    }

    #[cfg(unix)]
    pub fn perform_action(&self, action_id: &str, choice_id: &str) -> Result<(), String> {
        let writer = {
            let mut lock = self.state.lock().map_err(|_| "bridge state lock failed".to_string())?;
            let (_, client_id) = lock
                .actions
                .get(action_id)
                .cloned()
                .ok_or_else(|| "unknown action".to_string())?;
            let writer = lock
                .clients
                .get(&client_id)
                .map(|client| Arc::clone(&client.writer))
                .ok_or_else(|| "bridge client is not connected".to_string())?;
            remove_action_locked(&mut lock, action_id);
            writer
        };

        let response = serde_json::json!({
            "type": "action_response",
            "action_id": action_id,
            "choice_id": choice_id,
        });
        let mut guard = writer.lock().map_err(|_| "bridge writer lock failed".to_string())?;
        let payload = serde_json::to_string(&response).map_err(|e| e.to_string())?;
        guard.write_all(payload.as_bytes()).map_err(|e| e.to_string())?;
        guard.write_all(b"\n").map_err(|e| e.to_string())
    }

    #[cfg(not(unix))]
    pub fn perform_action(&self, _action_id: &str, _choice_id: &str) -> Result<(), String> {
        Err("action bridge requires unix domain sockets".to_string())
    }
}

pub fn tick_monitor(
    settings: &MonitorSettings,
    memory: &mut MonitorMemory,
    bridge: &ActionBridge,
    paths: &MonitorPaths,
    options: TickOptions,
) -> Result<TickPayload, String> {
    let bridge_snapshot = bridge.snapshot();
    let mut map = HashMap::new();
    if settings.enabled {
        if settings.enable_opencode {
            scan_opencode(&mut map, paths);
        }
        if settings.enable_codex {
            scan_codex(&mut map, paths);
        }
    }

    let repo_bindings = read_repo_bindings(&paths.repo_bindings_file);
    for agent in map.values_mut() {
        if agent.repo_path.is_none() {
            if let Some(path) = repo_bindings.get(&agent.key) {
                agent.repo_path = Some(path.clone());
            }
        }
    }

    merge_bridge_sessions(&mut map, &bridge_snapshot.sessions);
    let mut snapshot = build_snapshot(map, bridge_snapshot.pending_actions, settings.enabled);
    if !settings.enabled {
        snapshot.pending_actions.clear();
    }
    let notifications = build_notifications(&snapshot, memory, options);
    Ok(TickPayload { snapshot, notifications })
}

#[derive(Debug, Clone)]
struct AgentTemp {
    key: String,
    source: String,
    session_id: String,
    agent_name: Option<String>,
    state: String,
    last_ts_ms: i64,
    last_text: Option<String>,
    repo_path: Option<String>,
    recent_events: Vec<MonitorEventView>,
    can_reply: bool,
    jump_target: Option<JumpTarget>,
}

fn merge_bridge_sessions(map: &mut HashMap<String, AgentTemp>, sessions: &[InteractiveSession]) {
    for session in sessions {
        let normalized_source = normalize_source_name(&session.source);
        let key = format!("{}:{}", normalized_source, session.session_id);
        let default_jump = JumpTarget {
            source: normalized_source.clone(),
            session_id: session.session_id.clone(),
            repo_path: session.repo_path.clone(),
        };
        match map.get_mut(&key) {
            Some(existing) => {
                existing.can_reply = true;
                if existing.repo_path.is_none() {
                    existing.repo_path = session.repo_path.clone();
                }
                if existing.agent_name.is_none() {
                    existing.agent_name = session.display_name.clone();
                }
                if session.updated_at >= existing.last_ts_ms {
                    if let Some(state) = &session.state {
                        existing.state = state.clone();
                    }
                    if session.last_text.is_some() {
                        existing.last_text = session.last_text.clone();
                    }
                    existing.last_ts_ms = session.updated_at;
                }
                existing.jump_target = session.jump_target.clone().or_else(|| Some(default_jump));
            }
            None => {
                map.insert(
                    key.clone(),
                    AgentTemp {
                        key,
                        source: normalized_source.clone(),
                        session_id: session.session_id.clone(),
                        agent_name: session.display_name.clone(),
                        state: session.state.clone().unwrap_or_else(|| "idle".to_string()),
                        last_ts_ms: session.updated_at,
                        last_text: session.last_text.clone(),
                        repo_path: session.repo_path.clone(),
                        recent_events: Vec::new(),
                        can_reply: true,
                        jump_target: session.jump_target.clone().or(Some(default_jump)),
                    },
                );
            }
        }
    }
}

fn build_snapshot(
    mut map: HashMap<String, AgentTemp>,
    mut pending_actions: Vec<PendingActionView>,
    enabled: bool,
) -> IslandSnapshot {
    let now = now_ms();
    pending_actions.sort_by(|a, b| a.action_id.cmp(&b.action_id));

    let action_keys: HashSet<String> = pending_actions
        .iter()
        .map(|action| format!("{}:{}", normalize_source_name(&action.source), action.session_id))
        .collect();
    for action in &pending_actions {
        let key = format!("{}:{}", normalize_source_name(&action.source), action.session_id);
        if let Some(agent) = map.get_mut(&key) {
            if agent.state != "error" && agent.state != "done" {
                agent.state = "waiting".to_string();
            }
            agent.can_reply = true;
            if agent.last_text.is_none() {
                agent.last_text = Some(action.title.clone());
            }
            if agent.jump_target.is_none() {
                agent.jump_target = Some(action.jump_target.clone());
            }
        }
    }

    let mut agents: Vec<IslandAgentView> = map
        .into_values()
        .map(|mut agent| {
            let silence = now - agent.last_ts_ms;
            if enabled
                && (agent.state == "running" || agent.state == "thinking" || agent.state == "waiting")
                && silence > IDLE_AFTER_MS
                && !action_keys.contains(&agent.key)
            {
                agent.state = "idle".to_string();
                if agent.last_text.as_deref() == Some("Thinking") {
                    agent.last_text = Some("Idle".to_string());
                }
            }
            if enabled && agent.state == "idle" && silence > DONE_AFTER_MS {
                agent.state = "done".to_string();
                if agent.last_text.is_none()
                    || agent.last_text.as_deref() == Some("Idle")
                    || agent.last_text.as_deref() == Some("Thinking")
                {
                    agent.last_text = Some("No recent activity".to_string());
                }
            }

            let alerts = if agent.state == "error" {
                vec![MonitorAlert {
                    kind: "error".to_string(),
                    message: agent
                        .last_text
                        .clone()
                        .unwrap_or_else(|| "Error detected".to_string()),
                    ts_ms: agent.last_ts_ms,
                }]
            } else {
                Vec::new()
            };
            let jump_target = agent.jump_target.clone().unwrap_or_else(|| JumpTarget {
                source: agent.source.clone(),
                session_id: agent.session_id.clone(),
                repo_path: agent.repo_path.clone(),
            });
            let display_name = format_agent_display_name(
                &agent.source,
                &agent.session_id,
                agent.agent_name.as_deref(),
                agent.repo_path.as_deref(),
            );
            let last_activity_text = agent
                .last_text
                .clone()
                .unwrap_or_else(|| "No recent activity".to_string());
            IslandAgentView {
                key: agent.key.clone(),
                source: agent.source.clone(),
                session_id: agent.session_id.clone(),
                agent_id: agent.session_id.clone(),
                display_name,
                state: agent.state.clone(),
                last_ts_ms: agent.last_ts_ms,
                last_text: agent.last_text.clone(),
                last_activity_text,
                repo_path: agent.repo_path.clone(),
                files_touched: Vec::new(),
                alerts,
                recent_events: agent.recent_events,
                read_only: !agent.can_reply,
                can_reply: agent.can_reply,
                jump_target,
            }
        })
        .collect();

    agents.sort_by(|a, b| b.last_ts_ms.cmp(&a.last_ts_ms));
    let summary = IslandSummary {
        total: agents.len(),
        active: agents
            .iter()
            .filter(|agent| agent.state == "running" || agent.state == "thinking")
            .count(),
        waiting: agents.iter().filter(|agent| agent.state == "waiting").count(),
        done: agents.iter().filter(|agent| agent.state == "done").count(),
        error: agents.iter().filter(|agent| agent.state == "error").count(),
        pr_pending: 0,
        alerts: agents.iter().map(|agent| agent.alerts.len()).sum(),
    };

    IslandSnapshot {
        summary,
        agents,
        pending_actions,
        now_ms: now,
    }
}

fn build_notifications(
    snapshot: &IslandSnapshot,
    memory: &mut MonitorMemory,
    options: TickOptions,
) -> Vec<MonitorNotification> {
    let mut notifications = Vec::new();
    let mut next_states = HashMap::new();
    for agent in &snapshot.agents {
        next_states.insert(agent.key.clone(), agent.state.clone());
        if (agent.state == "done" || agent.state == "error")
            && memory.previous_states.get(&agent.key) != Some(&agent.state)
        {
            notifications.push(MonitorNotification {
                title: if agent.state == "error" {
                    "Agent error".to_string()
                } else {
                    "Agent done".to_string()
                },
                message: format!("{} - {}", agent.display_name, agent.last_activity_text),
                kind: if agent.state == "error" {
                    "error".to_string()
                } else {
                    "done".to_string()
                },
                key: agent.key.clone(),
            });
        }
    }
    memory.previous_states = next_states;

    let next_action_ids: HashSet<String> = snapshot
        .pending_actions
        .iter()
        .map(|action| action.action_id.clone())
        .collect();
    if options.actionable_notifications {
        for action in &snapshot.pending_actions {
            if !memory.previous_action_ids.contains(&action.action_id) {
                notifications.push(MonitorNotification {
                    title: if action.kind == "approval" {
                        "Approval needed".to_string()
                    } else {
                        "Question waiting".to_string()
                    },
                    message: format!("{} - {}", action.title, action.body),
                    kind: "actionable".to_string(),
                    key: action.action_id.clone(),
                });
            }
        }
    }
    memory.previous_action_ids = next_action_ids;
    notifications
}

#[cfg(unix)]
fn handle_bridge_client(stream: UnixStream, writer_stream: UnixStream, state: Arc<Mutex<BridgeState>>) {
    let client_id = {
        let mut lock = match state.lock() {
            Ok(lock) => lock,
            Err(_) => return,
        };
        lock.next_client_id = lock.next_client_id.saturating_add(1);
        let client_id = lock.next_client_id;
        lock.clients.insert(
            client_id,
            ClientState {
                writer: Arc::new(Mutex::new(writer_stream)),
                session_keys: HashSet::new(),
                action_ids: HashSet::new(),
            },
        );
        client_id
    };

    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else {
            break;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<BridgeClientMessage>(trimmed) else {
            continue;
        };
        let mut lock = match state.lock() {
            Ok(lock) => lock,
            Err(_) => break,
        };
        apply_bridge_message(&mut lock, client_id, message);
    }

    let mut lock = match state.lock() {
        Ok(lock) => lock,
        Err(_) => return,
    };
    if let Some(client) = lock.clients.remove(&client_id) {
        for key in client.session_keys {
            lock.sessions.remove(&key);
        }
        for action_id in client.action_ids {
            lock.actions.remove(&action_id);
        }
    }
}

#[cfg(unix)]
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BridgeClientMessage {
    UpsertSession {
        source: String,
        session_id: String,
        display_name: Option<String>,
        repo_path: Option<String>,
        state: Option<String>,
        last_text: Option<String>,
        jump_target: Option<JumpTarget>,
    },
    CloseSession {
        source: String,
        session_id: String,
    },
    PublishAction {
        action_id: String,
        source: String,
        session_id: String,
        kind: String,
        title: String,
        body: String,
        choices: Vec<ActionChoiceView>,
        expires_at: Option<i64>,
        jump_target: Option<JumpTarget>,
    },
    ClearAction {
        action_id: String,
    },
}

#[cfg(unix)]
fn apply_bridge_message(state: &mut BridgeState, client_id: u64, message: BridgeClientMessage) {
    match message {
        BridgeClientMessage::UpsertSession {
            source,
            session_id,
            display_name,
            repo_path,
            state: session_state,
            last_text,
            jump_target,
        } => {
            let normalized_source = normalize_source_name(&source);
            let key = format!("{}:{}", normalized_source, session_id);
            let session = InteractiveSession {
                source: normalized_source,
                session_id: session_id.clone(),
                display_name,
                repo_path: repo_path.clone(),
                state: session_state,
                last_text,
                updated_at: now_ms(),
                jump_target: jump_target.or_else(|| {
                    Some(JumpTarget {
                        source: source.clone(),
                        session_id,
                        repo_path,
                    })
                }),
            };
            state.sessions.insert(key.clone(), (session, client_id));
            if let Some(client) = state.clients.get_mut(&client_id) {
                client.session_keys.insert(key);
            }
        }
        BridgeClientMessage::CloseSession { source, session_id } => {
            let key = format!("{}:{}", normalize_source_name(&source), session_id);
            state.sessions.remove(&key);
            if let Some(client) = state.clients.get_mut(&client_id) {
                client.session_keys.remove(&key);
            }
        }
        BridgeClientMessage::PublishAction {
            action_id,
            source,
            session_id,
            kind,
            title,
            body,
            choices,
            expires_at,
            jump_target,
        } => {
            let default_jump = JumpTarget {
                source: normalize_source_name(&source),
                session_id: session_id.clone(),
                repo_path: None,
            };
            let action = PendingActionView {
                action_id: action_id.clone(),
                source,
                session_id,
                kind,
                title,
                body,
                choices,
                expires_at,
                jump_target: jump_target.unwrap_or(default_jump),
            };
            state.actions.insert(action_id.clone(), (action, client_id));
            if let Some(client) = state.clients.get_mut(&client_id) {
                client.action_ids.insert(action_id);
            }
        }
        BridgeClientMessage::ClearAction { action_id } => {
            remove_action_locked(state, &action_id);
        }
    }
}

#[cfg(unix)]
fn remove_action_locked(state: &mut BridgeState, action_id: &str) {
    let removed = state.actions.remove(action_id);
    if let Some((_, client_id)) = removed {
        if let Some(client) = state.clients.get_mut(&client_id) {
            client.action_ids.remove(action_id);
        }
    }
}

fn scan_opencode(map: &mut HashMap<String, AgentTemp>, paths: &MonitorPaths) {
    if scan_opencode_db(map, paths) {
        return;
    }

    let root = opencode_message_root(paths);
    if !root.exists() {
        return;
    }

    let session_repo = load_opencode_session_repo_map(paths);
    let session_name = load_opencode_session_name_map(paths);
    let files = collect_files(&root, "json", MAX_OPENCODE_FILES);
    for file in files {
        let raw = match fs::read_to_string(&file) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let session_id = string_at(&value, &["sessionID", "sessionId"])
            .or_else(|| {
                file.parent()
                    .and_then(|parent| parent.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "unknown".to_string());
        let ts = normalize_epoch_ms(
            number_at(&value, &["time", "created"]).unwrap_or_else(|| modified_ms(&file)),
        );
        let completed = number_at(&value, &["time", "completed"]).is_some();
        let state = if completed { "done" } else { "running" }.to_string();
        let text = truncate_option_text(
            string_at(&value, &["summary"]).or_else(|| string_at(&value, &["finish"])),
        );
        let repo_path = string_at(&value, &["path", "root"])
            .or_else(|| string_at(&value, &["path", "cwd"]))
            .or_else(|| session_repo.get(&session_id).cloned());
        upsert_agent(
            map,
            AgentTemp {
                key: format!("opencode:{}", session_id),
                source: "opencode".to_string(),
                session_id: session_id.clone(),
                agent_name: session_name.get(&session_id).cloned(),
                state: state.clone(),
                last_ts_ms: ts,
                last_text: text.clone(),
                repo_path,
                recent_events: vec![MonitorEventView {
                    ts_ms: ts,
                    event_type: "message".to_string(),
                    state_hint: state,
                    text,
                    files_touched: Vec::new(),
                }],
                can_reply: false,
                jump_target: None,
            },
        );
    }

    let part_root = opencode_part_root(paths);
    if !part_root.exists() {
        return;
    }

    let part_files = collect_files(&part_root, "json", MAX_OPENCODE_PART_FILES);
    for file in part_files {
        let raw = match fs::read_to_string(&file) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(_) => continue,
        };

        let Some(session_id) = string_at(&value, &["sessionID", "sessionId"]) else {
            continue;
        };

        let modified = normalize_epoch_ms(modified_ms(&file));
        let part_type = string_at(&value, &["type"]).unwrap_or_default();
        let (state, event_type, text, ts) = if part_type == "tool" {
            classify_opencode_tool(&value, modified)
        } else if part_type == "reasoning" {
            let start_ts = number_at(&value, &["time", "start"])
                .map(normalize_epoch_ms)
                .unwrap_or(modified);
            let end_ts = number_at(&value, &["time", "end"]).map(normalize_epoch_ms);
            (
                "thinking".to_string(),
                "status".to_string(),
                string_at(&value, &["text"]).or_else(|| Some("Thinking".to_string())),
                end_ts.unwrap_or(start_ts),
            )
        } else if part_type == "step-start" {
            (
                "running".to_string(),
                "status".to_string(),
                Some("Step started".to_string()),
                modified,
            )
        } else if part_type == "step-finish" {
            let reason = string_at(&value, &["reason"]).unwrap_or_else(|| "stop".to_string());
            (
                "done".to_string(),
                "status".to_string(),
                Some(format!("Step finished: {}", reason)),
                modified,
            )
        } else {
            continue;
        };

        let text = truncate_option_text(text);
        upsert_agent(
            map,
            AgentTemp {
                key: format!("opencode:{}", session_id),
                source: "opencode".to_string(),
                session_id: session_id.clone(),
                agent_name: load_opencode_session_name_map(paths).get(&session_id).cloned(),
                state: state.clone(),
                last_ts_ms: ts,
                last_text: text.clone(),
                repo_path: load_opencode_session_repo_map(paths).get(&session_id).cloned(),
                recent_events: vec![MonitorEventView {
                    ts_ms: ts,
                    event_type,
                    state_hint: state,
                    text,
                    files_touched: Vec::new(),
                }],
                can_reply: false,
                jump_target: None,
            },
        );
    }
}

fn scan_opencode_db(map: &mut HashMap<String, AgentTemp>, paths: &MonitorPaths) -> bool {
    let db_path = opencode_db_file(paths);
    if !db_path.exists() {
        return false;
    }
    let conn = match Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(conn) => conn,
        Err(_) => return false,
    };

    let mut session_repo = HashMap::new();
    let mut session_name = HashMap::new();
    {
        let mut stmt = match conn.prepare(
            "SELECT id, directory, title, time_updated
             FROM session
             WHERE time_archived IS NULL OR time_archived = 0
             ORDER BY time_updated DESC
             LIMIT ?1",
        ) {
            Ok(stmt) => stmt,
            Err(_) => return false,
        };
        let rows = stmt.query_map([MAX_OPENCODE_DB_SESSIONS as i64], |row| {
            let id: String = row.get(0)?;
            let directory: String = row.get(1)?;
            let title: Option<String> = row.get(2)?;
            let time_updated: i64 = row.get(3)?;
            Ok((id, directory, title, time_updated))
        });
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (session_id, directory, title, time_updated) = row;
                let ts = normalize_epoch_ms(time_updated);
                session_repo.insert(session_id.clone(), directory.clone());
                if let Some(name) = title.clone() {
                    session_name.insert(session_id.clone(), name);
                }
                upsert_agent(
                    map,
                    AgentTemp {
                        key: format!("opencode:{}", session_id),
                        source: "opencode".to_string(),
                        session_id,
                        agent_name: title.clone(),
                        state: "running".to_string(),
                        last_ts_ms: ts,
                        last_text: Some("Session activity".to_string()),
                        repo_path: Some(directory),
                        recent_events: vec![MonitorEventView {
                            ts_ms: ts,
                            event_type: "status".to_string(),
                            state_hint: "running".to_string(),
                            text: Some("Session activity".to_string()),
                            files_touched: Vec::new(),
                        }],
                        can_reply: false,
                        jump_target: None,
                    },
                );
            }
        }
    }
    {
        let mut stmt = match conn.prepare(
            "SELECT session_id, time_updated, data
             FROM part
             ORDER BY time_updated DESC
             LIMIT ?1",
        ) {
            Ok(stmt) => stmt,
            Err(_) => return false,
        };
        let rows = stmt.query_map([MAX_OPENCODE_DB_PARTS as i64], |row| {
            let session_id: String = row.get(0)?;
            let time_updated: i64 = row.get(1)?;
            let data: String = row.get(2)?;
            Ok((session_id, time_updated, data))
        });
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (session_id, time_updated, data) = row;
                let value: Value = match serde_json::from_str(&data) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let part_type = string_at(&value, &["type"]).unwrap_or_default();
                let fallback_ts = normalize_epoch_ms(time_updated);
                let (state, event_type, text, ts) = if part_type == "tool" {
                    classify_opencode_tool(&value, fallback_ts)
                } else if part_type == "reasoning" {
                    let start_ts = number_at(&value, &["time", "start"])
                        .map(normalize_epoch_ms)
                        .unwrap_or(fallback_ts);
                    let end_ts = number_at(&value, &["time", "end"]).map(normalize_epoch_ms);
                    (
                        "thinking".to_string(),
                        "status".to_string(),
                        string_at(&value, &["text"]).or_else(|| Some("Thinking".to_string())),
                        end_ts.unwrap_or(start_ts),
                    )
                } else if part_type == "step-start" {
                    (
                        "running".to_string(),
                        "status".to_string(),
                        Some("Step started".to_string()),
                        fallback_ts,
                    )
                } else if part_type == "step-finish" {
                    let reason =
                        string_at(&value, &["reason"]).unwrap_or_else(|| "stop".to_string());
                    (
                        "done".to_string(),
                        "status".to_string(),
                        Some(format!("Step finished: {}", reason)),
                        fallback_ts,
                    )
                } else {
                    continue;
                };

                let text = truncate_option_text(text);
                upsert_agent(
                    map,
                    AgentTemp {
                        key: format!("opencode:{}", session_id),
                        source: "opencode".to_string(),
                        session_id: session_id.clone(),
                        agent_name: session_name.get(&session_id).cloned(),
                        state: state.clone(),
                        last_ts_ms: ts,
                        last_text: text.clone(),
                        repo_path: session_repo.get(&session_id).cloned(),
                        recent_events: vec![MonitorEventView {
                            ts_ms: ts,
                            event_type,
                            state_hint: state,
                            text,
                            files_touched: Vec::new(),
                        }],
                        can_reply: false,
                        jump_target: None,
                    },
                );
            }
        }
    }
    true
}

fn classify_opencode_tool(value: &Value, fallback_ts: i64) -> (String, String, Option<String>, i64) {
    let state_obj = value
        .get("state")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(Map::new);
    let status = state_obj
        .get("status")
        .and_then(Value::as_str)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "running".to_string());
    let normalized_status = status.to_lowercase();
    let tool_name = string_at(value, &["tool"]).unwrap_or_else(|| "tool".to_string());
    let start_ts = state_obj
        .get("time")
        .and_then(Value::as_object)
        .and_then(|value| value.get("start"))
        .and_then(to_i64)
        .map(normalize_epoch_ms)
        .unwrap_or(fallback_ts);
    let end_ts = state_obj
        .get("time")
        .and_then(Value::as_object)
        .and_then(|value| value.get("end"))
        .and_then(to_i64)
        .map(normalize_epoch_ms);
    let hint = if normalized_status == "error" {
        "error".to_string()
    } else if normalized_status == "completed" || end_ts.is_some() {
        "done".to_string()
    } else {
        "running".to_string()
    };
    (
        hint.clone(),
        if normalized_status == "error" {
            "error".to_string()
        } else {
            "tool".to_string()
        },
        Some(format!("{}: {}", tool_name, normalized_status)),
        end_ts.unwrap_or(start_ts),
    )
}

fn scan_codex(map: &mut HashMap<String, AgentTemp>, paths: &MonitorPaths) {
    let root = codex_sessions_root(paths);
    if !root.exists() {
        return;
    }
    let session_name_map = load_codex_session_name_map(paths);
    let files = collect_files(&root, "jsonl", MAX_CODEX_FILES);
    for file in files {
        let modified = modified_ms(&file);
        let fallback_session = parse_session_from_filename(&file).unwrap_or_else(|| {
            file.file_stem()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_else(|| "unknown".to_string())
        });
        let tail = match read_tail(&file, CODEX_TAIL_BYTES) {
            Ok(value) => value,
            Err(_) => continue,
        };
        for line in tail.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let record: Value = match serde_json::from_str(trimmed) {
                Ok(value) => value,
                Err(_) => continue,
            };
            let payload = record
                .get("payload")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_else(Map::new);
            let kind = record
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let payload_type = payload
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let session_id = payload
                .get("id")
                .and_then(Value::as_str)
                .map(|value| value.to_string())
                .or_else(|| {
                    record
                        .get("session_id")
                        .and_then(Value::as_str)
                        .map(|value| value.to_string())
                })
                .or_else(|| {
                    record
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .map(|value| value.to_string())
                })
                .unwrap_or_else(|| fallback_session.clone());
            let ts = number_direct(&record, "ts")
                .or_else(|| number_direct(&record, "timestamp"))
                .or_else(|| payload.get("ts").and_then(to_i64))
                .or_else(|| payload.get("timestamp").and_then(to_i64))
                .map(normalize_epoch_ms)
                .unwrap_or(modified);
            let repo_path = payload
                .get("cwd")
                .and_then(Value::as_str)
                .map(|value| value.to_string())
                .or_else(|| {
                    record
                        .get("cwd")
                        .and_then(Value::as_str)
                        .map(|value| value.to_string())
                });
            let (state, event_type, text) = classify_codex_event(kind, payload_type, &record, &payload);
            let agent_name = extract_codex_agent_name(kind, payload_type, &record, &payload);

            let event = MonitorEventView {
                ts_ms: ts,
                event_type,
                state_hint: state.clone(),
                text: Some(text.clone()),
                files_touched: Vec::new(),
            };

            let entry = map.entry(format!("codex:{}", session_id)).or_insert(AgentTemp {
                key: format!("codex:{}", session_id),
                source: "codex".to_string(),
                session_id: session_id.clone(),
                agent_name: session_name_map.get(&session_id).cloned(),
                state: "idle".to_string(),
                last_ts_ms: ts,
                last_text: Some("Session discovered".to_string()),
                repo_path: repo_path.clone(),
                recent_events: Vec::new(),
                can_reply: false,
                jump_target: None,
            });

            if entry.repo_path.is_none() && repo_path.is_some() {
                entry.repo_path = repo_path;
            }
            if entry.agent_name.is_none() && agent_name.is_some() {
                entry.agent_name = agent_name.clone();
            }
            entry.recent_events.insert(0, event);
            if entry.recent_events.len() > MAX_RECENT_EVENTS {
                entry.recent_events.truncate(MAX_RECENT_EVENTS);
            }
            if ts >= entry.last_ts_ms {
                entry.last_ts_ms = ts;
                entry.state = state;
                entry.last_text = Some(text);
                if agent_name.is_some() {
                    entry.agent_name = agent_name;
                }
            }
        }
    }
}

fn load_codex_session_name_map(paths: &MonitorPaths) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let index_path = paths
        .codex_home
        .clone()
        .unwrap_or_else(|| {
            home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".codex")
        })
        .join("session_index.jsonl");
    let raw = match fs::read_to_string(index_path) {
        Ok(value) => value,
        Err(_) => return out,
    };
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let session_id = string_at(&value, &["id"]);
        let title = string_at(&value, &["thread_name", "threadName", "title"]);
        if let (Some(session_id), Some(title)) = (session_id, title.and_then(|t| normalize_agent_name(&t))) {
            out.insert(session_id, title);
        }
    }
    out
}

fn extract_codex_agent_name(
    kind: &str,
    payload_type: &str,
    record: &Value,
    payload: &Map<String, Value>,
) -> Option<String> {
    if kind == "event_msg" && payload_type == "user_message" {
        let message = payload
            .get("message")
            .and_then(Value::as_str)
            .map(|value| value.to_string())
            .or_else(|| {
                record
                    .get("message")
                    .and_then(Value::as_str)
                    .map(|value| value.to_string())
            });
        if let Some(message) = message {
            let first_line = message
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .to_string();
            if !first_line.is_empty() {
                return Some(first_line);
            }
        }
    }
    None
}

fn classify_codex_event(
    kind: &str,
    payload_type: &str,
    record: &Value,
    payload: &Map<String, Value>,
) -> (String, String, String) {
    let lower = format!("{} {}", kind.to_lowercase(), payload_type.to_lowercase());
    if lower.contains("task_complete")
        || lower.contains("turn_completed")
        || lower.contains("turn.complete")
        || lower.contains("task.complete")
        || lower.contains("item.completed")
        || lower.contains("completed")
    {
        return (
            "done".to_string(),
            "status".to_string(),
            "Turn completed".to_string(),
        );
    }
    if lower.contains("turn_aborted") || lower.contains("task_aborted") || lower.contains("aborted") {
        return (
            "waiting".to_string(),
            "status".to_string(),
            "Turn aborted".to_string(),
        );
    }
    if lower.contains("error")
        || lower.contains("failed")
        || lower.contains("exception")
        || lower.contains("fatal")
    {
        return (
            "error".to_string(),
            "error".to_string(),
            "Codex error".to_string(),
        );
    }
    if payload_type == "agent_message" || payload_type == "message" {
        let message = record
            .get("message")
            .and_then(Value::as_str)
            .map(|value| value.to_string())
            .or_else(|| {
                payload
                    .get("message")
                    .and_then(Value::as_str)
                    .map(|value| value.to_string())
            })
            .unwrap_or_else(|| "Assistant message".to_string());
        return ("running".to_string(), "message".to_string(), message);
    }
    if payload_type == "agent_reasoning" || payload_type == "reasoning" || payload_type == "token_count" {
        return (
            "thinking".to_string(),
            "status".to_string(),
            "Thinking".to_string(),
        );
    }
    if payload_type == "task_started" {
        return (
            "running".to_string(),
            "status".to_string(),
            "Task started".to_string(),
        );
    }
    if payload_type == "user_message" {
        return (
            "waiting".to_string(),
            "message".to_string(),
            "Waiting for input".to_string(),
        );
    }
    if payload_type == "function_call" || payload_type == "custom_tool_call" {
        let tool_name = payload
            .get("name")
            .and_then(Value::as_str)
            .map(|value| value.to_string())
            .or_else(|| {
                payload
                    .get("function")
                    .and_then(Value::as_object)
                    .and_then(|value| value.get("name"))
                    .and_then(Value::as_str)
                    .map(|value| value.to_string())
            })
            .unwrap_or_else(|| "tool".to_string());
        return (
            "running".to_string(),
            "tool".to_string(),
            format!("{}: running", tool_name),
        );
    }
    if payload_type == "function_call_output" || payload_type == "custom_tool_call_output" {
        return (
            "running".to_string(),
            "tool".to_string(),
            "Tool output".to_string(),
        );
    }
    let fallback = record
        .get("message")
        .and_then(Value::as_str)
        .map(|value| value.to_string())
        .or_else(|| {
            payload
                .get("message")
                .and_then(Value::as_str)
                .map(|value| value.to_string())
        })
        .unwrap_or_else(|| kind.to_string());
    ("running".to_string(), "message".to_string(), fallback)
}

fn upsert_agent(map: &mut HashMap<String, AgentTemp>, incoming: AgentTemp) {
    match map.get_mut(&incoming.key) {
        Some(existing) => {
            if incoming.last_ts_ms >= existing.last_ts_ms {
                let mut merged = incoming;
                if merged.repo_path.is_none() {
                    merged.repo_path = existing.repo_path.clone();
                }
                if merged.last_text.is_none() {
                    merged.last_text = existing.last_text.clone();
                }
                if merged.agent_name.is_none() {
                    merged.agent_name = existing.agent_name.clone();
                }
                if !merged.can_reply {
                    merged.can_reply = existing.can_reply;
                }
                if merged.jump_target.is_none() {
                    merged.jump_target = existing.jump_target.clone();
                }
                *existing = merged;
            }
        }
        None => {
            map.insert(incoming.key.clone(), incoming);
        }
    }
}

fn read_repo_bindings(path: &Path) -> HashMap<String, String> {
    match read_json_file(path) {
        Ok(value) => serde_json::from_value(value).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

fn collect_files(root: &Path, ext: &str, max_files: usize) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = WalkDir::new(root)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            let path = entry.path().to_path_buf();
            let matches = path
                .extension()
                .and_then(|value| value.to_str())
                .map(|value| value.eq_ignore_ascii_case(ext))
                .unwrap_or(false);
            if matches {
                Some(path)
            } else {
                None
            }
        })
        .collect();

    files.sort_by(|left, right| modified_ms(right).cmp(&modified_ms(left)));
    if files.len() > max_files {
        files.truncate(max_files);
    }
    files
}

fn truncate_option_text(text: Option<String>) -> Option<String> {
    text.map(truncate_text)
}

fn truncate_text(text: String) -> String {
    if text.chars().count() <= MAX_MONITOR_TEXT_CHARS {
        return text;
    }
    let mut out = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index >= MAX_MONITOR_TEXT_CHARS {
            break;
        }
        out.push(ch);
    }
    out.push_str("...");
    out
}

fn modified_ms(path: &Path) -> i64 {
    fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(system_time_to_ms)
        .unwrap_or_else(now_ms)
}

fn normalize_epoch_ms(value: i64) -> i64 {
    if value <= 0 {
        return value;
    }
    if value < 10_000_000_000 {
        return value.saturating_mul(1000);
    }
    if value > 10_000_000_000_000_000 {
        return value / 1_000_000;
    }
    if value > 10_000_000_000_000 {
        return value / 1000;
    }
    value
}

fn system_time_to_ms(time: SystemTime) -> Option<i64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as i64)
}

fn read_tail(path: &Path, max_bytes: usize) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len() as usize;
    let bytes = size.min(max_bytes);
    if bytes == 0 {
        return Ok(String::new());
    }
    file.seek(SeekFrom::Start((size - bytes) as u64))
        .map_err(|e| e.to_string())?;
    let mut buffer = vec![0_u8; bytes];
    file.read_exact(&mut buffer).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

fn parse_session_from_filename(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy().to_string();
    let regex = Regex::new(
        r"([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})$",
    )
    .ok()?;
    regex
        .captures(&stem)
        .and_then(|captures| captures.get(1).map(|value| value.as_str().to_string()))
}

fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().map(|value| value.to_string())
}

fn number_at(value: &Value, path: &[&str]) -> Option<i64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    to_i64(current)
}

fn number_direct(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(to_i64)
}

fn to_i64(value: &Value) -> Option<i64> {
    if let Some(value) = value.as_i64() {
        return Some(value);
    }
    if let Some(value) = value.as_u64() {
        return Some(value as i64);
    }
    if let Some(value) = value.as_f64() {
        return Some(value as i64);
    }
    None
}

fn short_session(session: &str) -> String {
    session.chars().take(8).collect()
}

fn normalize_agent_name(name: &str) -> Option<String> {
    let compact = name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    if compact.is_empty() {
        return None;
    }
    let mut out = String::new();
    for (index, ch) in compact.chars().enumerate() {
        if index >= 56 {
            break;
        }
        out.push(ch);
    }
    if compact.chars().count() > 56 {
        out.push_str("...");
    }
    Some(out)
}

fn repo_label(repo_path: &str) -> Option<String> {
    let trimmed = repo_path.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = Path::new(trimmed);
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(trimmed)
        .trim();
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

fn format_agent_display_name(
    source: &str,
    session_id: &str,
    agent_name: Option<&str>,
    repo_path: Option<&str>,
) -> String {
    let normalized_source = normalize_source_name(source);
    if let Some(name) = agent_name.and_then(normalize_agent_name) {
        return format!("{}: {}", normalized_source, name);
    }
    if let Some(repo) = repo_path
        .and_then(repo_label)
        .and_then(|name| normalize_agent_name(&name))
    {
        return format!("{}: {}", normalized_source, repo);
    }
    format!("{}: {}", normalized_source, short_session(session_id))
}

fn now_ms() -> i64 {
    system_time_to_ms(SystemTime::now()).unwrap_or(0)
}

pub fn claude_available() -> bool {
    command_available("claude")
}

pub fn command_available(command: &str) -> bool {
    if cfg!(target_os = "windows") {
        std::process::Command::new("where")
            .arg(command)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    } else {
        std::process::Command::new("sh")
            .arg("-lc")
            .arg(format!("command -v {}", command))
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
}

pub fn default_pixel_agents_dir() -> PathBuf {
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".pixel-agents")
}

fn ensure_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn read_json_file(path: &Path) -> Result<Value, String> {
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&raw).map_err(|e| e.to_string())
}

pub fn write_json_file(path: &Path, value: &Value) -> Result<(), String> {
    ensure_parent(path)?;
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| e.to_string())
}

fn opencode_message_root(paths: &MonitorPaths) -> PathBuf {
    opencode_storage_root(paths).join("message")
}

fn opencode_part_root(paths: &MonitorPaths) -> PathBuf {
    opencode_storage_root(paths).join("part")
}

fn opencode_session_root(paths: &MonitorPaths) -> PathBuf {
    opencode_storage_root(paths).join("session")
}

fn opencode_project_root(paths: &MonitorPaths) -> PathBuf {
    opencode_storage_root(paths).join("project")
}

fn opencode_storage_root(paths: &MonitorPaths) -> PathBuf {
    opencode_data_root(paths).join("storage")
}

fn opencode_db_file(paths: &MonitorPaths) -> PathBuf {
    opencode_data_root(paths).join("opencode.db")
}

fn opencode_data_root(paths: &MonitorPaths) -> PathBuf {
    let configured = paths.opencode_data_dir.clone().unwrap_or_else(|| {
        home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local")
            .join("share")
            .join("opencode")
    });
    if configured
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case("storage"))
        .unwrap_or(false)
    {
        return configured.parent().map(Path::to_path_buf).unwrap_or(configured);
    }
    configured
}

fn codex_sessions_root(paths: &MonitorPaths) -> PathBuf {
    paths
        .codex_home
        .clone()
        .unwrap_or_else(|| {
            home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".codex")
        })
        .join("sessions")
}

fn load_opencode_session_repo_map(paths: &MonitorPaths) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let session_root = opencode_session_root(paths);
    if !session_root.exists() {
        return out;
    }
    let project_root = opencode_project_root(paths);
    let session_files = collect_files(&session_root, "json", MAX_OPENCODE_FILES);
    for file in session_files {
        let raw = match fs::read_to_string(&file) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let session_id = string_at(&value, &["id"])
            .or_else(|| {
                file.parent()
                    .and_then(|parent| parent.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .or_else(|| file.file_stem().map(|name| name.to_string_lossy().into_owned()));
        let project_id = string_at(&value, &["projectID", "projectId"]);
        let Some(session_id) = session_id else {
            continue;
        };
        let Some(project_id) = project_id else {
            continue;
        };
        let project_file = project_root.join(format!("{}.json", project_id));
        let project_raw = match fs::read_to_string(project_file) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let project: Value = match serde_json::from_str(&project_raw) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if let Some(repo) = string_at(&project, &["worktree"]) {
            out.insert(session_id, repo);
        }
    }
    out
}

fn load_opencode_session_name_map(paths: &MonitorPaths) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let session_root = opencode_session_root(paths);
    if !session_root.exists() {
        return out;
    }
    let session_files = collect_files(&session_root, "json", MAX_OPENCODE_FILES);
    for file in session_files {
        let raw = match fs::read_to_string(&file) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let session_id = string_at(&value, &["id"])
            .or_else(|| {
                file.parent()
                    .and_then(|parent| parent.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .or_else(|| file.file_stem().map(|name| name.to_string_lossy().into_owned()));
        let title = string_at(&value, &["title"]);
        if let (Some(session_id), Some(title)) = (session_id, title) {
            out.insert(session_id, title);
        }
    }
    out
}

fn normalize_source_name(source: &str) -> String {
    let normalized = source.trim().to_lowercase();
    if normalized == "claude"
        || normalized == "claude code"
        || normalized == "claude-code"
        || normalized == "claudecode"
    {
        return "claude".to_string();
    }
    if normalized == "opencode"
        || normalized == "open"
        || normalized == "open-code"
        || normalized == "open_code"
    {
        return "opencode".to_string();
    }
    if normalized == "codex" {
        return "codex".to_string();
    }
    source.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn running_codex_record(session_id: &str, message: &str, ts: i64) -> String {
        serde_json::json!({
            "type": "event_msg",
            "payload": {
                "type": "agent_message",
                "id": session_id,
                "cwd": "/tmp/demo",
                "message": message
            },
            "ts": ts
        })
        .to_string()
    }

    fn completed_codex_record(session_id: &str, ts: i64) -> String {
        serde_json::json!({
            "type": "turn_completed",
            "payload": {
                "type": "task_complete",
                "id": session_id,
                "cwd": "/tmp/demo"
            },
            "ts": ts
        })
        .to_string()
    }

    fn write_codex_session(paths: &MonitorPaths, session_id: &str, lines: &[String]) {
        let root = codex_sessions_root(paths).join("project");
        fs::create_dir_all(&root).unwrap();
        let file = root.join(format!("session-{}.jsonl", session_id));
        fs::write(file, lines.join("\n")).unwrap();
    }

    #[test]
    fn tick_detects_codex_agent() {
        let tmp = tempdir().unwrap();
        let ts = now_ms();
        let paths = MonitorPaths {
            repo_bindings_file: tmp.path().join("bindings.json"),
            codex_home: Some(tmp.path().join("codex-home")),
            opencode_data_dir: Some(tmp.path().join("opencode")),
        };
        write_codex_session(
            &paths,
            "11111111-1111-1111-1111-111111111111",
            &[running_codex_record(
                "11111111-1111-1111-1111-111111111111",
                "Checking auth middleware",
                ts,
            )],
        );
        let bridge = ActionBridge::start(tmp.path().join("bridge.sock")).unwrap();
        let payload = tick_monitor(
            &MonitorSettings::default(),
            &mut MonitorMemory::default(),
            &bridge,
            &paths,
            TickOptions::default(),
        )
        .unwrap();
        assert_eq!(payload.snapshot.agents.len(), 1);
        assert_eq!(payload.snapshot.agents[0].source, "codex");
        assert_eq!(payload.snapshot.agents[0].state, "running");
    }

    #[test]
    fn done_notification_only_emits_once() {
        let tmp = tempdir().unwrap();
        let ts = now_ms();
        let paths = MonitorPaths {
            repo_bindings_file: tmp.path().join("bindings.json"),
            codex_home: Some(tmp.path().join("codex-home")),
            opencode_data_dir: Some(tmp.path().join("opencode")),
        };
        let session_id = "22222222-2222-2222-2222-222222222222";
        write_codex_session(&paths, session_id, &[running_codex_record(session_id, "Working", ts)]);
        let bridge = ActionBridge::start(tmp.path().join("bridge.sock")).unwrap();
        let mut memory = MonitorMemory::default();
        let initial = tick_monitor(
            &MonitorSettings::default(),
            &mut memory,
            &bridge,
            &paths,
            TickOptions::default(),
        )
        .unwrap();
        assert!(initial.notifications.is_empty());

        write_codex_session(
            &paths,
            session_id,
            &[
                running_codex_record(session_id, "Working", ts),
                completed_codex_record(session_id, ts + 2000),
            ],
        );
        let second = tick_monitor(
            &MonitorSettings::default(),
            &mut memory,
            &bridge,
            &paths,
            TickOptions::default(),
        )
        .unwrap();
        assert_eq!(second.notifications.len(), 1);
        assert_eq!(second.notifications[0].kind, "done");

        let third = tick_monitor(
            &MonitorSettings::default(),
            &mut memory,
            &bridge,
            &paths,
            TickOptions::default(),
        )
        .unwrap();
        assert!(third.notifications.is_empty());
    }

    #[test]
    fn bridge_sessions_make_agent_interactive() {
        let map = HashMap::new();
        let sessions = vec![InteractiveSession {
            source: "codex".to_string(),
            session_id: "33333333-3333-3333-3333-333333333333".to_string(),
            display_name: Some("fix auth bug".to_string()),
            repo_path: Some("/tmp/repo".to_string()),
            state: Some("running".to_string()),
            last_text: Some("Reading middleware.ts".to_string()),
            updated_at: now_ms(),
            jump_target: None,
        }];
        let snapshot = build_snapshot({
            let mut next = map;
            merge_bridge_sessions(&mut next, &sessions);
            next
        }, Vec::new(), true);
        assert_eq!(snapshot.agents.len(), 1);
        assert!(snapshot.agents[0].can_reply);
        assert!(!snapshot.agents[0].read_only);
    }

    #[cfg(unix)]
    #[test]
    fn bridge_action_lifecycle_round_trips_response() {
        let tmp = tempdir().unwrap();
        let bridge = ActionBridge::start(tmp.path().join("bridge.sock")).unwrap();
        let mut stream = UnixStream::connect(bridge.socket_path()).unwrap();
        let reader = stream.try_clone().unwrap();
        let mut reader = BufReader::new(reader);

        let session_line = serde_json::json!({
            "type": "upsert_session",
            "source": "codex",
            "session_id": "44444444-4444-4444-4444-444444444444",
            "state": "waiting",
            "last_text": "Waiting for approval"
        })
        .to_string();
        let action_line = serde_json::json!({
            "type": "publish_action",
            "action_id": "action-1",
            "source": "codex",
            "session_id": "44444444-4444-4444-4444-444444444444",
            "kind": "approval",
            "title": "Allow command",
            "body": "Run migration?",
            "choices": [{"id":"allow","label":"Allow"},{"id":"deny","label":"Deny"}]
        })
        .to_string();
        stream.write_all(session_line.as_bytes()).unwrap();
        stream.write_all(b"\n").unwrap();
        stream.write_all(action_line.as_bytes()).unwrap();
        stream.write_all(b"\n").unwrap();
        thread::sleep(std::time::Duration::from_millis(50));

        let snapshot = bridge.snapshot();
        assert_eq!(snapshot.sessions.len(), 1);
        assert_eq!(snapshot.pending_actions.len(), 1);

        bridge.perform_action("action-1", "allow").unwrap();
        let mut response = String::new();
        reader.read_line(&mut response).unwrap();
        assert!(response.contains("\"action_response\""));
        assert!(bridge.snapshot().pending_actions.is_empty());
    }
}
