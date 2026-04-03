use agent_monitor_core::{
    claude_available, command_available, read_json_file, tick_monitor, write_json_file,
    ActionBridge, MonitorMemory, MonitorPaths, MonitorSettings, TickOptions,
};
use dirs::home_dir;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use tauri::{LogicalPosition, LogicalSize, Manager, Position, Size, State, WebviewWindow};

const COLLAPSED_WIDTH: f64 = 580.0;
const COLLAPSED_HEIGHT: f64 = 164.0;
const EXPANDED_WIDTH: f64 = 1180.0;
const EXPANDED_HEIGHT: f64 = 780.0;
const TOP_MARGIN: f64 = 14.0;

struct IslandState {
    monitor_memory: Mutex<MonitorMemory>,
    action_bridge: ActionBridge,
    window_mode: Mutex<String>,
}

impl IslandState {
    fn new() -> Result<Self, String> {
        Ok(Self {
            monitor_memory: Mutex::new(MonitorMemory::default()),
            action_bridge: ActionBridge::start(agent_island_dir().join("bridge.sock"))?,
            window_mode: Mutex::new(read_window_mode()),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
struct BootstrapPayload {
    #[serde(rename = "socketPath")]
    socket_path: String,
    #[serde(rename = "claudeAvailable")]
    claude_available: bool,
    #[serde(rename = "monitorSettings")]
    monitor_settings: MonitorSettings,
    #[serde(rename = "windowMode")]
    window_mode: String,
}

#[tauri::command]
fn island_bootstrap(state: State<IslandState>) -> Result<BootstrapPayload, String> {
    let mode = state
        .window_mode
        .lock()
        .map_err(|_| "window mode lock failed".to_string())?
        .clone();
    Ok(BootstrapPayload {
        socket_path: state.action_bridge.socket_path().to_string_lossy().into_owned(),
        claude_available: claude_available(),
        monitor_settings: read_monitor_settings(),
        window_mode: mode,
    })
}

#[tauri::command]
fn island_tick(state: State<IslandState>) -> Result<agent_monitor_core::TickPayload, String> {
    let settings = read_monitor_settings();
    let paths = MonitorPaths::from_env(repo_bindings_file());
    let mut memory = state
        .monitor_memory
        .lock()
        .map_err(|_| "monitor memory lock failed".to_string())?;
    tick_monitor(
        &settings,
        &mut memory,
        &state.action_bridge,
        &paths,
        TickOptions {
            actionable_notifications: true,
        },
    )
}

#[tauri::command]
fn island_title_map() -> Result<HashMap<String, String>, String> {
    Ok(load_codex_title_map())
}

#[tauri::command]
fn island_perform_action(state: State<IslandState>, action_id: String, choice_id: String) -> Result<(), String> {
    state.action_bridge.perform_action(&action_id, &choice_id)
}

#[tauri::command]
fn island_jump_to_session(source: String, session_id: String, repo_path: Option<String>) -> Result<(), String> {
    if source == "codex" {
        if launch_codex_app(repo_path.clone()).is_ok() {
            return Ok(());
        }
    }
    if let Some(repo_path) = repo_path.as_ref() {
        if Path::new(repo_path).exists() {
            let (program, args) = resume_command(&source, &session_id);
            return launch_terminal_command(&program, &args, Some(repo_path.clone()));
        }
    }
    let sessions_root = if source == "opencode" {
        opencode_data_dir()
    } else {
        codex_home_dir()
    };
    opener::open(sessions_root).map_err(|e| e.to_string())
}

#[tauri::command]
fn island_launch_agent(state: State<IslandState>, source: String, cwd: Option<String>) -> Result<(), String> {
    let normalized = source.trim().to_lowercase();
    let command = match normalized.as_str() {
        "claude" => "claude",
        "codex" => "codex",
        "opencode" => "opencode",
        _ => return Err("Unknown agent source".to_string()),
    };
    if !command_available(command) {
        return Err(format!("Command `{}` not found in PATH", command));
    }
    let launch_dir = cwd.unwrap_or_else(|| {
        std::env::current_dir()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|_| ".".to_string())
    });
    let socket = state.action_bridge.socket_path().to_string_lossy().into_owned();
    let exports = format!(
        "export AGENT_ISLAND_SOCKET=\"{}\"; export AGENT_ISLAND_SOURCE=\"{}\";",
        socket.replace('"', "\\\""),
        normalized
    );
    let script = format!(
        "{} cd {} && {}",
        exports,
        shell_quote(&launch_dir),
        shell_quote(command),
    );
    launch_terminal_shell_script(&script, Some(launch_dir))
}

#[tauri::command]
fn island_set_window_mode(
    state: State<IslandState>,
    mode: String,
) -> Result<(), String> {
    let normalized = if mode == "expanded" { "expanded" } else { "collapsed" };
    {
        let mut lock = state
            .window_mode
            .lock()
            .map_err(|_| "window mode lock failed".to_string())?;
        *lock = normalized.to_string();
    }
    write_window_mode(normalized)
}

#[tauri::command]
fn island_set_monitor_settings(settings: MonitorSettings) -> Result<(), String> {
    write_json_file(
        &monitor_settings_file(),
        &serde_json::to_value(settings).map_err(|e| e.to_string())?,
    )
}

#[tauri::command]
fn island_open_path(path: String) -> Result<(), String> {
    opener::open(path).map_err(|e| e.to_string())
}

fn launch_codex_app(repo_path: Option<String>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("open");
        command.args(["-a", "Codex"]);
        if let Some(path) = repo_path.as_ref().filter(|value| Path::new(value).exists()) {
            command.arg(path);
        }
        command.spawn().map_err(|e| e.to_string())?;
        return Ok(());
    }

    #[cfg(not(target_os = "macos"))]
    {
        if !command_available("codex") {
            return Err("Command `codex` not found in PATH".to_string());
        }
        let mut command = Command::new("codex");
        command.arg("app");
        if let Some(path) = repo_path.as_ref().filter(|value| Path::new(value).exists()) {
            command.arg(path);
        }
        command.spawn().map_err(|e| e.to_string())?;
        return Ok(());
    }
}

fn load_codex_title_map() -> HashMap<String, String> {
    let mut out = HashMap::new();
    for root in [codex_home_dir(), legacy_codex_home_dir()] {
        let index_path = root.join("session_index.jsonl");
        let raw = match std::fs::read_to_string(index_path) {
            Ok(value) => value,
            Err(_) => continue,
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
            let session_id = value
                .get("id")
                .and_then(Value::as_str)
                .map(|value| value.to_string());
            let title = value
                .get("thread_name")
                .or_else(|| value.get("threadName"))
                .or_else(|| value.get("title"))
                .and_then(Value::as_str)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            if let (Some(session_id), Some(title)) = (session_id, title) {
                out.insert(session_id, title);
            }
        }
    }
    out
}

fn apply_window_mode(window: &WebviewWindow, width: f64, height: f64) -> Result<(), String> {
    window
        .set_size(Size::Logical(LogicalSize::new(width, height)))
        .map_err(|e| e.to_string())?;
    let monitor = window
        .primary_monitor()
        .map_err(|e| e.to_string())?
        .or_else(|| window.current_monitor().ok().flatten())
        .ok_or_else(|| "No display available".to_string())?;
    let scale = monitor.scale_factor();
    let size = monitor.size().to_logical::<f64>(scale);
    let position = monitor.position().to_logical::<f64>(scale);
    let x = position.x + ((size.width - width) / 2.0);
    let y = position.y + TOP_MARGIN;
    eprintln!(
        "[agent-island] monitor scale={scale:.2} pos=({}, {}) size=({}, {}) -> window pos=({x}, {y}) size=({width}, {height})",
        position.x, position.y, size.width, size.height
    );
    window
        .set_position(Position::Logical(LogicalPosition::new(x, y)))
        .map_err(|e| e.to_string())
}

fn launch_terminal_command(program: &str, args: &[String], cwd: Option<String>) -> Result<(), String> {
    let command = shell_command(program, args);
    launch_terminal_shell_script(&command, cwd)
}

fn launch_terminal_shell_script(script: &str, cwd: Option<String>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let resolved_cwd = cwd.unwrap_or_else(|| ".".to_string());
        let terminal_command = format!(
            "cd {} && {}",
            shell_quote(&resolved_cwd),
            script,
        );
        let apple_script = format!(
            "tell application \"Terminal\" to activate\ntell application \"Terminal\" to do script {}",
            apple_script_string_literal(&terminal_command),
        );
        Command::new("osascript")
            .args(["-e", &apple_script])
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let launch_cmd = if let Some(cwd) = cwd {
            format!("cd {} && {}", shell_quote(&cwd), script)
        } else {
            script.to_string()
        };
        Command::new("x-terminal-emulator")
            .args(["-e", "sh", "-lc", &launch_cmd])
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        let launch_cmd = if let Some(cwd) = cwd {
            format!("cd /d {} && {}", cmd_quote(&cwd), script)
        } else {
            script.to_string()
        };
        Command::new("cmd")
            .args(["/C", "start", "cmd", "/K", &launch_cmd])
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err("Unsupported platform".to_string())
}

fn resume_command(source: &str, session_id: &str) -> (String, Vec<String>) {
    match source {
        "opencode" => ("opencode".to_string(), vec!["resume".to_string(), session_id.to_string()]),
        "claude" => ("claude".to_string(), vec!["--resume".to_string(), session_id.to_string()]),
        _ => ("codex".to_string(), vec!["resume".to_string(), session_id.to_string()]),
    }
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    let escaped = value.replace('\'', "'\"'\"'");
    format!("'{}'", escaped)
}

#[cfg(windows)]
fn shell_command(program: &str, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(cmd_quote(program));
    for arg in args {
        parts.push(cmd_quote(arg));
    }
    parts.join(" ")
}

#[cfg(not(windows))]
fn shell_command(program: &str, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(shell_quote(program));
    for arg in args {
        parts.push(shell_quote(arg));
    }
    parts.join(" ")
}

fn apple_script_string_literal(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{}\"", escaped)
}

#[cfg(windows)]
fn cmd_quote(value: &str) -> String {
    let escaped = value.replace('"', "\"\"");
    format!("\"{}\"", escaped)
}

fn agent_island_dir() -> PathBuf {
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".agent-island")
}

