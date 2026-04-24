use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager, State};
use wait_timeout::ChildExt;

const COMMAND_LOG_LIMIT: usize = 80;
const CLI_TIMEOUT_MS: u64 = 12_000;
const STATUS_TIMEOUT_MS: u64 = 12_000;
const REFRESH_TIMEOUT_MS: u64 = 25_000;
const LOCAL_STALE_MS: i64 = 30 * 60 * 1000;
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

static COMMAND_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct AppState {
    logs: Arc<Mutex<VecDeque<CommandExecutionDto>>>,
    pub(crate) dirs: InternalDirectories,
}

impl AppState {
    pub fn new(app: &AppHandle) -> Self {
        let dirs = InternalDirectories::detect(app);
        let logs = load_logs(&dirs.app_log_file);
        Self {
            logs: Arc::new(Mutex::new(logs)),
            dirs,
        }
    }

    fn push_log(&self, log: CommandExecutionDto) {
        let mut guard = self.logs.lock().expect("command log lock poisoned");
        guard.push_front(log);

        while guard.len() > COMMAND_LOG_LIMIT {
            guard.pop_back();
        }

        persist_logs(&self.dirs.app_log_file, &guard);
    }

    fn recent_logs(&self) -> Vec<CommandExecutionDto> {
        self.logs
            .lock()
            .expect("command log lock poisoned")
            .iter()
            .cloned()
            .collect()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct InternalDirectories {
    codex_root: PathBuf,
    pub(crate) accounts_dir: PathBuf,
    sessions_dir: PathBuf,
    registry_path: PathBuf,
    app_log_dir: PathBuf,
    app_log_file: PathBuf,
}

impl InternalDirectories {
    fn detect(app: &AppHandle) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let codex_root = home.join(".codex");
        let accounts_dir = codex_root.join("accounts");
        let sessions_dir = codex_root.join("sessions");
        let registry_path = accounts_dir.join("registry.json");
        let app_log_dir = app
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| codex_root.join("gui-cache"));
        let _ = fs::create_dir_all(&app_log_dir);
        let app_log_file = app_log_dir.join("command-history.json");

        Self {
            codex_root,
            accounts_dir,
            sessions_dir,
            registry_path,
            app_log_dir,
            app_log_file,
        }
    }

