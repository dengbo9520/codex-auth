use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Deserializer, Serialize};
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
    performance: Arc<Mutex<VecDeque<PerformanceSpanDto>>>,
    pub(crate) dirs: InternalDirectories,
}

impl AppState {
    pub fn new(app: &AppHandle) -> Self {
        let dirs = InternalDirectories::detect(app);
        let logs = load_logs(&dirs.app_log_file);
        Self {
            logs: Arc::new(Mutex::new(logs)),
            performance: Arc::new(Mutex::new(VecDeque::new())),
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

    fn push_perf(&self, label: &str, duration_ms: i64, detail: impl Into<String>) {
        let mut guard = self
            .performance
            .lock()
            .expect("performance log lock poisoned");
        guard.push_front(PerformanceSpanDto {
            label: label.to_string(),
            duration_ms,
            detail: detail.into(),
            timestamp_ms: now_ms(),
        });

        while guard.len() > COMMAND_LOG_LIMIT {
            guard.pop_back();
        }
    }

    fn recent_performance(&self) -> Vec<PerformanceSpanDto> {
        self.performance
            .lock()
            .expect("performance log lock poisoned")
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
    verification_cache_file: PathBuf,
    gui_settings_file: PathBuf,
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
        let verification_cache_file = app_log_dir.join("verification-cache.json");
        let gui_settings_file = app_log_dir.join("gui-settings.json");

        Self {
            codex_root,
            accounts_dir,
            sessions_dir,
            registry_path,
            app_log_dir,
            app_log_file,
            verification_cache_file,
            gui_settings_file,
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
    pub performance: Vec<PerformanceSpanDto>,
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
pub struct PerformanceSpanDto {
    pub label: String,
    pub duration_ms: i64,
    pub detail: String,
    pub timestamp_ms: i64,
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
    pub login_expires_at_ms: Option<i64>,
    pub auth_last_refresh: Option<String>,
    pub auth_has_refresh_token: bool,
    pub subscription_active_until: Option<String>,
    pub subscription_last_checked: Option<String>,
    pub subscription_plan: Option<String>,
    pub verification_state: Option<String>,
    pub verification_label: Option<String>,
    pub verification_detail: Option<String>,
    pub verification_checked_at_ms: Option<i64>,
    pub usage_credits_has_credits: Option<bool>,
    pub usage_credits_unlimited: Option<bool>,
    pub usage_credits_balance: Option<String>,
    pub primary_usage: Option<UsageWindowDto>,
    pub weekly_usage: Option<UsageWindowDto>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationResultDto {
    pub command: CommandExecutionDto,
    pub registry: RegistrySnapshotDto,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountVerificationDto {
    pub command: CommandExecutionDto,
    pub registry: RegistrySnapshotDto,
    pub account_key: String,
    pub state: String,
    pub label: String,
    pub detail: String,
    pub switched_back: bool,
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

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
#[serde(default)]
struct GuiSettings {
    auto_switch_enabled: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
#[serde(default)]
struct RegistryStatusCheck {
    state: Option<String>,
    label: Option<String>,
    detail: Option<String>,
    checked_at_ms: Option<i64>,
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
    #[serde(default, deserialize_with = "deserialize_last_local_rollout")]
    last_local_rollout: Option<i64>,
}

fn deserialize_last_local_rollout<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match value {
        Some(serde_json::Value::Number(number)) => number.as_i64(),
        Some(serde_json::Value::Object(map)) => map
            .get("event_timestamp_ms")
            .and_then(serde_json::Value::as_i64),
        _ => None,
    })
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RegistryUsage {
    primary: Option<RegistryUsageWindow>,
    secondary: Option<RegistryUsageWindow>,
    credits: Option<RegistryUsageCredits>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RegistryUsageCredits {
    has_credits: Option<bool>,
    unlimited: Option<bool>,
    balance: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RegistryUsageWindow {
    used_percent: Option<i64>,
    window_minutes: Option<i64>,
    resets_at: Option<i64>,
}

#[derive(Default)]
struct AccountAuthMetadata {
    login_expires_at_ms: Option<i64>,
    auth_last_refresh: Option<String>,
    auth_has_refresh_token: bool,
    subscription_active_until: Option<String>,
    subscription_last_checked: Option<String>,
    subscription_plan: Option<String>,
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
        let total_started_at_ms = now_ms();
        let command = run_codex_auth_command(
            &state,
            &["list", "--debug"],
            "refresh-registry",
            REFRESH_TIMEOUT_MS,
        );
        state.push_perf(
            "refresh.cli",
            command.duration_ms,
            format!(
                "{} success={} timeout={}",
                command.display_command, command.success, command.timed_out
            ),
        );
        state.push_log(command.clone());
        let status_started_at_ms = now_ms();
        let account_auth_statuses = extract_account_auth_statuses(&state.recent_logs());
        state.push_perf(
            "refresh.status-parse",
            now_ms() - status_started_at_ms,
            format!("{} status entries", account_auth_statuses.len()),
        );
        let registry_started_at_ms = now_ms();
        let registry = read_registry_snapshot(&state.dirs, &account_auth_statuses);
        state.push_perf(
            "refresh.registry-read",
            now_ms() - registry_started_at_ms,
            format!("{} accounts", registry.accounts.len()),
        );
        state.push_perf(
            "refresh.total",
            now_ms() - total_started_at_ms,
            format!(
                "{} accounts, {} warnings",
                registry.accounts.len(),
                registry.warnings.len()
            ),
        );
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
pub fn get_local_registry_snapshot(state: State<'_, AppState>) -> MutationResultDto {
    let started_at_ms = now_ms();
    let account_auth_statuses = extract_account_auth_statuses(&state.recent_logs());
    let registry_started_at_ms = now_ms();
    let registry = read_registry_snapshot(&state.dirs, &account_auth_statuses);
    let registry_duration_ms = now_ms() - registry_started_at_ms;
    let duration_ms = now_ms() - started_at_ms;
    state.push_perf(
        "local-refresh.registry-read",
        registry_duration_ms,
        format!("{} accounts", registry.accounts.len()),
    );
    state.push_perf(
        "local-refresh.total",
        duration_ms,
        format!(
            "{} accounts, {} warnings",
            registry.accounts.len(),
            registry.warnings.len()
        ),
    );

    let command = CommandExecutionDto::synthetic(
        "local-refresh",
        "registry.json",
        "read local registry",
        vec!["local-refresh".to_string()],
        &state.dirs.codex_root,
        true,
        format!(
            "Read local registry in {duration_ms} ms; {} accounts.",
            registry.accounts.len()
        ),
        String::new(),
    );
    state.push_log(command.clone());
    MutationResultDto { command, registry }
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
                let active_summary = registry
                    .accounts
                    .iter()
                    .find(|account| account.active)
                    .map(account_display_summary)
                    .unwrap_or_else(|| "unknown active account".to_string());
                append_stdout_line(
                    &mut command.stdout,
                    format!(
                        "codex-auth activated a different account than requested; GUI follows actual active account: {active_summary}"
                    )
                    .as_str(),
                );
            }
        }
    }
    state.push_log(command.clone());
    MutationResultDto { command, registry }
}

#[tauri::command]
pub fn verify_account_state(
    account_key: String,
    state: State<'_, AppState>,
) -> AccountVerificationDto {
    let target_key = account_key.trim().to_string();
    let before_registry = read_registry_snapshot(&state.dirs, &HashMap::new());
    let previous_key = before_registry.active_account_key.clone();
    let previous_query = previous_key
        .as_ref()
        .and_then(|key| resolve_account_selector(&state.dirs, key, "switch").ok())
        .map(|selector| selector.query);

    let selector = resolve_account_selector(&state.dirs, &target_key, "switch");
    let mut command = match selector {
        Ok(selector) => run_query_command(&state, "verify-account", "switch", selector.query),
        Err(error) => CommandExecutionDto::synthetic(
            "verify-account",
            "codex-auth",
            "verify account",
            vec!["verify-account".to_string(), target_key.clone()],
            &state.dirs.codex_root,
            false,
            String::new(),
            error,
        ),
    };

    let after_switch_registry = read_registry_snapshot(&state.dirs, &HashMap::new());
    let active_matches = after_switch_registry.active_account_key.as_deref() == Some(&target_key);
    let mut switched_back = false;

    if previous_key.as_deref() != Some(&target_key) {
        if let Some(query) = previous_query {
            let restore_command = run_query_command(&state, "verify-restore", "switch", query);
            switched_back = restore_command.success;
            append_stdout_line(
                &mut command.stdout,
                format!(
                    "restore previous active account: success={}",
                    restore_command.success
                )
                .as_str(),
            );
            state.push_log(restore_command);
        }
    } else {
        switched_back = true;
    }

    let (state_label, label, detail) = if !command.success {
        (
            "verify_failed".to_string(),
            "验证失败".to_string(),
            command.stderr.clone(),
        )
    } else if active_matches {
        (
            "switchable".to_string(),
            "可切换".to_string(),
            "codex-auth 可以切到该账号/空间；不能仅凭 usage API 401 判定停用。".to_string(),
        )
    } else {
        let active_summary = after_switch_registry
            .accounts
            .iter()
            .find(|account| account.active)
            .map(|account| {
                format!(
                    "{} / {}",
                    account.email,
                    account
                        .account_name
                        .clone()
                        .unwrap_or_else(|| account.plan.clone())
                )
            })
            .unwrap_or_else(|| "无激活账号".to_string());
        (
            "suspected_disabled".to_string(),
            "疑似停用".to_string(),
            format!("请求切换后实际激活为 {active_summary}，目标未成为 active。"),
        )
    };

    append_stdout_line(
        &mut command.stdout,
        format!("verification: {label}; switched_back={switched_back}").as_str(),
    );

    match write_account_verification(&state.dirs, &target_key, &state_label, &label, &detail) {
        Ok(cache_path) => append_stdout_line(
            &mut command.stdout,
            format!("cached verification: {cache_path}").as_str(),
        ),
        Err(error) => append_stdout_line(
            &mut command.stdout,
            format!("failed to cache verification: {error}").as_str(),
        ),
    }

    state.push_log(command.clone());

    AccountVerificationDto {
        command,
        registry: read_registry_snapshot(&state.dirs, &HashMap::new()),
        account_key: target_key,
        state: state_label,
        label,
        detail,
        switched_back,
    }
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
    let command = match write_auto_switch_enabled(&state.dirs, enabled) {
        Ok(settings_path) => CommandExecutionDto::synthetic(
            "config-auto",
            "gui-settings.json",
            "set GUI auto-switch",
            vec![
                "config".to_string(),
                "auto".to_string(),
                if enabled { "enable" } else { "disable" }.to_string(),
            ],
            &state.dirs.codex_root,
            true,
            format!(
                "GUI auto-switch set to {}. Settings: {}",
                if enabled { "enabled" } else { "disabled" },
                settings_path
            ),
            String::new(),
        ),
        Err(error) => CommandExecutionDto::synthetic(
            "config-auto",
            "gui-settings.json",
            "set GUI auto-switch",
            vec![
                "config".to_string(),
                "auto".to_string(),
                if enabled { "enable" } else { "disable" }.to_string(),
            ],
            &state.dirs.codex_root,
            false,
            String::new(),
            error,
        ),
    };
    let registry = read_registry_snapshot(&state.dirs, &HashMap::new());
    state.push_log(command.clone());
    MutationResultDto { command, registry }
}

#[tauri::command]
pub fn set_usage_api_mode(enabled: bool, state: State<'_, AppState>) -> MutationResultDto {
    let command = match write_usage_api_enabled(&state.dirs, enabled) {
        Ok(backup_path) => CommandExecutionDto::synthetic(
            "config-api",
            "registry.json",
            "set usage API mode",
            vec![
                "config".to_string(),
                "api".to_string(),
                if enabled { "enable" } else { "disable" }.to_string(),
            ],
            &state.dirs.codex_root,
            true,
            format!(
                "Usage API mode set to {}. Backup: {}",
                if enabled { "enabled" } else { "disabled" },
                backup_path
            ),
            String::new(),
        ),
        Err(error) => CommandExecutionDto::synthetic(
            "config-api",
            "registry.json",
            "set usage API mode",
            vec![
                "config".to_string(),
                "api".to_string(),
                if enabled { "enable" } else { "disable" }.to_string(),
            ],
            &state.dirs.codex_root,
            false,
            String::new(),
            error,
        ),
    };
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
                "已打开外部 PowerShell 登录窗口；授权网页由 codex-auth 打开。".to_string()
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

& '{target}' login 2>&1 | ForEach-Object {{
    if ($_ -is [System.Management.Automation.ErrorRecord]) {{
        $line = $_.Exception.Message.Trim()
    }} else {{
        $line = $_.ToString().Trim()
    }}
    Write-Host $line
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
        performance: state.recent_performance(),
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

    let email_counts = account_email_counts(&parsed.accounts);
    let active_account_key = parsed.active_account_key.clone();
    let gui_status_checks = load_account_verifications(&dirs.verification_cache_file);
    let gui_settings = load_gui_settings(&dirs.gui_settings_file);
    let mut accounts = parsed
        .accounts
        .into_iter()
        .filter(|account| account_auth_file_path(dirs, &account.account_key).exists())
        .map(|account| {
            let verification = gui_status_checks.get(&account.account_key);
            registry_account_to_dto(
                dirs,
                account,
                active_account_key.as_deref(),
                account_auth_statuses,
                &email_counts,
                verification,
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
        auto_switch_enabled: gui_settings
            .auto_switch_enabled
            .unwrap_or(parsed.auto_switch.enabled),
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
    dirs: &InternalDirectories,
    account: RegistryAccount,
    active_account_key: Option<&str>,
    account_auth_statuses: &HashMap<String, AccountAuthStatus>,
    email_counts: &HashMap<String, usize>,
    verification: Option<&RegistryStatusCheck>,
) -> AccountDto {
    let auth_status = find_account_auth_status(&account, account_auth_statuses, email_counts);
    let auth_metadata = read_account_auth_metadata(dirs, &account.account_key);
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
        login_expires_at_ms: auth_metadata.login_expires_at_ms,
        auth_last_refresh: auth_metadata.auth_last_refresh,
        auth_has_refresh_token: auth_metadata.auth_has_refresh_token,
        subscription_active_until: auth_metadata.subscription_active_until,
        subscription_last_checked: auth_metadata.subscription_last_checked,
        subscription_plan: auth_metadata.subscription_plan,
        verification_state: verification.and_then(|check| check.state.clone()),
        verification_label: verification.and_then(|check| check.label.clone()),
        verification_detail: verification.and_then(|check| check.detail.clone()),
        verification_checked_at_ms: verification.and_then(|check| check.checked_at_ms),
        usage_credits_has_credits: account
            .last_usage
            .as_ref()
            .and_then(|usage| usage.credits.as_ref())
            .and_then(|credits| credits.has_credits),
        usage_credits_unlimited: account
            .last_usage
            .as_ref()
            .and_then(|usage| usage.credits.as_ref())
            .and_then(|credits| credits.unlimited),
        usage_credits_balance: account
            .last_usage
            .as_ref()
            .and_then(|usage| usage.credits.as_ref())
            .and_then(|credits| credits.balance.as_ref())
            .and_then(json_value_to_display_string),
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

fn json_value_to_display_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn account_email_counts(accounts: &[RegistryAccount]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for account in accounts {
        *counts.entry(account.email.to_lowercase()).or_insert(0) += 1;
    }
    counts
}

fn find_account_auth_status<'a>(
    account: &RegistryAccount,
    account_auth_statuses: &'a HashMap<String, AccountAuthStatus>,
    email_counts: &HashMap<String, usize>,
) -> Option<&'a AccountAuthStatus> {
    for key in account_status_lookup_keys(account) {
        if let Some(status) = account_auth_statuses.get(&key) {
            return Some(status);
        }
    }

    let email_key = account.email.to_lowercase();
    if email_counts.get(&email_key).copied().unwrap_or_default() == 1 {
        return account_auth_statuses.get(&email_key);
    }

    None
}

fn account_status_lookup_keys(account: &RegistryAccount) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(alias) = trimmed_optional_str(account.alias.as_deref()) {
        keys.push(normalize_account_status_key(
            format!("{} | {}", account.email, alias).as_str(),
        ));
    }
    if let Some(account_name) = trimmed_optional_str(account.account_name.as_deref()) {
        keys.push(normalize_account_status_key(
            format!("{} | {}", account.email, account_name).as_str(),
        ));
    }
    keys
}

fn read_account_auth_metadata(
    dirs: &InternalDirectories,
    account_key: &str,
) -> AccountAuthMetadata {
    let path = account_auth_file_path(dirs, account_key);
    let file_contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(_) => return AccountAuthMetadata::default(),
    };
    let parsed = match serde_json::from_str::<serde_json::Value>(&file_contents) {
        Ok(value) => value,
        Err(_) => return AccountAuthMetadata::default(),
    };
    let tokens = parsed.get("tokens").unwrap_or(&serde_json::Value::Null);
    let id_token = tokens
        .get("id_token")
        .and_then(serde_json::Value::as_str)
        .or_else(|| parsed.get("id_token").and_then(serde_json::Value::as_str));
    let claims = id_token.and_then(parse_jwt_payload);
    let openai_auth = claims
        .as_ref()
        .and_then(|claims| claims.get("https://api.openai.com/auth"));

    AccountAuthMetadata {
        login_expires_at_ms: claims
            .as_ref()
            .and_then(|claims| claims.get("exp"))
            .and_then(serde_json::Value::as_i64)
            .and_then(|seconds| seconds_to_ms(Some(seconds))),
        auth_last_refresh: parsed
            .get("last_refresh")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        auth_has_refresh_token: tokens
            .get("refresh_token")
            .and_then(serde_json::Value::as_str)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false),
        subscription_active_until: openai_auth
            .and_then(|auth| auth.get("chatgpt_subscription_active_until"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        subscription_last_checked: openai_auth
            .and_then(|auth| auth.get("chatgpt_subscription_last_checked"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        subscription_plan: openai_auth
            .and_then(|auth| auth.get("chatgpt_plan_type"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
    }
}

fn account_auth_file_path(dirs: &InternalDirectories, account_key: &str) -> PathBuf {
    dirs.accounts_dir.join(format!(
        "{}.auth.json",
        base64url_encode(account_key.as_bytes())
    ))
}

fn parse_jwt_payload(token: &str) -> Option<serde_json::Value> {
    let payload = token.split('.').nth(1)?;
    let decoded = base64url_decode(payload)?;
    serde_json::from_slice::<serde_json::Value>(&decoded).ok()
}

fn base64url_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut output = String::new();
    let mut index = 0;
    while index < bytes.len() {
        let b0 = bytes[index];
        let b1 = bytes.get(index + 1).copied().unwrap_or(0);
        let b2 = bytes.get(index + 2).copied().unwrap_or(0);
        output.push(ALPHABET[(b0 >> 2) as usize] as char);
        output.push(ALPHABET[(((b0 & 0b0000_0011) << 4) | (b1 >> 4)) as usize] as char);
        if index + 1 < bytes.len() {
            output.push(ALPHABET[(((b1 & 0b0000_1111) << 2) | (b2 >> 6)) as usize] as char);
        }
        if index + 2 < bytes.len() {
            output.push(ALPHABET[(b2 & 0b0011_1111) as usize] as char);
        }
        index += 3;
    }
    output
}

fn base64url_decode(value: &str) -> Option<Vec<u8>> {
    let mut buffer = 0u32;
    let mut bits = 0u8;
    let mut output = Vec::new();

    for byte in value.bytes() {
        if byte == b'=' {
            break;
        }
        let decoded = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(decoded);
        bits += 6;
        while bits >= 8 {
            bits -= 8;
            output.push(((buffer >> bits) & 0xff) as u8);
        }
    }

    Some(output)
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
        let Some((display_name, tail)) = payload.split_once(" status=") else {
            continue;
        };

        let status_key = normalize_account_status_key(display_name);
        if status_key.is_empty() {
            continue;
        }

        let (status_code, detail) = parse_debug_status_tail(tail);
        let state = match status_code {
            Some(200) => "ok".to_string(),
            Some(401) => "usage_unauthorized".to_string(),
            Some(402) => "disabled".to_string(),
            Some(_) => "warning".to_string(),
            None => "unknown".to_string(),
        };

        statuses.insert(
            status_key,
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

fn normalize_account_status_key(value: &str) -> String {
    value
        .split('|')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" | ")
        .to_lowercase()
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

fn write_auto_switch_enabled(dirs: &InternalDirectories, enabled: bool) -> Result<String, String> {
    let mut settings = load_gui_settings(&dirs.gui_settings_file);
    settings.auto_switch_enabled = Some(enabled);
    fs::create_dir_all(&dirs.app_log_dir).map_err(|error| {
        format!(
            "Failed to create GUI settings dir: {} ({error})",
            dirs.app_log_dir.display()
        )
    })?;
    let serialized = serde_json::to_string_pretty(&settings)
        .map_err(|error| format!("Failed to serialize GUI settings: {error}"))?;
    fs::write(&dirs.gui_settings_file, format!("{serialized}\n")).map_err(|error| {
        format!(
            "Failed to write GUI settings: {} ({error})",
            dirs.gui_settings_file.display()
        )
    })?;
    Ok(path_string(&dirs.gui_settings_file))
}

fn write_usage_api_enabled(dirs: &InternalDirectories, enabled: bool) -> Result<String, String> {
    write_registry_bools(
        dirs,
        &[
            (&["api", "usage"][..], enabled),
            (&["api", "account"][..], enabled),
        ],
        "usage API",
        "api",
    )
}

fn write_registry_bools(
    dirs: &InternalDirectories,
    updates: &[(&[&str], bool)],
    label: &str,
    backup_label: &str,
) -> Result<String, String> {
    let mut parsed = read_registry_json(dirs, label)?;
    for (path, enabled) in updates {
        set_json_bool_path(&mut parsed, path, *enabled)?;
    }
    let backup_path = backup_registry(dirs, backup_label)?;
    write_registry_json(dirs, &parsed, label)?;
    Ok(path_string(&backup_path))
}

fn read_registry_json(
    dirs: &InternalDirectories,
    label: &str,
) -> Result<serde_json::Value, String> {
    let file_contents = fs::read_to_string(&dirs.registry_path).map_err(|error| {
        format!(
            "Failed to read registry before setting {label}: {} ({error})",
            dirs.registry_path.display()
        )
    })?;
    serde_json::from_str::<serde_json::Value>(&file_contents).map_err(|error| {
        format!(
            "Failed to parse registry before setting {label}: {} ({error})",
            dirs.registry_path.display()
        )
    })
}

fn set_json_bool_path(
    root: &mut serde_json::Value,
    path: &[&str],
    enabled: bool,
) -> Result<(), String> {
    let Some((leaf, parents)) = path.split_last() else {
        return Err("registry bool path is empty.".to_string());
    };
    let mut current = root
        .as_object_mut()
        .ok_or_else(|| "registry root is not a JSON object.".to_string())?;
    for segment in parents {
        let value = current
            .entry((*segment).to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        current = value
            .as_object_mut()
            .ok_or_else(|| format!("registry field '{segment}' is not a JSON object."))?;
    }
    current.insert((*leaf).to_string(), serde_json::Value::Bool(enabled));
    Ok(())
}

fn write_account_verification(
    dirs: &InternalDirectories,
    account_key: &str,
    state: &str,
    label: &str,
    detail: &str,
) -> Result<String, String> {
    let mut checks = load_account_verifications(&dirs.verification_cache_file);
    checks.insert(
        account_key.to_string(),
        RegistryStatusCheck {
            state: Some(state.to_string()),
            label: Some(label.to_string()),
            detail: Some(detail.to_string()),
            checked_at_ms: Some(now_ms()),
        },
    );

    fs::create_dir_all(&dirs.app_log_dir).map_err(|error| {
        format!(
            "Failed to create verification cache dir: {} ({error})",
            dirs.app_log_dir.display()
        )
    })?;
    let serialized = serde_json::to_string_pretty(&checks)
        .map_err(|error| format!("Failed to serialize verification cache: {error}"))?;
    fs::write(&dirs.verification_cache_file, format!("{serialized}\n")).map_err(|error| {
        format!(
            "Failed to write verification cache: {} ({error})",
            dirs.verification_cache_file.display()
        )
    })?;
    Ok(path_string(&dirs.verification_cache_file))
}

fn load_account_verifications(path: &Path) -> HashMap<String, RegistryStatusCheck> {
    fs::read_to_string(path)
        .ok()
        .and_then(|contents| {
            serde_json::from_str::<HashMap<String, RegistryStatusCheck>>(&contents).ok()
        })
        .unwrap_or_default()
}

fn load_gui_settings(path: &Path) -> GuiSettings {
    fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str::<GuiSettings>(&contents).ok())
        .unwrap_or_default()
}

fn backup_registry(dirs: &InternalDirectories, label: &str) -> Result<PathBuf, String> {
    let backup_path = dirs
        .registry_path
        .with_file_name(format!("registry.json.bak.{label}.{}", now_ms()));
    fs::copy(&dirs.registry_path, &backup_path).map_err(|error| {
        format!(
            "Failed to backup registry before setting {label}: {} ({error})",
            backup_path.display()
        )
    })?;
    Ok(backup_path)
}

fn write_registry_json(
    dirs: &InternalDirectories,
    parsed: &serde_json::Value,
    label: &str,
) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(parsed)
        .map_err(|error| format!("Failed to serialize registry after setting {label}: {error}"))?;
    fs::write(&dirs.registry_path, format!("{serialized}\n")).map_err(|error| {
        format!(
            "Failed to write registry after setting {label}: {} ({error})",
            dirs.registry_path.display()
        )
    })
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

    let cleanup_failed_after_output = is_auto_task_cleanup_failure(category, &stdout, &stderr);

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
        success: (status.success() || cleanup_failed_after_output) && !timed_out,
        timed_out,
        stdout,
        stderr: if timed_out && stderr.is_empty() {
            "命令执行超时。".to_string()
        } else {
            stderr
        },
    }
}

fn is_auto_task_cleanup_failure(category: &str, stdout: &str, stderr: &str) -> bool {
    category == "refresh-registry"
        && !stdout.trim().is_empty()
        && stderr.contains("Unregister-ScheduledTask")
        && (stderr.contains("Access is denied") || stderr.contains("拒绝访问"))
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

fn trimmed_optional_str(value: Option<&str>) -> Option<&str> {
    let trimmed = value?.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn append_stdout_line(stdout: &mut String, line: &str) {
    if !stdout.is_empty() {
        stdout.push('\n');
    }
    stdout.push_str(line);
}

fn account_display_summary(account: &AccountDto) -> String {
    let name = if !account.alias.trim().is_empty() {
        Some(account.alias.as_str())
    } else {
        trimmed_optional_str(account.account_name.as_deref())
    };

    match name {
        Some(name) => format!("{} | {}", account.email, name),
        None => account.email.clone(),
    }
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
    fn registry_account_accepts_rollout_object() {
        let account: RegistryAccount = serde_json::from_str(
            r#"{
                "account_key": "key",
                "email": "user@example.com",
                "last_local_rollout": {
                    "path": "C:\\Users\\db\\.codex\\sessions\\rollout.jsonl",
                    "event_timestamp_ms": 1777101637434
                }
            }"#,
        )
        .expect("parse registry account");

        assert_eq!(account.last_local_rollout, Some(1777101637434));
    }

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
    fn parse_account_auth_statuses_marks_401_and_402_states() {
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
            stdout: "[debug] response usage: 18780858059@163.com status=401 result=http-response\n[debug] response usage: melissamontgomery6442@hotmail.com status=200 result=usage-windows\n[debug] response usage: tracycox8658@hotmail.com | LizzieDibberttm status=401 result=http-response\n[debug] response usage: banned@example.com | DeadSpace status=402 result=http-response".to_string(),
            stderr: String::new(),
        };

        let statuses = parse_account_auth_statuses(&log);

        assert_eq!(
            statuses
                .get("18780858059@163.com")
                .map(|value| value.state.as_str()),
            Some("usage_unauthorized")
        );
        assert_eq!(
            statuses
                .get("melissamontgomery6442@hotmail.com")
                .map(|value| value.state.as_str()),
            Some("ok")
        );
        assert_eq!(
            statuses
                .get("tracycox8658@hotmail.com | lizziedibberttm")
                .map(|value| value.state.as_str()),
            Some("usage_unauthorized")
        );
        assert_eq!(
            statuses
                .get("banned@example.com | deadspace")
                .map(|value| value.state.as_str()),
            Some("disabled")
        );
    }

    #[test]
    fn auth_status_lookup_uses_workspace_name_for_duplicate_email() {
        let mut statuses = HashMap::new();
        statuses.insert(
            "tracycox8658@hotmail.com | lizziedibberttm".to_string(),
            AccountAuthStatus {
                state: "invalid".to_string(),
                status_code: Some(401),
                detail: Some("HTTP 401 (http-response)".to_string()),
                checked_at_ms: 1,
            },
        );

        let lizzie = RegistryAccount {
            account_key: "key-lizzie".to_string(),
            chatgpt_account_id: Some("account-lizzie".to_string()),
            chatgpt_user_id: Some("user-tracy".to_string()),
            email: "tracycox8658@hotmail.com".to_string(),
            alias: None,
            account_name: Some("LizzieDibberttm".to_string()),
            plan: Some("team".to_string()),
            auth_mode: Some("chatgpt".to_string()),
            created_at: None,
            last_used_at: None,
            last_usage: None,
            last_usage_at: None,
            last_local_rollout: None,
        };
        let tingsky = RegistryAccount {
            account_key: "key-tingsky".to_string(),
            chatgpt_account_id: Some("account-tingsky".to_string()),
            chatgpt_user_id: Some("user-tracy".to_string()),
            email: "tracycox8658@hotmail.com".to_string(),
            alias: None,
            account_name: Some("Tingsky11".to_string()),
            plan: Some("team".to_string()),
            auth_mode: Some("chatgpt".to_string()),
            created_at: None,
            last_used_at: None,
            last_usage: None,
            last_usage_at: None,
            last_local_rollout: None,
        };
        let counts = account_email_counts(&[lizzie, tingsky]);

        let lizzie = RegistryAccount {
            account_key: "key-lizzie".to_string(),
            chatgpt_account_id: Some("account-lizzie".to_string()),
            chatgpt_user_id: Some("user-tracy".to_string()),
            email: "tracycox8658@hotmail.com".to_string(),
            alias: None,
            account_name: Some("LizzieDibberttm".to_string()),
            plan: Some("team".to_string()),
            auth_mode: Some("chatgpt".to_string()),
            created_at: None,
            last_used_at: None,
            last_usage: None,
            last_usage_at: None,
            last_local_rollout: None,
        };
        let tingsky = RegistryAccount {
            account_key: "key-tingsky".to_string(),
            chatgpt_account_id: Some("account-tingsky".to_string()),
            chatgpt_user_id: Some("user-tracy".to_string()),
            email: "tracycox8658@hotmail.com".to_string(),
            alias: None,
            account_name: Some("Tingsky11".to_string()),
            plan: Some("team".to_string()),
            auth_mode: Some("chatgpt".to_string()),
            created_at: None,
            last_used_at: None,
            last_usage: None,
            last_usage_at: None,
            last_local_rollout: None,
        };

        assert_eq!(
            find_account_auth_status(&lizzie, &statuses, &counts)
                .map(|status| status.state.as_str()),
            Some("invalid")
        );
        assert!(find_account_auth_status(&tingsky, &statuses, &counts).is_none());
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
            verification_cache_file: app_log_dir.join("verification-cache.json"),
            gui_settings_file: app_log_dir.join("gui-settings.json"),
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