fn legacy_codex_home_dir() -> PathBuf {
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".Codex")
}

fn settings_file() -> PathBuf {
    agent_island_dir().join("settings.json")
}

fn read_window_mode() -> String {
    read_json_file(&settings_file())
        .ok()
        .and_then(|value| value.get("windowMode").and_then(Value::as_str).map(|value| value.to_string()))
        .unwrap_or_else(|| "collapsed".to_string())
}

fn write_window_mode(mode: &str) -> Result<(), String> {
    let mut settings = read_json_file(&settings_file()).unwrap_or_else(|_| json!({}));
    if !settings.is_object() {
        settings = json!({});
    }
    if let Some(map) = settings.as_object_mut() {
        map.insert("windowMode".to_string(), Value::String(mode.to_string()));
    }
    write_json_file(&settings_file(), &settings)
}

fn read_monitor_settings() -> MonitorSettings {
    read_json_file(&monitor_settings_file())
        .or_else(|_| read_json_file(&legacy_monitor_settings_file()))
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

fn monitor_settings_file() -> PathBuf {
    agent_island_dir().join("monitor-settings.json")
}

fn repo_bindings_file() -> PathBuf {
    let primary = agent_island_dir().join("monitor-repo-bindings.json");
    if primary.exists() {
        primary
    } else {
        legacy_repo_bindings_file()
    }
}

fn legacy_monitor_settings_file() -> PathBuf {
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".pixel-agents")
        .join("monitor-settings.json")
}