    fn dto(&self) -> DirectorySetDto {
        DirectorySetDto {
            codex_root: path_string(&self.codex_root),
            accounts_dir: path_string(&self.accounts_dir),
            sessions_dir: path_string(&self.sessions_dir),
            registry_path: path_string(&self.registry_path),
            app_log_dir: path_string(&self.app_log_dir),
            app_log_file: path_string(&self.app_log_file),
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshotDto {
    pub registry: RegistrySnapshotDto,
    pub dashboard: DashboardSnapshotDto,
    pub diagnostics: DiagnosticsSnapshotDto,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistrySnapshotDto {
    pub schema_version: Option<u32>,
    pub registry_path: String,
    pub accounts_dir: String,
    pub active_account_key: Option<String>,
    pub active_account_activated_at_ms: Option<i64>,
    pub auto_switch_enabled: bool,
    pub usage_mode: String,
    pub account_api_enabled: bool,
    pub accounts: Vec<AccountDto>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSnapshotDto {
    pub active_account: Option<AccountDto>,
    pub remaining_5h_percent: Option<i64>,
    pub remaining_weekly_percent: Option<i64>,
    pub usage_mode: String,
    pub auto_switch_enabled: bool,
    pub data_freshness: String,
    pub env_checks: Vec<EnvCheckDto>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsSnapshotDto {
    pub env_checks: Vec<EnvCheckDto>,
    pub directories: DirectorySetDto,
    pub recent_logs: Vec<CommandExecutionDto>,
    pub latest_status_log: Option<CommandExecutionDto>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectorySetDto {
    pub codex_root: String,
    pub accounts_dir: String,
    pub sessions_dir: String,
    pub registry_path: String,
    pub app_log_dir: String,
    pub app_log_file: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvCheckDto {
    pub key: String,
    pub label: String,
    pub ok: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub message: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindowDto {
    pub used_percent: Option<i64>,
    pub remaining_percent: Option<i64>,
    pub window_minutes: Option<i64>,
    pub resets_at_ms: Option<i64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDto {
    pub account_key: String,
    pub chatgpt_account_id: Option<String>,
    pub chatgpt_user_id: Option<String>,
    pub email: String,
    pub alias: String,
    pub account_name: Option<String>,
    pub plan: String,
    pub auth_mode: String,
    pub active: bool,
    pub created_at_ms: Option<i64>,
    pub last_used_at_ms: Option<i64>,
    pub last_usage_at_ms: Option<i64>,
    pub last_local_rollout_ms: Option<i64>,
    pub auth_status: String,
    pub auth_status_code: Option<i32>,
    pub auth_status_detail: Option<String>,
    pub auth_checked_at_ms: Option<i64>,
    pub primary_usage: Option<UsageWindowDto>,
    pub weekly_usage: Option<UsageWindowDto>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationResultDto {
    pub command: CommandExecutionDto,
    pub registry: RegistrySnapshotDto,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandExecutionDto {
    pub id: String,
    pub category: String,
    pub executable_path: String,
    pub display_command: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
    pub duration_ms: i64,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub timed_out: bool,
    pub stdout: String,
    pub stderr: String,
}

impl CommandExecutionDto {
    fn synthetic(
        category: &str,
        executable_path: &str,
        display_command: &str,
        args: Vec<String>,
        cwd: &Path,
        success: bool,
        stdout: String,
        stderr: String,
    ) -> Self {
        let started_at_ms = now_ms();
        let finished_at_ms = now_ms();
        Self {
            id: next_command_id(),
            category: category.to_string(),
            executable_path: executable_path.to_string(),
            display_command: display_command.to_string(),
            args,
            cwd: path_string(cwd),
            started_at_ms,
            finished_at_ms,
            duration_ms: finished_at_ms - started_at_ms,
            exit_code: if success { Some(0) } else { None },
            success,
            timed_out: false,
            stdout,
            stderr,
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryChangedEventDto {
    pub kind: String,
    pub paths: Vec<String>,
    pub timestamp_ms: i64,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RegistryFile {
    schema_version: Option<u32>,
    active_account_key: Option<String>,
    active_account_activated_at_ms: Option<i64>,
    auto_switch: RegistryAutoSwitch,
    api: RegistryApi,
    accounts: Vec<RegistryAccount>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RegistryAutoSwitch {
    enabled: bool,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RegistryApi {
    usage: bool,
    account: bool,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RegistryAccount {
    account_key: String,
    chatgpt_account_id: Option<String>,
    chatgpt_user_id: Option<String>,
    email: String,
    alias: Option<String>,
    account_name: Option<String>,
    plan: Option<String>,
    auth_mode: Option<String>,
    created_at: Option<i64>,
    last_used_at: Option<i64>,
    last_usage: Option<RegistryUsage>,
    last_usage_at: Option<i64>,
    last_local_rollout: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RegistryUsage {
    primary: Option<RegistryUsageWindow>,
    secondary: Option<RegistryUsageWindow>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RegistryUsageWindow {
    used_percent: Option<i64>,
    window_minutes: Option<i64>,
    resets_at: Option<i64>,
}

#[derive(Clone)]
struct ResolvedCommand {
    launcher_path: String,
    display_path: String,
    base_args: Vec<String>,
}

struct AccountSelector {
    query: String,
    account_key: Option<String>,
}

pub fn spawn_registry_watcher(app: AppHandle, watch_dir: PathBuf) {
    std::thread::spawn(move || {
        let emitter = app.clone();
        let mut watcher = match RecommendedWatcher::new(
            move |result: notify::Result<Event>| {
                if let Ok(event) = result {
                    if !is_registry_event(&event) {
                        return;
                    }

                    let payload = RegistryChangedEventDto {
                        kind: format!("{:?}", event.kind),
                        paths: event.paths.iter().map(|path| path_string(path)).collect(),
                        timestamp_ms: now_ms(),
                    };

                    let _ = emitter.emit("registry-changed", payload);
                }
            },
            Config::default(),
        ) {
            Ok(watcher) => watcher,
            Err(_) => return,
        };

        if watcher
            .watch(&watch_dir, RecursiveMode::NonRecursive)
            .is_err()
        {
            return;
        }

        loop {
            std::thread::park();
        }
    });
}

#[tauri::command]
pub fn get_app_snapshot(state: State<'_, AppState>) -> AppSnapshotDto {
    build_app_snapshot(&state)
}

#[tauri::command]
pub async fn run_codex_auth_status(
    state: State<'_, AppState>,
) -> Result<CommandExecutionDto, String> {
    let state = state.inner().clone();
    let fallback_state = state.clone();

    match tauri::async_runtime::spawn_blocking(move || {
        let log = run_codex_auth_command(&state, &["status"], "status", STATUS_TIMEOUT_MS);
        state.push_log(log.clone());
        log
    })
    .await
    {
        Ok(log) => Ok(log),
        Err(error) => {
            let log = CommandExecutionDto::synthetic(
                "status",
                "codex-auth",
                "codex-auth status",
                vec!["status".to_string()],
                &fallback_state.dirs.codex_root,
                false,
                String::new(),
                format!("Failed to run status worker: {error}"),
            );
            fallback_state.push_log(log.clone());
            Ok(log)
        }
    }
}

#[tauri::command]
pub async fn refresh_registry_snapshot(
    state: State<'_, AppState>,
) -> Result<MutationResultDto, String> {
    let state = state.inner().clone();
    let fallback_state = state.clone();

    match tauri::async_runtime::spawn_blocking(move || {
        let command = run_codex_auth_command(
            &state,
            &["list", "--debug"],
            "refresh-registry",
            REFRESH_TIMEOUT_MS,
        );
        state.push_log(command.clone());
        let account_auth_statuses = extract_account_auth_statuses(&state.recent_logs());
        let registry = read_registry_snapshot(&state.dirs, &account_auth_statuses);
        MutationResultDto { command, registry }
    })
    .await
    {
        Ok(result) => Ok(result),
        Err(error) => {
            let command = CommandExecutionDto::synthetic(
                "refresh-registry",
                "codex-auth",
                "codex-auth list --debug",
                vec!["list".to_string(), "--debug".to_string()],
                &fallback_state.dirs.codex_root,
                false,
                String::new(),
                format!("Failed to run refresh worker: {error}"),
            );
            fallback_state.push_log(command.clone());
            let registry = read_registry_snapshot(&fallback_state.dirs, &HashMap::new());
            Ok(MutationResultDto { command, registry })
        }
    }
}

#[tauri::command]
pub fn switch_account(query: String, state: State<'_, AppState>) -> MutationResultDto {
    let selector = resolve_account_selector(&state.dirs, &query, "switch");
    let target_account_key = selector
        .as_ref()
        .ok()
        .and_then(|selector| selector.account_key.clone());
    let mut command = match selector {
        Ok(selector) => run_query_command(&state, "switch", "switch", selector.query),
        Err(error) => CommandExecutionDto::synthetic(
            "switch",
            "codex-auth",
            "codex-auth switch",
            vec!["switch".to_string(), query.clone()],
            &state.dirs.codex_root,
            false,
            String::new(),
            error,
        ),
    };
    let registry = read_registry_snapshot(&state.dirs, &HashMap::new());
    if command.success && command.stdout.contains("Select account to activate:") {
        command.success = false;
        command.exit_code = Some(1);
        command.stderr =
            "codex-auth switch entered interactive selection; GUI requires a unique account selector.".to_string();
    }
    if command.success {
        if let Some(account_key) = target_account_key {
            if registry.active_account_key.as_deref() != Some(account_key.as_str()) {
                command.success = false;
                command.exit_code = Some(1);
                command.stderr = format!(
                    "codex-auth switch completed but active account is not requested account: {}",
                    account_key
                );
            }
        }
    }
    state.push_log(command.clone());
    MutationResultDto { command, registry }
}

#[tauri::command]
pub fn remove_account(query: String, state: State<'_, AppState>) -> MutationResultDto {
    let selector = resolve_account_selector(&state.dirs, &query, "remove");
    let target_account_key = selector
        .as_ref()
        .ok()
        .and_then(|selector| selector.account_key.clone());
    let mut command = match selector {
        Ok(selector) => run_query_command(&state, "remove", "remove", selector.query),
        Err(error) => CommandExecutionDto::synthetic(
            "remove",
            "codex-auth",
            "codex-auth remove",
            vec!["remove".to_string(), query.clone()],
            &state.dirs.codex_root,
            false,
            String::new(),
            error,
        ),
    };
    let registry = read_registry_snapshot(&state.dirs, &HashMap::new());
    if command.success && command.stdout.contains("Select account") {
        command.success = false;
        command.exit_code = Some(1);
        command.stderr =
            "codex-auth remove entered interactive selection; GUI requires a unique account selector.".to_string();
    }
    if command.success {
        if let Some(account_key) = target_account_key {
            if registry
                .accounts
                .iter()
                .any(|account| account.account_key == account_key)
            {
                command.success = false;
                command.exit_code = Some(1);
                command.stderr = format!(
                    "codex-auth remove completed but requested account is still present: {}",
                    account_key
                );
            }
        }
    }
    state.push_log(command.clone());
    MutationResultDto { command, registry }
}

#[tauri::command]
pub fn set_account_alias(
    account_key: String,
    alias: String,
    state: State<'_, AppState>,
) -> MutationResultDto {
    let trimmed_key = account_key.trim();
    let trimmed_alias = alias.trim();
    let mut command = match write_account_alias(&state.dirs, trimmed_key, trimmed_alias) {
        Ok(backup_path) => CommandExecutionDto::synthetic(
            "set-alias",
            "registry.json",
            "set account alias",
            vec![
                "set-alias".to_string(),
                trimmed_key.to_string(),
                trimmed_alias.to_string(),
            ],
            &state.dirs.codex_root,
            true,
            format!(
                "Alias set to '{}' for account {}. Backup: {}",
                trimmed_alias, trimmed_key, backup_path
            ),
            String::new(),
        ),
        Err(error) => CommandExecutionDto::synthetic(
            "set-alias",
            "registry.json",
            "set account alias",
            vec![
                "set-alias".to_string(),
                trimmed_key.to_string(),
                trimmed_alias.to_string(),
            ],
            &state.dirs.codex_root,
            false,
            String::new(),
            error,
        ),
    };
    let registry = read_registry_snapshot(&state.dirs, &HashMap::new());
    if command.success
        && !registry
            .accounts
            .iter()
            .any(|account| account.account_key == trimmed_key && account.alias == trimmed_alias)
    {
        command.success = false;
        command.exit_code = Some(1);
        command.stderr = format!(
            "Alias write completed but registry does not show '{}' for {}.",
            trimmed_alias, trimmed_key
        );
    }
    state.push_log(command.clone());
    MutationResultDto { command, registry }
}

#[tauri::command]
pub fn import_auth_file(
    path: String,
    alias: Option<String>,
    state: State<'_, AppState>,
) -> MutationResultDto {
    let mut args = vec!["import".to_string(), path];
    if let Some(alias_value) = clean_optional(alias) {
        args.push("--alias".to_string());
        args.push(alias_value);
    }
    let command = run_codex_auth_command_owned(&state, args, "import-file", CLI_TIMEOUT_MS);
    let registry = read_registry_snapshot(&state.dirs, &HashMap::new());
    state.push_log(command.clone());
    MutationResultDto { command, registry }
}

#[tauri::command]
pub fn import_auth_directory(path: String, state: State<'_, AppState>) -> MutationResultDto {
    let command = run_codex_auth_command_owned(
        &state,
        vec!["import".to_string(), path],
        "import-directory",
        CLI_TIMEOUT_MS,
    );
    let registry = read_registry_snapshot(&state.dirs, &HashMap::new());
    state.push_log(command.clone());
    MutationResultDto { command, registry }
}

#[tauri::command]
pub fn import_cpa(
    path: Option<String>,
    alias: Option<String>,
    state: State<'_, AppState>,
) -> MutationResultDto {
    let mut args = vec!["import".to_string(), "--cpa".to_string()];
    if let Some(path_value) = clean_optional(path) {
        args.push(path_value);
    }
    if let Some(alias_value) = clean_optional(alias) {
        args.push("--alias".to_string());
        args.push(alias_value);
    }
    let command = run_codex_auth_command_owned(&state, args, "import-cpa", CLI_TIMEOUT_MS);
    let registry = read_registry_snapshot(&state.dirs, &HashMap::new());
    state.push_log(command.clone());
    MutationResultDto { command, registry }
}

#[tauri::command]
pub fn rebuild_registry(path: Option<String>, state: State<'_, AppState>) -> MutationResultDto {
    let mut args = vec!["import".to_string(), "--purge".to_string()];
    if let Some(path_value) = clean_optional(path) {
        args.push(path_value);
    }
    let command = run_codex_auth_command_owned(&state, args, "rebuild-registry", CLI_TIMEOUT_MS);
    let registry = read_registry_snapshot(&state.dirs, &HashMap::new());
    state.push_log(command.clone());
    MutationResultDto { command, registry }
}

#[tauri::command]
pub fn set_auto_switch(enabled: bool, state: State<'_, AppState>) -> MutationResultDto {
    let action = if enabled { "enable" } else { "disable" };
    let command = run_codex_auth_command(
        &state,
        &["config", "auto", action],
        "config-auto",
        CLI_TIMEOUT_MS,
    );
    let registry = read_registry_snapshot(&state.dirs, &HashMap::new());
    state.push_log(command.clone());
    MutationResultDto { command, registry }
}

#[tauri::command]
pub fn set_usage_api_mode(enabled: bool, state: State<'_, AppState>) -> MutationResultDto {
    let action = if enabled { "enable" } else { "disable" };
    let command = run_codex_auth_command(
        &state,
        &["config", "api", action],
        "config-api",
        CLI_TIMEOUT_MS,
    );
    let registry = read_registry_snapshot(&state.dirs, &HashMap::new());
    state.push_log(command.clone());
    MutationResultDto { command, registry }
}

#[tauri::command]
pub fn record_ui_event(
    name: String,
    detail: Option<String>,
    state: State<'_, AppState>,
) -> CommandExecutionDto {
    let trimmed_name = name.trim();
    let event_name = if trimmed_name.is_empty() {
        "unnamed-ui-event"
    } else {
        trimmed_name
    };
    let log = CommandExecutionDto::synthetic(
        "ui-event",
        "frontend",
        event_name,
        vec![event_name.to_string()],
        &state.dirs.codex_root,
        true,
        detail.unwrap_or_default(),
        String::new(),
    );
    state.push_log(log.clone());
    log
}

#[tauri::command]
pub fn launch_login(device_auth: bool, state: State<'_, AppState>) -> CommandExecutionDto {
    let args = if device_auth {
        vec!["login".to_string(), "--device-auth".to_string()]
    } else {
        vec!["login".to_string()]
    };

    let Some(target_path) = resolve_command_path_for_powershell("codex-auth") else {
        let log = CommandExecutionDto::synthetic(
            "launch-login",
            "codex-auth",
            "codex-auth login",
            args,
            &state.dirs.codex_root,
            false,
            String::new(),
            "PATH 中未找到 codex-auth。".to_string(),
        );
        state.push_log(log.clone());
        return log;
    };

    let mut command = Command::new("powershell.exe");
    let display_command = if !device_auth {
        let script_path = state.dirs.app_log_dir.join("codex-auth-web-login.ps1");
        if let Err(error) = write_web_login_script(&script_path, &target_path) {
            let log = CommandExecutionDto::synthetic(
                "launch-login",
                "powershell.exe",
                format!("{} login", path_string(&target_path)).as_str(),
                args,
                &state.dirs.codex_root,
                false,
                String::new(),
                error.to_string(),
            );
            state.push_log(log.clone());
            return log;
        }

        command.args(["-NoExit", "-ExecutionPolicy", "Bypass", "-File"]);
        command.arg(path_string(&script_path));
        format!("{} login", path_string(&target_path))
    } else {
        let is_powershell_script = target_path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.eq_ignore_ascii_case("ps1"))
            .unwrap_or(false);

        if is_powershell_script {
            command.args(["-NoExit", "-ExecutionPolicy", "Bypass", "-File"]);
            command.arg(path_string(&target_path));
            command.arg("login");
            command.arg("--device-auth");
        } else {
            let mut command_text = format!(
                "& '{}' login",
                powershell_escape(&path_string(&target_path))
            );
            command_text.push_str(" --device-auth");
            command.args(["-NoExit", "-ExecutionPolicy", "Bypass", "-Command"]);
            command.arg(&command_text);
        }

        format!("{} login --device-auth", path_string(&target_path))
    };
    command.current_dir(&state.dirs.codex_root);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NEW_CONSOLE);
    }

    let log = match command.spawn() {
        Ok(_) => CommandExecutionDto::synthetic(
            "launch-login",
            "powershell.exe",
            display_command.as_str(),
            args,
            &state.dirs.codex_root,
            true,
            if device_auth {
                "已打开外部 PowerShell 登录窗口。".to_string()
            } else {
                "已打开外部 PowerShell 登录窗口，并会自动打开授权网页。".to_string()
            },
            String::new(),
        ),
        Err(error) => CommandExecutionDto::synthetic(
            "launch-login",
            "powershell.exe",
            display_command.as_str(),
            args,
            &state.dirs.codex_root,
            false,
            String::new(),
            error.to_string(),
        ),
    };

    state.push_log(log.clone());
    log
}

fn write_web_login_script(script_path: &Path, target_path: &Path) -> std::io::Result<()> {
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let target = powershell_escape(&path_string(target_path));
    let script = format!(
        r#"$ErrorActionPreference = 'Continue'
$opened = $false

& '{target}' login 2>&1 | ForEach-Object {{
    if ($_ -is [System.Management.Automation.ErrorRecord]) {{
        $line = $_.Exception.Message.Trim()
    }} else {{
        $line = $_.ToString().Trim()
    }}
    Write-Host $line

    if (-not $opened -and $line -match 'https://auth\.openai\.com/oauth/authorize\?\S+') {{
        $opened = $true
        $url = $Matches[0]
        Write-Host ''
        Write-Host 'Opening browser for Codex login...'
        Start-Process $url
    }}
}}

$exitCode = $LASTEXITCODE
Write-Host ''
Write-Host "codex-auth login exited with code $exitCode"
exit $exitCode
"#
    );

    fs::write(script_path, script)
}

#[tauri::command]
pub fn open_diagnostic_path(target: String, state: State<'_, AppState>) -> CommandExecutionDto {
    let maybe_path = match target.as_str() {
        "codexRoot" => Some(state.dirs.codex_root.clone()),
        "accountsDir" => Some(state.dirs.accounts_dir.clone()),
        "sessionsDir" => Some(state.dirs.sessions_dir.clone()),
        "registryFile" => Some(state.dirs.registry_path.clone()),
        "logsDir" => Some(state.dirs.app_log_dir.clone()),
        "logsFile" => Some(state.dirs.app_log_file.clone()),
        _ => None,
    };

    let Some(path) = maybe_path else {
        let log = CommandExecutionDto::synthetic(
            "open-path",
            "explorer.exe",
            "explorer.exe",
            vec![target],
            &state.dirs.codex_root,
            false,
            String::new(),
            "未知的诊断目标。".to_string(),
        );
        state.push_log(log.clone());
        return log;
    };

    let log = run_host_command(
        "open-path",
        "explorer.exe",
        vec![path_string(&path)],
        &state.dirs.codex_root,
        5_000,
        false,
    );
    state.push_log(log.clone());
    log
}

fn build_app_snapshot(state: &AppState) -> AppSnapshotDto {
    let recent_logs = state.recent_logs();
    let account_auth_statuses = extract_account_auth_statuses(&recent_logs);
    let registry = read_registry_snapshot(&state.dirs, &account_auth_statuses);
    let env_checks = collect_env_checks();
    let warnings = build_dashboard_warnings(&registry, &env_checks);
    let active_account = registry
        .accounts
        .iter()
        .find(|account| account.active)
        .cloned();

    let dashboard = DashboardSnapshotDto {
        remaining_5h_percent: active_account
            .as_ref()
            .and_then(|account| account.primary_usage.as_ref())
            .and_then(|usage| usage.remaining_percent),
        remaining_weekly_percent: active_account
            .as_ref()
            .and_then(|account| account.weekly_usage.as_ref())
            .and_then(|usage| usage.remaining_percent),
        usage_mode: registry.usage_mode.clone(),
        auto_switch_enabled: registry.auto_switch_enabled,
        data_freshness: compute_freshness(&registry, active_account.as_ref()),
        active_account,
        env_checks: env_checks.clone(),
        warnings,
    };

    let latest_status_log = recent_logs
        .iter()
        .find(|entry| entry.category == "status")
        .cloned();

    let diagnostics = DiagnosticsSnapshotDto {
        env_checks,
        directories: state.dirs.dto(),
        recent_logs,
        latest_status_log,
    };

    AppSnapshotDto {
        registry,
        dashboard,
        diagnostics,
    }
}

fn build_dashboard_warnings(
    registry: &RegistrySnapshotDto,
    env_checks: &[EnvCheckDto],
) -> Vec<String> {
    let mut warnings = registry.warnings.clone();

    if !env_checks.iter().all(|check| check.ok) {
        warnings.push("环境检查失败。请到“诊断”页查看缺失程序或无效版本。".to_string());
    }

    if registry.usage_mode == "local" {
        warnings.push("本地模式下的用量数据可能落后于最新 Codex 会话数据。".to_string());
    }

    if registry
        .accounts
        .iter()
        .any(|account| account.active && account.auth_status == "invalid")
    {
        warnings.push("Active account auth is invalid. Re-login or switch accounts.".to_string());
    }

    warnings
}

fn compute_freshness(
    registry: &RegistrySnapshotDto,
    active_account: Option<&AccountDto>,
) -> String {
    if active_account.is_none() {
        return "missing".to_string();
    }

    if registry.usage_mode == "local" {
        let is_stale = active_account
            .and_then(|account| account.last_usage_at_ms)
            .map(|last_usage_at_ms| now_ms() - last_usage_at_ms > LOCAL_STALE_MS)
            .unwrap_or(true);

        if is_stale {
            return "stale".to_string();
        }
    }

    "fresh".to_string()
}

fn read_registry_snapshot(
    dirs: &InternalDirectories,
    account_auth_statuses: &HashMap<String, AccountAuthStatus>,
) -> RegistrySnapshotDto {
    let mut warnings = Vec::new();

    let file_contents = match fs::read_to_string(&dirs.registry_path) {
        Ok(contents) => contents,
        Err(error) => {
            warnings.push(format!(
                "Failed to read registry.json: {} ({error})",
                dirs.registry_path.display()
            ));
            return RegistrySnapshotDto {
                schema_version: None,
                registry_path: path_string(&dirs.registry_path),
                accounts_dir: path_string(&dirs.accounts_dir),
                active_account_key: None,
                active_account_activated_at_ms: None,
                auto_switch_enabled: false,
                usage_mode: "local".to_string(),
                account_api_enabled: false,
                accounts: Vec::new(),
                warnings,
            };
        }
    };

    let parsed = match serde_json::from_str::<RegistryFile>(&file_contents) {
        Ok(parsed) => parsed,
        Err(error) => {
            warnings.push(format!(
                "Failed to parse registry.json: {} ({error})",
                dirs.registry_path.display()
            ));
            return RegistrySnapshotDto {
                schema_version: None,
                registry_path: path_string(&dirs.registry_path),
                accounts_dir: path_string(&dirs.accounts_dir),
                active_account_key: None,
                active_account_activated_at_ms: None,
                auto_switch_enabled: false,
                usage_mode: "local".to_string(),
                account_api_enabled: false,
                accounts: Vec::new(),
                warnings,
            };
        }
    };

    let active_account_key = parsed.active_account_key.clone();
    let mut accounts = parsed
        .accounts
        .into_iter()
        .map(|account| {
            registry_account_to_dto(
                account,
                active_account_key.as_deref(),
                account_auth_statuses,
            )
        })
        .collect::<Vec<_>>();

    accounts.sort_by(|left, right| {
        right
            .active
            .cmp(&left.active)
            .then(right.last_used_at_ms.cmp(&left.last_used_at_ms))
            .then(left.email.cmp(&right.email))
    });

    RegistrySnapshotDto {
        schema_version: parsed.schema_version,
        registry_path: path_string(&dirs.registry_path),
        accounts_dir: path_string(&dirs.accounts_dir),
        active_account_key,
        active_account_activated_at_ms: parsed.active_account_activated_at_ms,
        auto_switch_enabled: parsed.auto_switch.enabled,
        usage_mode: if parsed.api.usage {
            "api".to_string()
        } else {
            "local".to_string()
        },
        account_api_enabled: parsed.api.account,
        accounts,
        warnings,
    }
}
fn registry_account_to_dto(
    account: RegistryAccount,
    active_account_key: Option<&str>,
    account_auth_statuses: &HashMap<String, AccountAuthStatus>,
) -> AccountDto {
    let auth_status = account_auth_statuses.get(&account.email.to_lowercase());
    AccountDto {
        active: active_account_key
            .map(|key| key == account.account_key)
            .unwrap_or(false),
        account_key: account.account_key,
        chatgpt_account_id: account.chatgpt_account_id,
        chatgpt_user_id: account.chatgpt_user_id,
        email: account.email,
        alias: account.alias.unwrap_or_default(),
        account_name: account.account_name,
        plan: account.plan.unwrap_or_else(|| "unknown".to_string()),
        auth_mode: account.auth_mode.unwrap_or_else(|| "unknown".to_string()),
        created_at_ms: seconds_to_ms(account.created_at),
        last_used_at_ms: seconds_to_ms(account.last_used_at),
        last_usage_at_ms: seconds_to_ms(account.last_usage_at),
        last_local_rollout_ms: account.last_local_rollout,
        auth_status: auth_status
            .map(|status| status.state.clone())
            .unwrap_or_else(|| "unknown".to_string()),
        auth_status_code: auth_status.and_then(|status| status.status_code),
        auth_status_detail: auth_status.and_then(|status| status.detail.clone()),
        auth_checked_at_ms: auth_status.map(|status| status.checked_at_ms),
        primary_usage: account
            .last_usage
            .as_ref()
            .and_then(|usage| usage.primary.as_ref())
            .map(usage_window_to_dto),
        weekly_usage: account
            .last_usage
            .as_ref()
            .and_then(|usage| usage.secondary.as_ref())
            .map(usage_window_to_dto),
    }
}

fn usage_window_to_dto(window: &RegistryUsageWindow) -> UsageWindowDto {
    let remaining = window.used_percent.map(|used| (100 - used).clamp(0, 100));
    UsageWindowDto {
        used_percent: window.used_percent,
        remaining_percent: remaining,
        window_minutes: window.window_minutes,
        resets_at_ms: seconds_to_ms(window.resets_at),
    }
}

#[derive(Clone, Debug)]
struct AccountAuthStatus {
    state: String,
    status_code: Option<i32>,
    detail: Option<String>,
    checked_at_ms: i64,
}

fn extract_account_auth_statuses(
    recent_logs: &[CommandExecutionDto],
) -> HashMap<String, AccountAuthStatus> {
    let Some(latest_refresh_log) = recent_logs
        .iter()
        .find(|entry| entry.category == "refresh-registry")
    else {
        return HashMap::new();
    };

    parse_account_auth_statuses(latest_refresh_log)
}

fn parse_account_auth_statuses(log: &CommandExecutionDto) -> HashMap<String, AccountAuthStatus> {
    let mut statuses = HashMap::new();
    let combined = if log.stderr.is_empty() {
        log.stdout.clone()
    } else if log.stdout.is_empty() {
        log.stderr.clone()
    } else {
        format!("{}\n{}", log.stdout, log.stderr)
    };

    for raw_line in combined.lines() {
        let line = raw_line.trim();
        if !line.starts_with("[debug] response usage: ") {
            continue;
        }

        let payload = &line["[debug] response usage: ".len()..];
        let Some((email, tail)) = payload.split_once(" status=") else {
            continue;
        };

        let email = email.trim().to_lowercase();
        if email.is_empty() {
            continue;
        }

        let (status_code, detail) = parse_debug_status_tail(tail);
        let state = match status_code {
            Some(200) => "ok".to_string(),
            Some(401) => "invalid".to_string(),
            Some(_) => "warning".to_string(),
            None => "unknown".to_string(),
        };

        statuses.insert(
            email,
            AccountAuthStatus {
                state,
                status_code,
                detail,
                checked_at_ms: log.finished_at_ms,
            },
        );
    }

    statuses
}

fn parse_debug_status_tail(tail: &str) -> (Option<i32>, Option<String>) {
    let tail = tail.trim();
    let (status_part, result_part) = match tail.split_once(" result=") {
        Some((status_part, result_part)) => (status_part.trim(), Some(result_part.trim())),
        None => (tail, None),
    };

    let status_code = status_part.parse::<i32>().ok();
    let detail = match (status_code, result_part) {
        (Some(code), Some(result)) if !result.is_empty() => {
            Some(format!("HTTP {} ({})", code, result))
        }
        (Some(code), _) => Some(format!("HTTP {}", code)),
        (None, Some(result)) if !result.is_empty() => Some(result.to_string()),
        _ => None,
    };

    (status_code, detail)
}

fn collect_env_checks() -> Vec<EnvCheckDto> {
    vec![
        env_check_for("codex-auth", "codex-auth"),
        env_check_for("codex", "Codex CLI"),
        env_check_for("node", "Node.js"),
    ]
}

fn env_check_for(key: &str, label: &str) -> EnvCheckDto {
    let path = resolve_command_path(key).map(|path| path_string(&path));
    match path {
        Some(path_value) => {
            let version = command_version(key);
            let message = if key == "node" {
                match version.as_deref() {
                    Some(value) if node_version_ok(value) => "已检测到，版本兼容。".to_string(),
                    Some(_) => "已检测到，但 codex-auth 要求 Node.js 22+。".to_string(),
                    None => "已检测到，但版本探测失败。".to_string(),
                }
            } else if version.is_some() {
                "已检测到，可正常使用。".to_string()
            } else {
                "已检测到，但版本探测失败。".to_string()
            };

            let ok = key != "node" || version.as_deref().map(node_version_ok).unwrap_or(false);

            EnvCheckDto {
                key: key.to_string(),
                label: label.to_string(),
                ok,
                path: Some(path_value),
                version,
                message,
            }
        }
        None => EnvCheckDto {
            key: key.to_string(),
            label: label.to_string(),
            ok: false,
            path: None,
            version: None,
            message: "PATH 中未找到。".to_string(),
        },
    }
}

fn node_version_ok(version: &str) -> bool {
    let cleaned = version.trim_start_matches('v');
    let major = cleaned
        .split('.')
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default();
    major >= 22
}

fn command_version(name: &str) -> Option<String> {
    let resolved = resolve_command(name)?;
    let mut command = Command::new(&resolved.launcher_path);
    command.args(&resolved.base_args);
    command.arg("--version");
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    hide_background_console(&mut command);
    let output = command.output().ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stdout.is_empty() {
        Some(stdout)
    } else if !stderr.is_empty() {
        Some(stderr)
    } else {
        None
    }
}

fn run_query_command(
    state: &AppState,
    category: &str,
    verb: &str,
    query: String,
) -> CommandExecutionDto {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return CommandExecutionDto::synthetic(
            category,
            "codex-auth",
            format!("codex-auth {verb}").as_str(),
            vec![verb.to_string()],
            &state.dirs.codex_root,
            false,
            String::new(),
            "此 GUI 禁用交互式模式，请提供非空查询词。".to_string(),
        );
    }

    run_codex_auth_command_owned(
        state,
        vec![verb.to_string(), trimmed.to_string()],
        category,
        CLI_TIMEOUT_MS,
    )
}

fn resolve_account_selector(
    dirs: &InternalDirectories,
    query: &str,
    verb: &str,
) -> Result<AccountSelector, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err(format!(
            "codex-auth {verb} requires a non-empty account selector."
        ));
    }

    let file_contents = fs::read_to_string(&dirs.registry_path).map_err(|error| {
        format!(
            "Failed to read registry before codex-auth {verb}: {} ({error})",
            dirs.registry_path.display()
        )
    })?;
    let parsed = serde_json::from_str::<RegistryFile>(&file_contents).map_err(|error| {
        format!(
            "Failed to parse registry before codex-auth {verb}: {} ({error})",
            dirs.registry_path.display()
        )
    })?;

    let Some(target) = parsed
        .accounts
        .iter()
        .find(|account| account.account_key == trimmed)
    else {
        return Ok(AccountSelector {
            query: trimmed.to_string(),
            account_key: None,
        });
    };

    if let Some(alias) = clean_optional(target.alias.clone()) {
        let count = parsed
            .accounts
            .iter()
            .filter(|account| {
                clean_optional(account.alias.clone())
                    .map(|value| value.eq_ignore_ascii_case(&alias))
                    .unwrap_or(false)
            })
            .count();
        if count == 1 {
            return Ok(AccountSelector {
                query: alias,
                account_key: Some(target.account_key.clone()),
            });
        }
    }

    if let Some(account_name) = clean_optional(target.account_name.clone()) {
        let count = parsed
            .accounts
            .iter()
            .filter(|account| {
                clean_optional(account.account_name.clone())
                    .map(|value| value.eq_ignore_ascii_case(&account_name))
                    .unwrap_or(false)
            })
            .count();
        if count == 1 {
            return Ok(AccountSelector {
                query: account_name,
                account_key: Some(target.account_key.clone()),
            });
        }
    }

    let email_count = parsed
        .accounts
        .iter()
        .filter(|account| account.email.eq_ignore_ascii_case(&target.email))
        .count();
    if email_count == 1 {
        return Ok(AccountSelector {
            query: target.email.clone(),
            account_key: Some(target.account_key.clone()),
        });
    }

    Err(format!(
        "Cannot {verb} account non-interactively: '{}' matches {email_count} accounts and this account has no unique alias or workspace name.",
        target.email
    ))
}

fn write_account_alias(
    dirs: &InternalDirectories,
    account_key: &str,
    alias: &str,
) -> Result<String, String> {
    if account_key.is_empty() {
        return Err("Account key is required.".to_string());
    }
    if alias.is_empty() {
        return Err("Alias is required.".to_string());
    }
    if alias.chars().count() > 64 {
        return Err("Alias must be 64 characters or fewer.".to_string());
    }
    if !alias
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '@' | '+'))
    {
        return Err(
            "Alias can only contain A-Z, a-z, 0-9, dot, underscore, hyphen, @, or +.".to_string(),
        );
    }

    let file_contents = fs::read_to_string(&dirs.registry_path).map_err(|error| {
        format!(
            "Failed to read registry before setting alias: {} ({error})",
            dirs.registry_path.display()
        )
    })?;
    let mut parsed =
        serde_json::from_str::<serde_json::Value>(&file_contents).map_err(|error| {
            format!(
                "Failed to parse registry before setting alias: {} ({error})",
                dirs.registry_path.display()
            )
        })?;
    let accounts = parsed
        .get_mut("accounts")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| "registry.json does not contain an accounts array.".to_string())?;

    let duplicate_alias = accounts.iter().any(|account| {
        let key_matches = account
            .get("account_key")
            .and_then(serde_json::Value::as_str)
            .map(|value| value == account_key)
            .unwrap_or(false);
        let alias_matches = account
            .get("alias")
            .and_then(serde_json::Value::as_str)
            .map(|value| value.eq_ignore_ascii_case(alias))
            .unwrap_or(false);
        !key_matches && alias_matches
    });
    if duplicate_alias {
        return Err(format!(
            "Alias '{}' is already used by another account.",
            alias
        ));
    }

    let target = accounts
        .iter_mut()
        .find(|account| {
            account
                .get("account_key")
                .and_then(serde_json::Value::as_str)
                .map(|value| value == account_key)
                .unwrap_or(false)
        })
        .ok_or_else(|| format!("Account key not found in registry: {account_key}"))?;
    let target_object = target
        .as_object_mut()
        .ok_or_else(|| "Target account entry is not a JSON object.".to_string())?;
    target_object.insert(
        "alias".to_string(),
        serde_json::Value::String(alias.to_string()),
    );

    let backup_path = dirs
        .registry_path
        .with_file_name(format!("registry.json.bak.alias.{}", now_ms()));
    fs::copy(&dirs.registry_path, &backup_path).map_err(|error| {
        format!(
            "Failed to backup registry before setting alias: {} ({error})",
            backup_path.display()
        )
    })?;
    let serialized = serde_json::to_string_pretty(&parsed)
        .map_err(|error| format!("Failed to serialize registry after setting alias: {error}"))?;
    fs::write(&dirs.registry_path, format!("{serialized}\n")).map_err(|error| {
        format!(
            "Failed to write registry after setting alias: {} ({error})",
            dirs.registry_path.display()
        )
    })?;

    Ok(path_string(&backup_path))
}

fn run_codex_auth_command(
    state: &AppState,
    args: &[&str],
    category: &str,
    timeout_ms: u64,
) -> CommandExecutionDto {
    run_codex_auth_command_owned(
        state,
        args.iter().map(|value| value.to_string()).collect(),
        category,
        timeout_ms,
    )
}

fn run_codex_auth_command_owned(
    state: &AppState,
    args: Vec<String>,
    category: &str,
    timeout_ms: u64,
) -> CommandExecutionDto {
    let Some(resolved) = resolve_command("codex-auth") else {
        return CommandExecutionDto::synthetic(
            category,
            "codex-auth",
            "codex-auth",
            args,
            &state.dirs.codex_root,
            false,
            String::new(),
            "PATH 中未找到 codex-auth。".to_string(),
        );
    };

    run_resolved_command(
        category,
        &resolved,
        args,
        &state.dirs.codex_root,
        timeout_ms,
        true,
    )
}

fn run_host_command(
    category: &str,
    executable: &str,
    args: Vec<String>,
    cwd: &Path,
    timeout_ms: u64,
    hide_window: bool,
) -> CommandExecutionDto {
    let resolved = ResolvedCommand {
        launcher_path: executable.to_string(),
        display_path: executable.to_string(),
        base_args: Vec::new(),
    };

    run_resolved_command(category, &resolved, args, cwd, timeout_ms, hide_window)
}

fn run_resolved_command(
    category: &str,
    resolved: &ResolvedCommand,
    args: Vec<String>,
    cwd: &Path,
    timeout_ms: u64,
    hide_window: bool,
) -> CommandExecutionDto {
    let started_at_ms = now_ms();
    let display_command = display_command(&resolved.display_path, &args);
    let mut command = Command::new(&resolved.launcher_path);
    command.args(&resolved.base_args);
    command.args(&args);
    command.current_dir(cwd);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    if hide_window {
        hide_background_console(&mut command);
    }

    let child_spawn = command.spawn();
    let mut child = match child_spawn {
        Ok(child) => child,
        Err(error) => {
            return CommandExecutionDto {
                id: next_command_id(),
                category: category.to_string(),
                executable_path: resolved.display_path.clone(),
                display_command,
                args,
                cwd: path_string(cwd),
                started_at_ms,
                finished_at_ms: now_ms(),
                duration_ms: now_ms() - started_at_ms,
                exit_code: None,
                success: false,
                timed_out: false,
                stdout: String::new(),
                stderr: error.to_string(),
            };
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let stdout_handle = std::thread::spawn(move || read_pipe(stdout));
    let stderr_handle = std::thread::spawn(move || read_pipe(stderr));

    let timeout = Duration::from_millis(timeout_ms);
    let mut timed_out = false;
    let status = match child.wait_timeout(timeout) {
        Ok(Some(status)) => status,
        Ok(None) => {
            timed_out = true;
            let _ = child.kill();
            child.wait().unwrap_or_else(|_| failed_exit_status())
        }
        Err(_) => child.wait().unwrap_or_else(|_| failed_exit_status()),
    };

    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();
    let finished_at_ms = now_ms();

    CommandExecutionDto {
        id: next_command_id(),
        category: category.to_string(),
        executable_path: resolved.display_path.clone(),
        display_command,
        args,
        cwd: path_string(cwd),
        started_at_ms,
        finished_at_ms,
        duration_ms: finished_at_ms - started_at_ms,
        exit_code: status.code(),
        success: status.success() && !timed_out,
        timed_out,
        stdout,
        stderr: if timed_out && stderr.is_empty() {
            "命令执行超时。".to_string()
        } else {
            stderr
        },
    }
}

#[cfg(target_os = "windows")]
fn failed_exit_status() -> std::process::ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(1)
}

#[cfg(not(target_os = "windows"))]
fn failed_exit_status() -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(1)
}

fn read_pipe<R: Read>(pipe: Option<R>) -> String {
    let Some(mut pipe) = pipe else {
        return String::new();
    };

    let mut buffer = Vec::new();
    let _ = pipe.read_to_end(&mut buffer);
    String::from_utf8_lossy(&buffer).trim().to_string()
}

fn resolve_command(name: &str) -> Option<ResolvedCommand> {
    let command_path = resolve_command_path(name)?;
    let display_path = path_string(&command_path);

    match command_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("ps1"))
    {
        Some(true) => Some(ResolvedCommand {
            launcher_path: "powershell.exe".to_string(),
            display_path,
            base_args: vec![
                "-NoProfile".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-File".to_string(),
                path_string(&command_path),
            ],
        }),
        _ => Some(ResolvedCommand {
            launcher_path: path_string(&command_path),
            display_path,
            base_args: Vec::new(),
        }),
    }
}

fn resolve_command_path(name: &str) -> Option<PathBuf> {
    let paths = resolve_command_paths(name)?;

    #[cfg(target_os = "windows")]
    {
        prefer_command_path(&paths, &["exe", "cmd", "bat", "com"])
            .or_else(|| paths.first().cloned())
    }

    #[cfg(not(target_os = "windows"))]
    {
        paths.first().cloned()
    }
}

fn resolve_command_path_for_powershell(name: &str) -> Option<PathBuf> {
    let paths = resolve_command_paths(name)?;

    #[cfg(target_os = "windows")]
    {
        prefer_command_path(&paths, &["ps1", "cmd", "bat", "exe", "com"])
            .or_else(|| paths.first().cloned())
    }

    #[cfg(not(target_os = "windows"))]
    {
        paths.first().cloned()
    }
}

fn resolve_command_paths(name: &str) -> Option<Vec<PathBuf>> {
    let mut command = Command::new("where.exe");
    command.arg(name);
    hide_background_console(&mut command);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }

    let paths = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect::<Vec<_>>();

    if paths.is_empty() {
        None
    } else {
        Some(paths)
    }
}

#[cfg(target_os = "windows")]
fn prefer_command_path(paths: &[PathBuf], extensions: &[&str]) -> Option<PathBuf> {
    extensions.iter().find_map(|expected| {
        paths.iter().find_map(|path| {
            let extension = path.extension().and_then(|value| value.to_str())?;
            if extension.eq_ignore_ascii_case(expected) {
                Some(path.clone())
            } else {
                None
            }
        })
    })
}

fn powershell_escape(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(target_os = "windows")]
fn hide_background_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(target_os = "windows"))]
fn hide_background_console(_command: &mut Command) {}

fn load_logs(path: &Path) -> VecDeque<CommandExecutionDto> {
    let file_contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(_) => return VecDeque::new(),
    };

    serde_json::from_str::<Vec<CommandExecutionDto>>(&file_contents)
        .map(|logs| logs.into_iter().collect())
        .unwrap_or_default()
}

fn persist_logs(path: &Path, logs: &VecDeque<CommandExecutionDto>) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let payload = logs.iter().cloned().collect::<Vec<_>>();
    if let Ok(serialized) = serde_json::to_string_pretty(&payload) {
        let _ = fs::write(path, serialized);
    }
}

fn is_registry_event(event: &Event) -> bool {
    event.paths.iter().any(|path| {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        file_name == "registry.json"
            || file_name.starts_with("registry.json.bak.")
            || file_name.ends_with(".auth.json")
    })
}

fn seconds_to_ms(value: Option<i64>) -> Option<i64> {
    value.map(|seconds| seconds.saturating_mul(1000))
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn display_command(display_path: &str, args: &[String]) -> String {
    if args.is_empty() {
        display_path.to_string()
    } else {
        let rendered_args = args
            .iter()
            .map(|arg| {
                if arg.contains(' ') {
                    format!("\"{arg}\"")
                } else {
                    arg.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");

        format!("{display_path} {rendered_args}")
    }
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\")
}

fn next_command_id() -> String {
    let counter = COMMAND_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("cmd-{}-{counter}", now_ms())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "windows")]
    fn prefer_command_path_uses_spawnable_extension_first() {
        let paths = vec![
            PathBuf::from(r"C:\tools\codex-auth"),
            PathBuf::from(r"C:\tools\codex-auth.cmd"),
            PathBuf::from(r"C:\tools\codex-auth.ps1"),
        ];

        let selected = prefer_command_path(&paths, &["exe", "cmd", "bat", "com"]);
        assert_eq!(selected, Some(PathBuf::from(r"C:\tools\codex-auth.cmd")));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn prefer_command_path_can_prioritize_powershell_script() {
        let paths = vec![
            PathBuf::from(r"C:\tools\codex-auth.cmd"),
            PathBuf::from(r"C:\tools\codex-auth.ps1"),
        ];

        let selected = prefer_command_path(&paths, &["ps1", "cmd", "bat", "exe", "com"]);
        assert_eq!(selected, Some(PathBuf::from(r"C:\tools\codex-auth.ps1")));
    }

    #[test]
    fn powershell_escape_doubles_single_quotes() {
        assert_eq!(
            powershell_escape("C:\\Users\\o'connor\\tool.ps1"),
            "C:\\Users\\o''connor\\tool.ps1"
        );
    }

    #[test]
    fn parse_account_auth_statuses_marks_401_invalid() {
        let log = CommandExecutionDto {
            id: "cmd-test".to_string(),
            category: "refresh-registry".to_string(),
            executable_path: "codex-auth".to_string(),
            display_command: "codex-auth list --debug".to_string(),
            args: vec!["list".to_string(), "--debug".to_string()],
            cwd: "C:\\Users\\db\\.codex".to_string(),
            started_at_ms: 1,
            finished_at_ms: 2,
            duration_ms: 1,
            exit_code: Some(0),
            success: true,
            timed_out: false,
            stdout: "[debug] response usage: 18780858059@163.com status=401 result=http-response\n[debug] response usage: melissamontgomery6442@hotmail.com status=200 result=usage-windows".to_string(),
            stderr: String::new(),
        };

        let statuses = parse_account_auth_statuses(&log);

        assert_eq!(
            statuses
                .get("18780858059@163.com")
                .map(|value| value.state.as_str()),
            Some("invalid")
        );
        assert_eq!(
            statuses
                .get("melissamontgomery6442@hotmail.com")
                .map(|value| value.state.as_str()),
            Some("ok")
        );
    }

    #[test]
    fn write_account_alias_updates_target_and_rejects_duplicate() {
        let root = std::env::temp_dir().join(format!("codex-auth-gui-test-{}", now_ms()));
        let accounts_dir = root.join("accounts");
        let app_log_dir = root.join("app");
        fs::create_dir_all(&accounts_dir).expect("create test accounts dir");
        fs::create_dir_all(&app_log_dir).expect("create test app dir");
        let registry_path = accounts_dir.join("registry.json");
        fs::write(
            &registry_path,
            r#"{
  "schema_version": 1,
  "active_account_key": "key-1",
  "accounts": [
    {
      "account_key": "key-1",
      "email": "one@example.com",
      "alias": "one",
      "plan": "team"
    },
    {
      "account_key": "key-2",
      "email": "two@example.com",
      "plan": "plus"
    }
  ]
}"#,
        )
        .expect("write test registry");
        let dirs = InternalDirectories {
            codex_root: root.clone(),
            accounts_dir,
            sessions_dir: root.join("sessions"),
            registry_path: registry_path.clone(),
            app_log_dir: app_log_dir.clone(),
            app_log_file: app_log_dir.join("command-history.json"),
        };

        let backup_path = write_account_alias(&dirs, "key-2", "melissa-plus").expect("set alias");
        assert!(PathBuf::from(backup_path).exists());

        let parsed = serde_json::from_str::<RegistryFile>(
            &fs::read_to_string(&registry_path).expect("read updated registry"),
        )
        .expect("parse updated registry");
        assert_eq!(
            parsed
                .accounts
                .iter()
                .find(|account| account.account_key == "key-2")
                .and_then(|account| account.alias.as_deref()),
            Some("melissa-plus")
        );

        let duplicate = write_account_alias(&dirs, "key-1", "melissa-plus")
            .expect_err("duplicate alias should fail");
        assert!(duplicate.contains("already used"));

        fs::remove_dir_all(root).expect("remove test dir");
    }
}