fn legacy_repo_bindings_file() -> PathBuf {
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".pixel-agents")
        .join("monitor-repo-bindings.json")
}

fn codex_home_dir() -> PathBuf {
    std::env::var("CODEX_HOME")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".codex")
        })
}

fn opencode_data_dir() -> PathBuf {
    std::env::var("OPENCODE_DATA_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".local")
                .join("share")
                .join("opencode")
        })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = IslandState::new().expect("failed to initialize Agent Island");
    tauri::Builder::default()
        .manage(state)
        .setup(|app| {
            eprintln!("[agent-island] setup:start");
            let window = app.get_webview_window("main").ok_or("missing main window")?;
            let (width, height) = (EXPANDED_WIDTH, EXPANDED_HEIGHT);
            apply_window_mode(&window, width, height).map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
            eprintln!("[agent-island] setup:positioned");
            window.show().map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
            eprintln!("[agent-island] setup:show");
            match window.unminimize() {
                Ok(_) => eprintln!("[agent-island] setup:unminimize"),
                Err(error) => eprintln!("[agent-island] setup:unminimize-error {error}"),
            }
            match window.set_focus() {
                Ok(_) => eprintln!("[agent-island] setup:focus"),
                Err(error) => eprintln!("[agent-island] setup:focus-error {error}"),
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            island_bootstrap,
            island_tick,
            island_title_map,
            island_perform_action,
            island_jump_to_session,
            island_launch_agent,
            island_set_window_mode,
            island_set_monitor_settings,
            island_open_path
        ])
        .run(tauri::generate_context!())
        .expect("error while running Agent Island");
}
