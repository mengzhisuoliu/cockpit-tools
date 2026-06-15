use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::modules;
use chrono::{TimeZone, Utc};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use serde_json::{json, Value as JsonValue};

const DEFAULT_INSTANCE_ID: &str = "__default__";
const DEFAULT_INSTANCE_NAME: &str = "默认实例";
const DEFAULT_PROVIDER_ID: &str = "openai";
const STATE_DB_FILE: &str = "state_5.sqlite";
const SQLITE_DIR_NAME: &str = "sqlite";
const PREFERRED_SQLITE_DB_FILE: &str = "codex-dev.db";
const CONFIG_FILE_NAME: &str = "config.toml";
const SESSION_INDEX_FILE: &str = "session_index.jsonl";
const SESSION_DIRS: [&str; 2] = ["sessions", "archived_sessions"];
const SESSION_VISIBILITY_REPAIR_BACKUP_PREFIX: &str = "backup-";
const SESSION_VISIBILITY_REPAIR_BACKUP_SUFFIX: &str = "-session-visibility-repair";
const MAX_SESSION_VISIBILITY_REPAIR_BACKUPS: usize = 1;
const SESSION_INDEX_ACTIVITY_DRIFT_MS: i128 = 3_600_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionVisibilityRepairItem {
    pub instance_id: String,
    pub instance_name: String,
    pub target_provider: String,
    pub changed_rollout_file_count: usize,
    pub updated_sqlite_row_count: usize,
    pub updated_sqlite_timestamp_row_count: usize,
    pub added_session_index_entry_count: usize,
    pub updated_session_index_entry_count: usize,
    pub skipped_sqlite_file: bool,
    pub metadata_rebuild_failed: bool,
    pub backup_dir: Option<String>,
    pub running: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionVisibilityRepairSummary {
    pub instance_count: usize,
    pub mutated_instance_count: usize,
    pub changed_rollout_file_count: usize,
    pub updated_sqlite_row_count: usize,
    pub updated_sqlite_timestamp_row_count: usize,
    pub added_session_index_entry_count: usize,
    pub updated_session_index_entry_count: usize,
    pub skipped_sqlite_file_count: usize,
    pub metadata_rebuild_failed_count: usize,
    pub items: Vec<CodexSessionVisibilityRepairItem>,
    pub backup_dirs: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone)]
struct CodexSyncInstance {
    id: String,
    name: String,
    data_dir: PathBuf,
    last_pid: Option<u32>,
}

#[derive(Debug, Clone)]
struct RolloutProviderChange {
    relative_path: PathBuf,
    absolute_path: PathBuf,
    updated_content: Option<String>,
    target_modified_at: Option<SystemTime>,
}

#[derive(Debug, Clone, Default)]
struct RolloutThreadFacts {
    user_event_thread_ids: HashSet<String>,
    cwd_by_thread_id: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy)]
struct SqliteProviderScan {
    rows_to_update: usize,
    skipped_unusable_database: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct SessionIndexRepairScan {
    entries_to_add: usize,
    entries_to_update: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct SessionIndexReconcileResult {
    added_entries: usize,
    updated_entries: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct RepairSingleInstanceResult {
    updated_sqlite_rows: usize,
    updated_sqlite_timestamp_rows: usize,
    added_session_index_entries: usize,
    updated_session_index_entries: usize,
}

#[derive(Debug, Clone)]
struct SqliteTimestampUpdate {
    id: String,
    updated_at_seconds: i64,
    updated_at_ms: i64,
}

#[derive(Debug, Clone, Default)]
struct SqliteTimestampRepairPlan {
    updates: Vec<SqliteTimestampUpdate>,
    has_updated_at: bool,
    has_updated_at_ms: bool,
}

#[derive(Debug, Clone, Copy)]
struct ThreadsTableColumns {
    model_provider: bool,
    has_user_event: bool,
    first_user_message: bool,
    thread_source: bool,
    cwd: bool,
}

#[derive(Debug, Clone)]
struct SqliteThreadIndexRow {
    id: String,
    title: String,
    updated_at: Option<i64>,
    updated_at_ms: Option<i64>,
    rollout_path: Option<String>,
}

pub fn repair_session_visibility_across_instances(
) -> Result<CodexSessionVisibilityRepairSummary, String> {
    let instances = collect_instances()?;
    let process_entries = modules::process::collect_codex_process_entries();
    let mut items = Vec::with_capacity(instances.len());
    let mut backup_dirs = Vec::new();
    let mut mutated_instance_count = 0usize;
    let mut changed_rollout_file_count = 0usize;
    let mut updated_sqlite_row_count = 0usize;
    let mut updated_sqlite_timestamp_row_count = 0usize;
    let mut added_session_index_entry_count = 0usize;
    let mut updated_session_index_entry_count = 0usize;
    let mut skipped_sqlite_file_count = 0usize;
    let mut metadata_rebuild_failed_count = 0usize;
    let mut mutated_running_instance_count = 0usize;

    for instance in &instances {
        let running = is_instance_running(instance, &process_entries);
        let target_provider = read_target_provider(&instance.data_dir)?;
        let rollout_changes =
            collect_rollout_provider_changes(&instance.data_dir, &target_provider)?;
        let sqlite_scan = count_sqlite_rows_to_update(&instance.data_dir, &target_provider)?;
        let sqlite_rows_to_update = sqlite_scan.rows_to_update;
        let sqlite_timestamp_rows_to_update =
            count_sqlite_thread_timestamps_to_update(&instance.data_dir)?;
        let session_index_scan = count_session_index_entries_to_repair(&instance.data_dir)?;
        if sqlite_scan.skipped_unusable_database {
            skipped_sqlite_file_count += 1;
        }

        if rollout_changes.is_empty()
            && sqlite_rows_to_update == 0
            && sqlite_timestamp_rows_to_update == 0
            && session_index_scan.entries_to_add == 0
            && session_index_scan.entries_to_update == 0
        {
            items.push(CodexSessionVisibilityRepairItem {
                instance_id: instance.id.clone(),
                instance_name: instance.name.clone(),
                target_provider,
                changed_rollout_file_count: 0,
                updated_sqlite_row_count: 0,
                updated_sqlite_timestamp_row_count: 0,
                added_session_index_entry_count: 0,
                updated_session_index_entry_count: 0,
                skipped_sqlite_file: sqlite_scan.skipped_unusable_database,
                metadata_rebuild_failed: false,
                backup_dir: None,
                running,
            });
            continue;
        }

        let backup_dir = backup_instance_files(
            &instance.data_dir,
            &rollout_changes,
            sqlite_rows_to_update > 0 || sqlite_timestamp_rows_to_update > 0,
            session_index_scan.entries_to_add > 0 || session_index_scan.entries_to_update > 0,
            &instance.id,
            &target_provider,
        )?;
        let backup_dir_string = backup_dir.to_string_lossy().to_string();

        let repaired = repair_single_instance(
            &instance.data_dir,
            &target_provider,
            &rollout_changes,
            sqlite_rows_to_update > 0,
            sqlite_timestamp_rows_to_update > 0,
            session_index_scan.entries_to_add > 0 || session_index_scan.entries_to_update > 0,
        );
        let repaired = match repaired {
            Ok(value) => value,
            Err(error) => {
                let restore_result = restore_instance_files_from_backup(
                    &instance.data_dir,
                    &backup_dir,
                    sqlite_rows_to_update > 0 || sqlite_timestamp_rows_to_update > 0,
                );
                if let Err(restore_error) = restore_result {
                    return Err(format!(
                        "修复实例历史会话可见性失败 ({}): {}；自动回滚也失败: {}；备份目录: {}",
                        instance.name,
                        error,
                        restore_error,
                        backup_dir.display()
                    ));
                }
                return Err(format!(
                    "修复实例历史会话可见性失败 ({}): {}；已自动回滚，备份目录: {}",
                    instance.name,
                    error,
                    backup_dir.display()
                ));
            }
        };

        let instance_mutated = !rollout_changes.is_empty()
            || repaired.updated_sqlite_rows > 0
            || repaired.updated_sqlite_timestamp_rows > 0
            || repaired.added_session_index_entries > 0
            || repaired.updated_session_index_entries > 0;
        let mut metadata_rebuild_failed = false;
        if instance_mutated && !try_rebuild_thread_metadata(instance) {
            metadata_rebuild_failed = true;
            metadata_rebuild_failed_count += 1;
        }

        if instance_mutated {
            mutated_instance_count += 1;
            if running {
                mutated_running_instance_count += 1;
            }
        }
        changed_rollout_file_count += rollout_changes.len();
        updated_sqlite_row_count += repaired.updated_sqlite_rows;
        updated_sqlite_timestamp_row_count += repaired.updated_sqlite_timestamp_rows;
        added_session_index_entry_count += repaired.added_session_index_entries;
        updated_session_index_entry_count += repaired.updated_session_index_entries;
        backup_dirs.push(backup_dir_string.clone());
        items.push(CodexSessionVisibilityRepairItem {
            instance_id: instance.id.clone(),
            instance_name: instance.name.clone(),
            target_provider,
            changed_rollout_file_count: rollout_changes.len(),
            updated_sqlite_row_count: repaired.updated_sqlite_rows,
            updated_sqlite_timestamp_row_count: repaired.updated_sqlite_timestamp_rows,
            added_session_index_entry_count: repaired.added_session_index_entries,
            updated_session_index_entry_count: repaired.updated_session_index_entries,
            skipped_sqlite_file: sqlite_scan.skipped_unusable_database,
            metadata_rebuild_failed,
            backup_dir: Some(backup_dir_string),
            running,
        });
    }

    prune_session_visibility_repair_backups(&instances);

    let message = build_summary_message(
        mutated_instance_count,
        changed_rollout_file_count,
        updated_sqlite_row_count,
        updated_sqlite_timestamp_row_count,
        added_session_index_entry_count,
        updated_session_index_entry_count,
        mutated_running_instance_count,
        skipped_sqlite_file_count,
        metadata_rebuild_failed_count,
    );

    Ok(CodexSessionVisibilityRepairSummary {
        instance_count: instances.len(),
        mutated_instance_count,
        changed_rollout_file_count,
        updated_sqlite_row_count,
        updated_sqlite_timestamp_row_count,
        added_session_index_entry_count,
        updated_session_index_entry_count,
        skipped_sqlite_file_count,
        metadata_rebuild_failed_count,
        items,
        backup_dirs,
        message,
    })
}

pub fn read_history_visibility_provider_for_dir(data_dir: &Path) -> Result<String, String> {
    read_target_provider(data_dir)
}

fn repair_single_instance(
    data_dir: &Path,
    target_provider: &str,
    rollout_changes: &[RolloutProviderChange],
    update_sqlite: bool,
    update_sqlite_timestamps: bool,
    reconcile_session_index: bool,
) -> Result<RepairSingleInstanceResult, String> {
    let sqlite_rows_updated = if update_sqlite {
        update_sqlite_provider(data_dir, target_provider)?
    } else {
        0
    };
    for change in rollout_changes {
        rewrite_rollout_provider(change)?;
    }
    let sqlite_timestamp_rows_updated = if update_sqlite_timestamps {
        repair_sqlite_thread_timestamps(data_dir)?
    } else {
        0
    };
    let session_index_result = if reconcile_session_index {
        reconcile_session_index_from_sqlite(data_dir)?
    } else {
        SessionIndexReconcileResult::default()
    };
    Ok(RepairSingleInstanceResult {
        updated_sqlite_rows: sqlite_rows_updated,
        updated_sqlite_timestamp_rows: sqlite_timestamp_rows_updated,
        added_session_index_entries: session_index_result.added_entries,
        updated_session_index_entries: session_index_result.updated_entries,
    })
}

fn build_summary_message(
    mutated_instance_count: usize,
    changed_rollout_file_count: usize,
    updated_sqlite_row_count: usize,
    updated_sqlite_timestamp_row_count: usize,
    added_session_index_entry_count: usize,
    updated_session_index_entry_count: usize,
    mutated_running_instance_count: usize,
    _skipped_sqlite_file_count: usize,
    metadata_rebuild_failed_count: usize,
) -> String {
    if mutated_instance_count == 0 {
        return "所有 Codex 实例的 rollout、session_index 与 SQLite 会话索引均一致，无需修复"
            .to_string();
    }

    let added_index_suffix = if added_session_index_entry_count > 0 {
        format!(
            "，补写 {} 条 session_index 记录",
            added_session_index_entry_count
        )
    } else {
        String::new()
    };
    let updated_index_suffix = if updated_session_index_entry_count > 0 {
        format!(
            "，刷新 {} 条 session_index 记录",
            updated_session_index_entry_count
        )
    } else {
        String::new()
    };
    let running_suffix = if mutated_running_instance_count > 0 {
        "。运行中的实例可能需要重启后完全刷新"
    } else {
        ""
    };
    let metadata_suffix = if metadata_rebuild_failed_count > 0 {
        format!(
            "；{} 个实例的官方侧边栏索引重建未完成，重启 Codex 后会重新加载",
            metadata_rebuild_failed_count
        )
    } else {
        String::new()
    };

    format!(
        "已为 {} 个实例修复会话索引：校正 {} 个 rollout 文件，更新 {} 条 SQLite 可见性记录，校正 {} 条 SQLite 时间记录{}{}{}{}",
        mutated_instance_count,
        changed_rollout_file_count,
        updated_sqlite_row_count,
        updated_sqlite_timestamp_row_count,
        added_index_suffix,
        updated_index_suffix,
        running_suffix,
        metadata_suffix
    )
}

fn collect_instances() -> Result<Vec<CodexSyncInstance>, String> {
    let mut instances = Vec::new();
    let default_dir = modules::codex_instance::get_default_codex_home()?;
    let store = modules::codex_instance::load_instance_store()?;
    instances.push(CodexSyncInstance {
        id: DEFAULT_INSTANCE_ID.to_string(),
        name: DEFAULT_INSTANCE_NAME.to_string(),
        data_dir: default_dir,
        last_pid: store.default_settings.last_pid,
    });

    for instance in store.instances {
        let user_data_dir = instance.user_data_dir.trim();
        if user_data_dir.is_empty() {
            continue;
        }
        instances.push(CodexSyncInstance {
            id: instance.id,
            name: instance.name,
            data_dir: PathBuf::from(user_data_dir),
            last_pid: instance.last_pid,
        });
    }

    Ok(instances)
}

fn is_instance_running(
    instance: &CodexSyncInstance,
    process_entries: &[(u32, Option<String>)],
) -> bool {
    let codex_home = instance.data_dir.to_str();
    modules::process::resolve_codex_pid_from_entries(instance.last_pid, codex_home, process_entries)
        .is_some()
}

fn try_rebuild_thread_metadata(instance: &CodexSyncInstance) -> bool {
    match modules::codex_official_app_server::rebuild_thread_metadata(&instance.data_dir) {
        Ok(()) => true,
        Err(error) => {
            modules::logger::log_warn(&format!(
                "Codex 会话索引修复后触发官方侧边栏索引重建失败 ({} / {}): {}",
                instance.name,
                instance.data_dir.display(),
                error
            ));
            false
        }
    }
}

fn read_target_provider(data_dir: &Path) -> Result<String, String> {
    let config_path = data_dir.join(CONFIG_FILE_NAME);
    if !config_path.exists() {
        return Ok(DEFAULT_PROVIDER_ID.to_string());
    }

    let content = fs::read_to_string(&config_path).map_err(|error| {
        format!(
            "读取 config.toml 失败 ({}): {}",
            config_path.display(),
            error
        )
    })?;
    if content.trim().is_empty() {
        return Ok(DEFAULT_PROVIDER_ID.to_string());
    }

    let doc = modules::codex_config_format::read_codex_config_doc_from_str(&content).map_err(
        |error| {
            format!(
                "解析 config.toml 失败 ({}): {}",
                config_path.display(),
                error
            )
        },
    )?;
    let provider = doc
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_PROVIDER_ID);
    Ok(provider.to_string())
}

fn collect_rollout_provider_changes(
    data_dir: &Path,
    target_provider: &str,
) -> Result<Vec<RolloutProviderChange>, String> {
    let session_index_map = match read_session_index_map(data_dir) {
        Ok(value) => value,
        Err(error) => {
            modules::logger::log_warn(&format!(
                "读取 Codex session_index.jsonl 失败，跳过该时间来源并继续修复会话可见性: {}",
                error
            ));
            HashMap::new()
        }
    };
    let mut changes = Vec::new();

    for dir_name in SESSION_DIRS {
        let root_dir = data_dir.join(dir_name);
        if !root_dir.exists() {
            continue;
        }
        let rollout_paths = list_rollout_files(&root_dir)?;
        for rollout_path in rollout_paths {
            let content = fs::read_to_string(&rollout_path).map_err(|error| {
                format!(
                    "读取 rollout 文件失败 ({}): {}",
                    rollout_path.display(),
                    error
                )
            })?;
            let rewrite = rewrite_rollout_session_meta_providers(&content, target_provider)?;
            if rewrite.session_meta_count == 0 {
                continue;
            }
            let session_id = rewrite.thread_id.clone();
            let fallback_modified_ms =
                modules::codex_session_file_time::read_modified_time(&rollout_path)
                    .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
                    .map(|value| value.as_millis() as i128);
            let target_modified_at = resolve_target_modified_at_ms(
                session_id.as_deref(),
                &session_index_map,
                &rollout_path,
                fallback_modified_ms,
            )
            .and_then(modules::codex_session_file_time::system_time_from_unix_millis);
            let current_modified_at =
                modules::codex_session_file_time::read_modified_time(&rollout_path);
            let provider_matches = !rewrite.rewrite_needed;
            let modified_time_matches = target_modified_at.is_none()
                || modules::codex_session_file_time::same_modified_time_millis(
                    current_modified_at,
                    target_modified_at,
                );
            if provider_matches && modified_time_matches {
                continue;
            }

            let relative_path = rollout_path
                .strip_prefix(data_dir)
                .map_err(|_| format!("无法计算 rollout 相对路径: {}", rollout_path.display()))?;
            changes.push(RolloutProviderChange {
                relative_path: relative_path.to_path_buf(),
                absolute_path: rollout_path,
                updated_content: if rewrite.rewrite_needed {
                    Some(rewrite.next_content)
                } else {
                    None
                },
                target_modified_at,
            });
        }
    }

    changes.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(changes)
}

#[derive(Debug, Default)]
struct RolloutProviderRewrite {
    next_content: String,
    rewrite_needed: bool,
    thread_id: Option<String>,
    session_meta_count: usize,
}

fn rewrite_rollout_session_meta_providers(
    content: &str,
    target_provider: &str,
) -> Result<RolloutProviderRewrite, String> {
    let mut rewrite = RolloutProviderRewrite::default();
    for segment in content.split_inclusive('\n') {
        let (line, line_ending) = split_line_ending(segment);
        let mut next_line = line.to_string();
        if !line.trim().is_empty() {
            if let Ok(mut record) = serde_json::from_str::<JsonValue>(line) {
                if record.get("type").and_then(JsonValue::as_str) == Some("session_meta") {
                    let Some(payload) =
                        record.get_mut("payload").and_then(JsonValue::as_object_mut)
                    else {
                        rewrite.next_content.push_str(&next_line);
                        rewrite.next_content.push_str(line_ending);
                        continue;
                    };
                    rewrite.session_meta_count += 1;
                    if rewrite.thread_id.is_none() {
                        rewrite.thread_id = payload
                            .get("id")
                            .or_else(|| payload.get("session_id"))
                            .and_then(JsonValue::as_str)
                            .map(str::to_string);
                    }
                    if payload.get("model_provider").and_then(JsonValue::as_str)
                        != Some(target_provider)
                    {
                        payload.insert(
                            "model_provider".to_string(),
                            JsonValue::String(target_provider.to_string()),
                        );
                        next_line = serde_json::to_string(&record)
                            .map_err(|error| format!("序列化 session_meta 失败: {}", error))?;
                        rewrite.rewrite_needed = true;
                    }
                }
            }
        }
        rewrite.next_content.push_str(&next_line);
        rewrite.next_content.push_str(line_ending);
    }
    if !content.ends_with('\n') && rewrite.next_content.ends_with('\n') {
        rewrite.next_content.pop();
    }
    Ok(rewrite)
}

fn split_line_ending(segment: &str) -> (&str, &str) {
    if let Some(line) = segment.strip_suffix("\r\n") {
        (line, "\r\n")
    } else if let Some(line) = segment.strip_suffix('\n') {
        (line, "\n")
    } else {
        (segment, "")
    }
}

fn collect_rollout_thread_facts(data_dir: &Path) -> Result<RolloutThreadFacts, String> {
    let mut facts = RolloutThreadFacts::default();
    for dir_name in SESSION_DIRS {
        let root_dir = data_dir.join(dir_name);
        if !root_dir.exists() {
            continue;
        }
        for rollout_path in list_rollout_files(&root_dir)? {
            let content = fs::read_to_string(&rollout_path).map_err(|error| {
                format!(
                    "读取 rollout 文件失败 ({}): {}",
                    rollout_path.display(),
                    error
                )
            })?;
            let has_user_event =
                content.contains("\"user_message\"") || content.contains("\"user_input\"");
            for line in content.lines() {
                let Ok(record) = serde_json::from_str::<JsonValue>(line.trim()) else {
                    continue;
                };
                if record.get("type").and_then(JsonValue::as_str) != Some("session_meta") {
                    continue;
                }
                let Some(payload) = record.get("payload").and_then(JsonValue::as_object) else {
                    continue;
                };
                let Some(thread_id) = payload
                    .get("id")
                    .or_else(|| payload.get("session_id"))
                    .and_then(JsonValue::as_str)
                    .map(str::to_string)
                else {
                    continue;
                };
                if has_user_event {
                    facts.user_event_thread_ids.insert(thread_id.clone());
                }
                if let Some(cwd) = payload
                    .get("cwd")
                    .and_then(JsonValue::as_str)
                    .and_then(to_desktop_workspace_path)
                {
                    facts.cwd_by_thread_id.entry(thread_id).or_insert(cwd);
                }
            }
        }
    }
    Ok(facts)
}

fn to_desktop_workspace_path(value: &str) -> Option<String> {
    let stripped = value.trim();
    if stripped.is_empty() {
        return None;
    }
    let lower = stripped.to_ascii_lowercase();
    if lower.starts_with(r"\\?\unc\") {
        return Some(format!(r"\\{}", stripped[8..].replace('/', r"\")));
    }
    if stripped.starts_with(r"\\?\") {
        return Some(stripped[4..].replace('\\', "/"));
    }
    Some(stripped.to_string())
}

fn list_rollout_files(root_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut result = Vec::new();
    let entries = fs::read_dir(root_dir)
        .map_err(|error| format!("读取目录失败 ({}): {}", root_dir.display(), error))?;

    for entry in entries {
        let entry =
            entry.map_err(|error| format!("读取目录项失败 ({}): {}", root_dir.display(), error))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("读取文件类型失败 ({}): {}", path.display(), error))?;
        if file_type.is_dir() {
            result.extend(list_rollout_files(&path)?);
            continue;
        }
        if file_type.is_file() {
            let file_name = path
                .file_name()
                .and_then(|item| item.to_str())
                .unwrap_or_default();
            if file_name.starts_with("rollout-") && file_name.ends_with(".jsonl") {
                result.push(path);
            }
        }
    }

    result.sort();
    Ok(result)
}

fn read_session_index_map(root_dir: &Path) -> Result<HashMap<String, JsonValue>, String> {
    let path = root_dir.join(SESSION_INDEX_FILE);
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let content = fs::read_to_string(&path).map_err(|error| {
        format!(
            "读取 session_index.jsonl 失败 ({}): {}",
            path.display(),
            error
        )
    })?;
    let mut entries = HashMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<JsonValue>(trimmed) else {
            continue;
        };
        let Some(id) = entry.get("id").and_then(JsonValue::as_str) else {
            continue;
        };
        entries.insert(id.to_string(), entry);
    }
    Ok(entries)
}

fn count_session_index_entries_to_repair(
    data_dir: &Path,
) -> Result<SessionIndexRepairScan, String> {
    let session_index_map = read_session_index_map(data_dir)?;
    let rows = load_sqlite_thread_index_rows(data_dir)?;
    let mut scan = SessionIndexRepairScan::default();
    for row in &rows {
        match session_index_map.get(&row.id) {
            Some(entry) if session_index_entry_needs_update(data_dir, row, entry) => {
                scan.entries_to_update += 1;
            }
            Some(_) => {}
            None => {
                scan.entries_to_add += 1;
            }
        }
    }
    Ok(scan)
}

fn count_missing_session_index_entries(data_dir: &Path) -> Result<usize, String> {
    Ok(count_session_index_entries_to_repair(data_dir)?.entries_to_add)
}

fn load_sqlite_thread_index_rows(data_dir: &Path) -> Result<Vec<SqliteThreadIndexRow>, String> {
    let mut rows = Vec::new();
    let mut seen_ids = HashSet::new();
    for db_path in sqlite_candidate_paths(data_dir) {
        for row in load_sqlite_thread_index_rows_from_db(&db_path)? {
            if seen_ids.insert(row.id.clone()) {
                rows.push(row);
            }
        }
    }
    Ok(rows)
}

fn load_sqlite_thread_index_rows_from_db(
    db_path: &Path,
) -> Result<Vec<SqliteThreadIndexRow>, String> {
    if !db_path.exists() {
        return Ok(Vec::new());
    }

    let connection = match Connection::open(db_path) {
        Ok(connection) => connection,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(Vec::new());
        }
        Err(error) => {
            return Err(format!(
                "打开实例数据库失败 ({}): {}",
                db_path.display(),
                error
            ));
        }
    };

    let mut statement = match connection.prepare("PRAGMA table_info(threads)") {
        Ok(statement) => statement,
        Err(error) if is_missing_threads_table_error(&error) => return Ok(Vec::new()),
        Err(error) => {
            return Err(format_sqlite_read_error(
                db_path,
                "读取 SQLite threads 表结构失败",
                &error,
            ));
        }
    };
    let rows = statement
        .query_map([], |row| row.get::<usize, String>(1))
        .map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite threads 表结构失败", &error)
        })?;
    let mut names = HashSet::new();
    for row in rows {
        names.insert(row.map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite threads 表结构失败", &error)
        })?);
    }
    if !names.contains("id") {
        return Ok(Vec::new());
    }

    let title_expr = if names.contains("title") {
        "COALESCE(title, '')"
    } else {
        "''"
    };
    let updated_at_expr = if names.contains("updated_at") {
        "updated_at"
    } else {
        "NULL"
    };
    let updated_at_ms_expr = if names.contains("updated_at_ms") {
        "updated_at_ms"
    } else {
        "NULL"
    };
    let rollout_path_expr = if names.contains("rollout_path") {
        "rollout_path"
    } else {
        "NULL"
    };
    let order_expr = if names.contains("updated_at") {
        "updated_at DESC"
    } else {
        "id ASC"
    };
    let sql = format!(
        "SELECT id, {title_expr}, {updated_at_expr}, {updated_at_ms_expr}, {rollout_path_expr} FROM threads ORDER BY {order_expr}"
    );
    let mut statement = connection.prepare(sql.as_str()).map_err(|error| {
        format!(
            "准备 SQLite 会话索引查询失败 ({}): {}",
            db_path.display(),
            error
        )
    })?;
    let mapped = statement
        .query_map([], |row| {
            Ok(SqliteThreadIndexRow {
                id: row.get(0)?,
                title: row.get(1)?,
                updated_at: row.get(2)?,
                updated_at_ms: row.get(3)?,
                rollout_path: row.get(4)?,
            })
        })
        .map_err(|error| {
            format!(
                "查询 SQLite 会话索引行失败 ({}): {}",
                db_path.display(),
                error
            )
        })?;
    let mut result = Vec::new();
    for row in mapped {
        result.push(row.map_err(|error| {
            format!(
                "读取 SQLite 会话索引行失败 ({}): {}",
                db_path.display(),
                error
            )
        })?);
    }
    Ok(result)
}

fn format_thread_updated_at_iso_ms(updated_at_ms: Option<i128>) -> String {
    let milliseconds = updated_at_ms.unwrap_or_else(|| Utc::now().timestamp_millis() as i128);
    i64::try_from(milliseconds)
        .ok()
        .and_then(|value| Utc.timestamp_millis_opt(value).single())
        .unwrap_or_else(Utc::now)
        .to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}

fn resolve_thread_updated_at_ms(data_dir: &Path, row: &SqliteThreadIndexRow) -> Option<i128> {
    let rollout_activity_ms = row
        .rollout_path
        .as_deref()
        .map(|path| resolve_rollout_path(data_dir, path))
        .filter(|path| path.exists())
        .and_then(|path| rollout_file_activity_ms(&path));
    let sqlite_ms = row
        .updated_at_ms
        .map(|value| value as i128)
        .or_else(|| row.updated_at.map(|value| value as i128 * 1000));
    match (sqlite_ms, rollout_activity_ms) {
        (Some(sqlite_ms), Some(activity_ms))
            if (sqlite_ms - activity_ms).abs() > SESSION_INDEX_ACTIVITY_DRIFT_MS =>
        {
            Some(activity_ms)
        }
        (Some(sqlite_ms), _) => Some(sqlite_ms),
        (None, Some(activity_ms)) => Some(activity_ms),
        (None, None) => None,
    }
}

fn build_session_index_entry_from_thread(data_dir: &Path, row: &SqliteThreadIndexRow) -> JsonValue {
    json!({
        "id": row.id,
        "thread_name": if row.title.trim().is_empty() {
            "Untitled"
        } else {
            row.title.as_str()
        },
        "updated_at": format_thread_updated_at_iso_ms(resolve_thread_updated_at_ms(data_dir, row)),
    })
}

fn build_updated_session_index_entry(
    data_dir: &Path,
    existing: &JsonValue,
    row: &SqliteThreadIndexRow,
) -> JsonValue {
    let mut entry = existing.clone();
    let Some(object) = entry.as_object_mut() else {
        return build_session_index_entry_from_thread(data_dir, row);
    };
    object.insert("id".to_string(), JsonValue::String(row.id.clone()));
    if !row.title.trim().is_empty() {
        object.insert(
            "thread_name".to_string(),
            JsonValue::String(row.title.clone()),
        );
    }
    object.insert(
        "updated_at".to_string(),
        JsonValue::String(format_thread_updated_at_iso_ms(
            resolve_thread_updated_at_ms(data_dir, row),
        )),
    );
    entry
}

fn session_index_entry_needs_update(
    data_dir: &Path,
    row: &SqliteThreadIndexRow,
    entry: &JsonValue,
) -> bool {
    let Some(target_ms) = resolve_thread_updated_at_ms(data_dir, row) else {
        return false;
    };
    match parse_session_index_updated_at_ms(entry) {
        Some(current_ms) => (current_ms - target_ms).abs() > 1000,
        None => true,
    }
}

fn reconcile_session_index_from_sqlite(
    data_dir: &Path,
) -> Result<SessionIndexReconcileResult, String> {
    let session_index_map = read_session_index_map(data_dir)?;
    let rows = load_sqlite_thread_index_rows(data_dir)?;
    let mut entries_to_add = Vec::<JsonValue>::new();
    let mut entries_to_update = HashMap::<String, JsonValue>::new();
    for row in &rows {
        match session_index_map.get(&row.id) {
            Some(existing) if session_index_entry_needs_update(data_dir, row, existing) => {
                entries_to_update.insert(
                    row.id.clone(),
                    build_updated_session_index_entry(data_dir, existing, row),
                );
            }
            Some(_) => {}
            None => entries_to_add.push(build_session_index_entry_from_thread(data_dir, row)),
        }
    }
    if entries_to_add.is_empty() && entries_to_update.is_empty() {
        return Ok(SessionIndexReconcileResult::default());
    }

    let path = data_dir.join(SESSION_INDEX_FILE);
    let mut lines = if path.exists() {
        fs::read_to_string(&path)
            .map_err(|error| {
                format!(
                    "读取 session_index.jsonl 失败 ({}): {}",
                    path.display(),
                    error
                )
            })?
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }

    let mut updated_ids = HashSet::<String>::new();
    for line in &mut lines {
        let Ok(entry) = serde_json::from_str::<JsonValue>(line.trim()) else {
            continue;
        };
        let Some(id) = entry.get("id").and_then(JsonValue::as_str) else {
            continue;
        };
        let Some(updated_entry) = entries_to_update.get(id) else {
            continue;
        };
        *line = serde_json::to_string(updated_entry)
            .map_err(|error| format!("序列化 session_index 条目失败: {}", error))?;
        updated_ids.insert(id.to_string());
    }

    for entry in &entries_to_add {
        let line = serde_json::to_string(&entry)
            .map_err(|error| format!("序列化 session_index 条目失败: {}", error))?;
        lines.push(line);
    }

    let mut output = lines.join("\n");
    output.push('\n');
    modules::atomic_write::write_string_atomic(&path, &output).map_err(|error| {
        format!(
            "写入 session_index.jsonl 失败 ({}): {}",
            path.display(),
            error
        )
    })?;
    Ok(SessionIndexReconcileResult {
        added_entries: entries_to_add.len(),
        updated_entries: updated_ids.len(),
    })
}

fn normalize_codex_timestamp_ms(timestamp: i64) -> i128 {
    let timestamp = timestamp as i128;
    if timestamp > 10_000_000_000_000 {
        timestamp / 1_000
    } else if timestamp > 10_000_000_000 {
        timestamp
    } else {
        timestamp * 1_000
    }
}

fn parse_timestamp_ms(value: &JsonValue) -> Option<i128> {
    match value {
        JsonValue::Number(number) => number.as_i64().map(normalize_codex_timestamp_ms),
        JsonValue::String(text) => chrono::DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|value| value.timestamp_millis() as i128)
            .or_else(|| text.parse::<i64>().ok().map(normalize_codex_timestamp_ms)),
        _ => None,
    }
}

fn parse_session_index_updated_at_ms(entry: &JsonValue) -> Option<i128> {
    [
        "updated_at",
        "updatedAt",
        "last_updated_at",
        "lastUpdatedAt",
    ]
    .iter()
    .filter_map(|key| entry.get(*key))
    .find_map(parse_timestamp_ms)
}

fn parse_rollout_line_timestamp_ms(value: &JsonValue) -> Option<i128> {
    value
        .get("timestamp")
        .or_else(|| value.get("time"))
        .or_else(|| value.get("created_at"))
        .or_else(|| value.get("createdAt"))
        .and_then(parse_timestamp_ms)
        .or_else(|| {
            value
                .get("payload")
                .and_then(|payload| {
                    payload
                        .get("timestamp")
                        .or_else(|| payload.get("time"))
                        .or_else(|| payload.get("created_at"))
                        .or_else(|| payload.get("createdAt"))
                })
                .and_then(parse_timestamp_ms)
        })
}

fn rollout_file_activity_ms(path: &Path) -> Option<i128> {
    let content = fs::read_to_string(path).ok()?;
    content
        .lines()
        .filter_map(|line| serde_json::from_str::<JsonValue>(line.trim()).ok())
        .filter_map(|value| parse_rollout_line_timestamp_ms(&value))
        .max()
}

fn resolve_target_modified_at_ms(
    session_id: Option<&str>,
    session_index_map: &HashMap<String, JsonValue>,
    rollout_path: &Path,
    fallback_ms: Option<i128>,
) -> Option<i128> {
    let indexed = session_id
        .and_then(|id| session_index_map.get(id))
        .and_then(parse_session_index_updated_at_ms);
    let activity = rollout_file_activity_ms(rollout_path);
    match (indexed, activity) {
        (Some(indexed), Some(activity))
            if (indexed - activity).abs() > SESSION_INDEX_ACTIVITY_DRIFT_MS =>
        {
            Some(activity)
        }
        (Some(indexed), _) => Some(indexed),
        (None, Some(activity)) => Some(activity),
        (None, None) => fallback_ms,
    }
}

fn resolve_rollout_path(data_dir: &Path, rollout_path: &str) -> PathBuf {
    let trimmed = rollout_path.trim();
    let stripped = trimmed.strip_prefix(r"\\?\").unwrap_or(trimmed);
    let path = Path::new(stripped);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        data_dir.join(path)
    }
}

fn count_sqlite_thread_timestamps_to_update(data_dir: &Path) -> Result<usize, String> {
    let mut total = 0usize;
    for db_path in sqlite_candidate_paths(data_dir) {
        total += plan_sqlite_thread_timestamp_repair_for_db(data_dir, &db_path)?
            .updates
            .len();
    }
    Ok(total)
}

fn plan_sqlite_thread_timestamp_repair_for_db(
    data_dir: &Path,
    db_path: &Path,
) -> Result<SqliteTimestampRepairPlan, String> {
    if !db_path.exists() {
        return Ok(SqliteTimestampRepairPlan::default());
    }

    let connection = match Connection::open(db_path) {
        Ok(connection) => connection,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(SqliteTimestampRepairPlan::default());
        }
        Err(error) => {
            return Err(format!(
                "打开实例数据库失败 ({}): {}",
                db_path.display(),
                error
            ));
        }
    };

    let mut statement = match connection.prepare("PRAGMA table_info(threads)") {
        Ok(statement) => statement,
        Err(error) if is_missing_threads_table_error(&error) => {
            return Ok(SqliteTimestampRepairPlan::default())
        }
        Err(error) => {
            return Err(format_sqlite_read_error(
                db_path,
                "读取 SQLite threads 表结构失败",
                &error,
            ));
        }
    };
    let rows = statement
        .query_map([], |row| row.get::<usize, String>(1))
        .map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite threads 表结构失败", &error)
        })?;
    let mut names = HashSet::new();
    for row in rows {
        names.insert(row.map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite threads 表结构失败", &error)
        })?);
    }
    drop(statement);

    let has_updated_at = names.contains("updated_at");
    let has_updated_at_ms = names.contains("updated_at_ms");
    if !names.contains("id")
        || !names.contains("rollout_path")
        || (!has_updated_at && !has_updated_at_ms)
    {
        return Ok(SqliteTimestampRepairPlan::default());
    }

    let updated_at_expr = if has_updated_at { "updated_at" } else { "NULL" };
    let updated_at_ms_expr = if has_updated_at_ms {
        "updated_at_ms"
    } else {
        "NULL"
    };
    let sql = format!(
        "SELECT id, rollout_path, {updated_at_expr}, {updated_at_ms_expr} FROM threads WHERE rollout_path IS NOT NULL AND rollout_path <> ''"
    );
    let mut statement = connection.prepare(sql.as_str()).map_err(|error| {
        format_sqlite_read_error(db_path, "准备 SQLite 会话时间修复查询失败", &error)
    })?;

    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })
        .map_err(|error| format_sqlite_read_error(db_path, "查询 SQLite 会话时间失败", &error))?;

    let mut updates = Vec::new();
    for row in rows {
        let (thread_id, rollout_path, updated_at, updated_at_ms) = row.map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite 会话时间失败", &error)
        })?;
        let rollout = resolve_rollout_path(data_dir, &rollout_path);
        if !rollout.exists() {
            continue;
        }
        let Some(activity_ms) = rollout_file_activity_ms(&rollout) else {
            continue;
        };
        let activity_seconds = (activity_ms / 1000) as i64;
        let activity_ms = activity_seconds * 1000;
        let current_ms = updated_at_ms
            .or_else(|| updated_at.map(|value| value * 1000))
            .unwrap_or(0);
        if i64::abs(current_ms - activity_ms) <= 1000 {
            continue;
        }
        updates.push(SqliteTimestampUpdate {
            id: thread_id,
            updated_at_seconds: activity_seconds,
            updated_at_ms: activity_ms,
        });
    }
    Ok(SqliteTimestampRepairPlan {
        updates,
        has_updated_at,
        has_updated_at_ms,
    })
}

fn repair_sqlite_thread_timestamps(data_dir: &Path) -> Result<usize, String> {
    let mut total = 0usize;
    for db_path in sqlite_candidate_paths(data_dir) {
        total += repair_sqlite_thread_timestamps_for_db(data_dir, &db_path)?;
    }
    Ok(total)
}

fn repair_sqlite_thread_timestamps_for_db(
    data_dir: &Path,
    db_path: &Path,
) -> Result<usize, String> {
    if !db_path.exists() {
        return Ok(0);
    }

    let plan = plan_sqlite_thread_timestamp_repair_for_db(data_dir, db_path)?;
    let updates = plan.updates;

    if updates.is_empty() {
        return Ok(0);
    }

    let mut connection = match Connection::open(db_path) {
        Ok(connection) => connection,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(0);
        }
        Err(error) => {
            return Err(format!(
                "打开实例数据库失败 ({}): {}",
                db_path.display(),
                error
            ));
        }
    };
    let transaction = connection
        .transaction()
        .map_err(|error| format_sqlite_write_error(db_path, &error))?;
    for update in &updates {
        if plan.has_updated_at && plan.has_updated_at_ms {
            transaction
                .execute(
                    "UPDATE threads SET updated_at = ?1, updated_at_ms = ?2 WHERE id = ?3",
                    (
                        update.updated_at_seconds,
                        update.updated_at_ms,
                        update.id.as_str(),
                    ),
                )
                .map_err(|error| format_sqlite_write_error(db_path, &error))?;
        } else if plan.has_updated_at {
            transaction
                .execute(
                    "UPDATE threads SET updated_at = ?1 WHERE id = ?2",
                    (update.updated_at_seconds, update.id.as_str()),
                )
                .map_err(|error| format_sqlite_write_error(db_path, &error))?;
        } else if plan.has_updated_at_ms {
            transaction
                .execute(
                    "UPDATE threads SET updated_at_ms = ?1 WHERE id = ?2",
                    (update.updated_at_ms, update.id.as_str()),
                )
                .map_err(|error| format_sqlite_write_error(db_path, &error))?;
        }
    }
    transaction
        .commit()
        .map_err(|error| format_sqlite_write_error(db_path, &error))?;
    Ok(updates.len())
}

fn is_missing_threads_table_error(error: &rusqlite::Error) -> bool {
    error
        .to_string()
        .to_ascii_lowercase()
        .contains("no such table: threads")
}

fn log_skipped_sqlite_database(path: &Path, reason: &str) {
    modules::logger::log_warn(&format!(
        "跳过无效或损坏的 Codex SQLite 会话库 ({}): {}",
        path.display(),
        reason
    ));
}

fn count_sqlite_rows_to_update(
    data_dir: &Path,
    target_provider: &str,
) -> Result<SqliteProviderScan, String> {
    let facts = collect_rollout_thread_facts(data_dir)?;
    let mut scan = SqliteProviderScan {
        rows_to_update: 0,
        skipped_unusable_database: false,
    };
    for db_path in sqlite_candidate_paths(data_dir) {
        let item = count_sqlite_rows_to_update_for_db(&db_path, target_provider, &facts)?;
        scan.rows_to_update += item.rows_to_update;
        scan.skipped_unusable_database |= item.skipped_unusable_database;
    }
    Ok(scan)
}

fn count_sqlite_rows_to_update_for_db(
    db_path: &Path,
    target_provider: &str,
    facts: &RolloutThreadFacts,
) -> Result<SqliteProviderScan, String> {
    if !db_path.exists() {
        return Ok(SqliteProviderScan {
            rows_to_update: 0,
            skipped_unusable_database: false,
        });
    }

    let connection = match Connection::open(db_path) {
        Ok(connection) => connection,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(SqliteProviderScan {
                rows_to_update: 0,
                skipped_unusable_database: true,
            });
        }
        Err(error) => {
            return Err(format!(
                "打开实例数据库失败 ({}): {}",
                db_path.display(),
                error
            ));
        }
    };
    let columns = match read_threads_table_columns(&connection) {
        Ok(columns) => columns,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(SqliteProviderScan {
                rows_to_update: 0,
                skipped_unusable_database: true,
            });
        }
        Err(error) => {
            return Err(format_sqlite_read_error(
                db_path,
                "读取 SQLite threads 表结构失败",
                &error,
            ));
        }
    };
    let Some(columns) = columns else {
        return Ok(SqliteProviderScan {
            rows_to_update: 0,
            skipped_unusable_database: false,
        });
    };
    let mut count = 0i64;
    if let Some(where_clause) = build_threads_repair_where_clause(columns) {
        let sql = format!("SELECT COUNT(*) FROM threads WHERE {where_clause}");
        let count_result = if columns.model_provider {
            connection.query_row(sql.as_str(), [target_provider], |row| {
                row.get::<usize, i64>(0)
            })
        } else {
            connection.query_row(sql.as_str(), [], |row| row.get::<usize, i64>(0))
        };
        count += match count_result {
            Ok(count) => count,
            Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
                log_skipped_sqlite_database(db_path, &error.to_string());
                return Ok(SqliteProviderScan {
                    rows_to_update: 0,
                    skipped_unusable_database: true,
                });
            }
            Err(error) if is_missing_threads_table_error(&error) => {
                return Ok(SqliteProviderScan {
                    rows_to_update: 0,
                    skipped_unusable_database: false,
                });
            }
            Err(error) => {
                return Err(format!(
                    "统计 SQLite 会话可见性差异失败 ({}): {}",
                    db_path.display(),
                    error
                ));
            }
        };
    }
    if columns.has_user_event {
        for thread_id in &facts.user_event_thread_ids {
            count += connection
                .query_row(
                    "SELECT COUNT(*) FROM threads WHERE id = ?1 AND COALESCE(has_user_event, 0) <> 1",
                    [thread_id.as_str()],
                    |row| row.get::<usize, i64>(0),
                )
                .map_err(|error| {
                    format!(
                        "统计 SQLite has_user_event 差异失败 ({}): {}",
                        db_path.display(),
                        error
                    )
                })?;
        }
    }
    if columns.cwd {
        for (thread_id, cwd) in &facts.cwd_by_thread_id {
            count += connection
                .query_row(
                    "SELECT COUNT(*) FROM threads WHERE id = ?1 AND COALESCE(cwd, '') <> ?2",
                    (thread_id.as_str(), cwd.as_str()),
                    |row| row.get::<usize, i64>(0),
                )
                .map_err(|error| {
                    format!(
                        "统计 SQLite cwd 差异失败 ({}): {}",
                        db_path.display(),
                        error
                    )
                })?;
        }
    }
    Ok(SqliteProviderScan {
        rows_to_update: count.max(0) as usize,
        skipped_unusable_database: false,
    })
}

fn update_sqlite_provider(data_dir: &Path, target_provider: &str) -> Result<usize, String> {
    let facts = collect_rollout_thread_facts(data_dir)?;
    let mut total = 0usize;
    for db_path in sqlite_candidate_paths(data_dir) {
        total += update_sqlite_provider_for_db(&db_path, target_provider, &facts)?;
    }
    Ok(total)
}

fn update_sqlite_provider_for_db(
    db_path: &Path,
    target_provider: &str,
    facts: &RolloutThreadFacts,
) -> Result<usize, String> {
    if !db_path.exists() {
        return Ok(0);
    }

    let mut connection = match Connection::open(db_path) {
        Ok(connection) => connection,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(0);
        }
        Err(error) => {
            return Err(format!(
                "打开实例数据库失败 ({}): {}",
                db_path.display(),
                error
            ));
        }
    };
    connection
        .busy_timeout(Duration::from_secs(3))
        .map_err(|error| {
            format!(
                "设置 SQLite busy_timeout 失败 ({}): {}",
                db_path.display(),
                error
            )
        })?;
    let columns = match read_threads_table_columns(&connection) {
        Ok(columns) => columns,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(0);
        }
        Err(error) => {
            return Err(format_sqlite_read_error(
                db_path,
                "读取 SQLite threads 表结构失败",
                &error,
            ));
        }
    };
    let Some(columns) = columns else {
        return Ok(0);
    };
    let transaction = connection
        .transaction()
        .map_err(|error| format_sqlite_write_error(db_path, &error))?;
    let mut updated_rows = 0usize;
    if let Some(where_clause) = build_threads_repair_where_clause(columns) {
        let set_clause = build_threads_repair_set_clause(columns);
        let sql = format!("UPDATE threads SET {set_clause} WHERE {where_clause}");
        let update_result = if columns.model_provider {
            transaction.execute(sql.as_str(), [target_provider])
        } else {
            transaction.execute(sql.as_str(), [])
        };
        updated_rows += match update_result {
            Ok(updated_rows) => updated_rows,
            Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
                log_skipped_sqlite_database(db_path, &error.to_string());
                return Ok(0);
            }
            Err(error) if is_missing_threads_table_error(&error) => {
                return Ok(0);
            }
            Err(error) => return Err(format_sqlite_write_error(db_path, &error)),
        };
    }
    if columns.has_user_event {
        for thread_id in &facts.user_event_thread_ids {
            updated_rows += transaction
                .execute(
                    "UPDATE threads SET has_user_event = 1 WHERE id = ?1 AND COALESCE(has_user_event, 0) <> 1",
                    [thread_id.as_str()],
                )
                .map_err(|error| format_sqlite_write_error(db_path, &error))?;
        }
    }
    if columns.cwd {
        for (thread_id, cwd) in &facts.cwd_by_thread_id {
            updated_rows += transaction
                .execute(
                    "UPDATE threads SET cwd = ?1 WHERE id = ?2 AND COALESCE(cwd, '') <> ?1",
                    (cwd.as_str(), thread_id.as_str()),
                )
                .map_err(|error| format_sqlite_write_error(db_path, &error))?;
        }
    }
    if let Err(error) = transaction.commit() {
        if modules::db::is_unusable_sqlite_database_error(&error) {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(0);
        }
        return Err(format_sqlite_write_error(db_path, &error));
    }
    Ok(updated_rows)
}

fn read_threads_table_columns(
    connection: &Connection,
) -> Result<Option<ThreadsTableColumns>, rusqlite::Error> {
    let mut statement = connection.prepare("PRAGMA table_info(threads)")?;
    let rows = statement.query_map([], |row| row.get::<usize, String>(1))?;
    let mut names = HashSet::new();
    for row in rows {
        let name = row?;
        names.insert(name);
    }
    if names.is_empty() {
        return Ok(None);
    }
    Ok(Some(ThreadsTableColumns {
        model_provider: names.contains("model_provider"),
        has_user_event: names.contains("has_user_event"),
        first_user_message: names.contains("first_user_message"),
        thread_source: names.contains("thread_source"),
        cwd: names.contains("cwd"),
    }))
}

fn build_threads_repair_where_clause(columns: ThreadsTableColumns) -> Option<String> {
    let mut predicates = Vec::new();
    if columns.model_provider {
        predicates.push("COALESCE(model_provider, '') <> ?1");
    }
    if columns.has_user_event && columns.first_user_message {
        predicates
            .push("(COALESCE(first_user_message, '') <> '' AND COALESCE(has_user_event, 0) <> 1)");
    }
    if columns.thread_source && columns.first_user_message {
        predicates
            .push("(COALESCE(first_user_message, '') <> '' AND COALESCE(thread_source, '') = '')");
    }
    if predicates.is_empty() {
        None
    } else {
        Some(predicates.join(" OR "))
    }
}

fn build_threads_repair_set_clause(columns: ThreadsTableColumns) -> String {
    let mut assignments = Vec::new();
    if columns.model_provider {
        assignments.push("model_provider = ?1");
    }
    if columns.has_user_event && columns.first_user_message {
        assignments.push(
            "has_user_event = CASE WHEN COALESCE(first_user_message, '') <> '' THEN 1 ELSE has_user_event END",
        );
    }
    if columns.thread_source && columns.first_user_message {
        assignments.push(
            "thread_source = CASE WHEN COALESCE(thread_source, '') = '' AND COALESCE(first_user_message, '') <> '' THEN 'user' ELSE thread_source END",
        );
    }
    assignments.join(", ")
}

fn format_sqlite_read_error(path: &Path, action: &str, error: &rusqlite::Error) -> String {
    format!("{} ({}): {}", action, path.display(), error)
}

fn format_sqlite_write_error(path: &Path, error: &rusqlite::Error) -> String {
    let message = error.to_string();
    let lowered = message.to_ascii_lowercase();
    if lowered.contains("database is locked") || lowered.contains("database busy") {
        return format!(
            "Codex SQLite 会话库当前被占用，请关闭 Codex / Codex App 后重试 ({}): {}",
            path.display(),
            message
        );
    }
    format!(
        "更新 SQLite 会话可见性失败 ({}): {}",
        path.display(),
        message
    )
}

fn rewrite_rollout_provider(change: &RolloutProviderChange) -> Result<(), String> {
    let original_modified_at =
        modules::codex_session_file_time::read_modified_time(&change.absolute_path);
    if let Some(updated_content) = change.updated_content.as_deref() {
        write_bytes_atomic(&change.absolute_path, updated_content.as_bytes())?;
    }
    modules::codex_session_file_time::restore_modified_time(
        &change.absolute_path,
        change.target_modified_at.or(original_modified_at),
    )
}

fn write_bytes_atomic(path: &Path, content: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("无法定位目标目录: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("创建目录失败 ({}): {}", parent.display(), error))?;

    let temp_path = parent.join(format!(
        ".{}.provider-repair.{}.{}",
        path.file_name()
            .and_then(|item| item.to_str())
            .unwrap_or("file"),
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::write(&temp_path, content)
        .map_err(|error| format!("写入临时文件失败 ({}): {}", temp_path.display(), error))?;
    if let Err(error) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format!("替换文件失败 ({}): {}", path.display(), error));
    }
    Ok(())
}

fn sqlite_candidate_paths(data_dir: &Path) -> Vec<PathBuf> {
    let mut paths = sqlite_dir_session_dbs(data_dir);
    let legacy = data_dir.join(STATE_DB_FILE);
    if !paths.iter().any(|path| path == &legacy) {
        paths.push(legacy);
    }
    paths
}

fn sqlite_dir_session_dbs(data_dir: &Path) -> Vec<PathBuf> {
    let sqlite_dir = data_dir.join(SQLITE_DIR_NAME);
    let Ok(entries) = fs::read_dir(&sqlite_dir) else {
        return Vec::new();
    };
    let mut candidates = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| is_sqlite_candidate(path))
        .filter(|path| has_codex_session_table(path))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|path| {
        (
            path.file_name()
                .map(|name| name != OsStr::new(PREFERRED_SQLITE_DB_FILE))
                .unwrap_or(true),
            path.file_name().map(|name| name.to_os_string()),
        )
    });
    candidates
}

fn is_sqlite_candidate(path: &Path) -> bool {
    matches!(
        path.extension().and_then(OsStr::to_str),
        Some("db") | Some("sqlite") | Some("sqlite3")
    )
}

fn has_codex_session_table(path: &Path) -> bool {
    let Ok(connection) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return false;
    };
    ["threads", "automation_runs", "inbox_items"]
        .iter()
        .any(|table| {
            connection
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
                    [table],
                    |_| Ok(()),
                )
                .is_ok()
        })
}

fn relative_to_instance_root(data_dir: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(data_dir).unwrap_or(path).to_path_buf()
}

fn sqlite_sidecar_paths(db_path: &Path) -> Vec<PathBuf> {
    let raw = db_path.to_string_lossy();
    vec![
        PathBuf::from(format!("{}-wal", raw)),
        PathBuf::from(format!("{}-shm", raw)),
    ]
}

fn remove_sqlite_sidecar_files(db_path: &Path) -> Result<(), String> {
    for path in sqlite_sidecar_paths(db_path) {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "清理 SQLite sidecar 文件失败 ({}): {}",
                    path.display(),
                    error
                ));
            }
        }
    }
    Ok(())
}

fn backup_sqlite_databases(data_dir: &Path, backup_dir: &Path) -> Result<Vec<String>, String> {
    let mut backed_up = Vec::new();
    for db_path in sqlite_candidate_paths(data_dir) {
        if !db_path.exists() {
            continue;
        }
        let relative = relative_to_instance_root(data_dir, &db_path);
        let backup_db_path = backup_dir.join("db").join(&relative);
        if let Some(parent) = backup_db_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("创建 SQLite 备份目录失败 ({}): {}", parent.display(), error)
            })?;
        }
        let connection = Connection::open(&db_path).map_err(|error| {
            format!(
                "打开 SQLite 会话库以创建一致备份失败 ({}): {}",
                db_path.display(),
                error
            )
        })?;
        connection
            .busy_timeout(Duration::from_secs(3))
            .map_err(|error| {
                format!(
                    "设置 SQLite 备份 busy_timeout 失败 ({}): {}",
                    db_path.display(),
                    error
                )
            })?;

        if backup_db_path.exists() {
            fs::remove_file(&backup_db_path).map_err(|error| {
                format!(
                    "删除旧 SQLite 备份失败 ({}): {}",
                    backup_db_path.display(),
                    error
                )
            })?;
        }
        let backup_target = backup_db_path.to_string_lossy().to_string();
        connection
            .execute("VACUUM main INTO ?1", [backup_target.as_str()])
            .map_err(|error| {
                format!(
                    "备份 SQLite 会话库失败 ({} -> {}): {}",
                    db_path.display(),
                    backup_db_path.display(),
                    error
                )
            })?;
        backed_up.push(relative.to_string_lossy().replace('\\', "/"));
    }
    Ok(backed_up)
}

fn restore_sqlite_databases_from_backup(
    data_dir: &Path,
    backup_dir: &Path,
) -> Result<Vec<String>, String> {
    let backup_db_root = backup_dir.join("db");
    if !backup_db_root.exists() {
        return Ok(Vec::new());
    }
    let backup_paths = list_backup_sqlite_files(&backup_db_root)?;
    let mut restored = Vec::new();
    for backup_db_path in backup_paths {
        let relative = backup_db_path
            .strip_prefix(&backup_db_root)
            .map_err(|_| format!("无法计算 SQLite 备份相对路径: {}", backup_db_path.display()))?;
        let target_db_path = data_dir.join(relative);
        if let Some(parent) = target_db_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("创建 SQLite 恢复目录失败 ({}): {}", parent.display(), error)
            })?;
        }
        remove_sqlite_sidecar_files(&target_db_path)?;
        fs::copy(&backup_db_path, &target_db_path).map_err(|error| {
            format!(
                "恢复 SQLite 会话库失败 ({} -> {}): {}",
                backup_db_path.display(),
                target_db_path.display(),
                error
            )
        })?;
        remove_sqlite_sidecar_files(&target_db_path)?;
        restored.push(relative.to_string_lossy().replace('\\', "/"));
    }
    Ok(restored)
}

fn list_backup_sqlite_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut result = Vec::new();
    let entries = fs::read_dir(root)
        .map_err(|error| format!("读取 SQLite 备份目录失败 ({}): {}", root.display(), error))?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!("读取 SQLite 备份目录项失败 ({}): {}", root.display(), error)
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "读取 SQLite 备份文件类型失败 ({}): {}",
                path.display(),
                error
            )
        })?;
        if file_type.is_dir() {
            result.extend(list_backup_sqlite_files(&path)?);
        } else if file_type.is_file() {
            result.push(path);
        }
    }
    result.sort();
    Ok(result)
}

fn backup_instance_files(
    data_dir: &Path,
    rollout_changes: &[RolloutProviderChange],
    include_sqlite: bool,
    include_session_index: bool,
    instance_id: &str,
    target_provider: &str,
) -> Result<PathBuf, String> {
    let backup_dir_name = format!(
        "{}{}{}",
        SESSION_VISIBILITY_REPAIR_BACKUP_PREFIX,
        Utc::now().format("%Y%m%d-%H%M%S"),
        SESSION_VISIBILITY_REPAIR_BACKUP_SUFFIX
    );
    let backup_dir = data_dir.join(backup_dir_name);
    fs::create_dir_all(&backup_dir)
        .map_err(|error| format!("创建备份目录失败 ({}): {}", backup_dir.display(), error))?;

    let mut backed_up_files = Vec::new();
    let mut sqlite_backup_files = Vec::new();
    for change in rollout_changes {
        let target = backup_dir.join("files").join(&change.relative_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "创建 rollout 备份目录失败 ({}): {}",
                    parent.display(),
                    error
                )
            })?;
        }
        fs::copy(&change.absolute_path, &target).map_err(|error| {
            format!(
                "备份 rollout 文件失败 ({} -> {}): {}",
                change.absolute_path.display(),
                target.display(),
                error
            )
        })?;
        modules::codex_session_file_time::restore_modified_time(
            &target,
            modules::codex_session_file_time::read_modified_time(&change.absolute_path),
        )?;
        backed_up_files.push(change.relative_path.to_string_lossy().to_string());
    }

    if include_sqlite {
        sqlite_backup_files = backup_sqlite_databases(data_dir, &backup_dir)?;
    }

    let mut session_index_backup_created = false;
    if include_session_index {
        let source = data_dir.join(SESSION_INDEX_FILE);
        if source.exists() {
            let target = backup_dir.join("files").join(SESSION_INDEX_FILE);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    format!(
                        "创建 session_index 备份目录失败 ({}): {}",
                        parent.display(),
                        error
                    )
                })?;
            }
            fs::copy(&source, &target).map_err(|error| {
                format!(
                    "备份 session_index.jsonl 失败 ({} -> {}): {}",
                    source.display(),
                    target.display(),
                    error
                )
            })?;
            session_index_backup_created = true;
        }
    }

    let manifest = json!({
        "instanceId": instance_id,
        "instanceRoot": data_dir,
        "targetProvider": target_provider,
        "createdAt": Utc::now().to_rfc3339(),
        "hasSqliteBackup": !sqlite_backup_files.is_empty(),
        "sqliteFiles": sqlite_backup_files,
        "hasSessionIndexBackup": session_index_backup_created,
        "rolloutFiles": backed_up_files,
    });
    fs::write(
        backup_dir.join("manifest.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&manifest)
                .map_err(|error| format!("序列化可见性修复备份清单失败: {}", error))?
        ),
    )
    .map_err(|error| {
        format!(
            "写入可见性修复备份清单失败 ({}): {}",
            backup_dir.display(),
            error
        )
    })?;

    Ok(backup_dir)
}

fn parse_session_visibility_repair_backup_timestamp(name: &str) -> Option<&str> {
    let timestamp = name
        .strip_prefix(SESSION_VISIBILITY_REPAIR_BACKUP_PREFIX)?
        .strip_suffix(SESSION_VISIBILITY_REPAIR_BACKUP_SUFFIX)?;
    if timestamp.len() != 15 {
        return None;
    }
    if !timestamp.chars().enumerate().all(|(index, value)| {
        if index == 8 {
            value == '-'
        } else {
            value.is_ascii_digit()
        }
    }) {
        return None;
    }
    Some(timestamp)
}

fn prune_session_visibility_repair_backups(instances: &[CodexSyncInstance]) {
    for instance in instances {
        if let Err(error) = prune_instance_session_visibility_repair_backups(&instance.data_dir) {
            modules::logger::log_warn(&format!(
                "清理 Codex 会话可见性修复旧备份失败 ({}): {}",
                instance.data_dir.display(),
                error
            ));
        }
    }
}

fn prune_instance_session_visibility_repair_backups(data_dir: &Path) -> Result<(), String> {
    let entries = match fs::read_dir(data_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "读取实例目录失败 ({}): {}",
                data_dir.display(),
                error
            ));
        }
    };
    let mut backups: Vec<(String, PathBuf)> = Vec::new();

    for entry in entries {
        let entry = entry
            .map_err(|error| format!("读取实例目录项失败 ({}): {}", data_dir.display(), error))?;
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "读取实例目录项类型失败 ({}): {}",
                entry.path().display(),
                error
            )
        })?;
        if !file_type.is_dir() {
            continue;
        }

        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        let Some(timestamp) = parse_session_visibility_repair_backup_timestamp(file_name) else {
            continue;
        };
        backups.push((timestamp.to_string(), entry.path()));
    }

    if backups.len() <= MAX_SESSION_VISIBILITY_REPAIR_BACKUPS {
        return Ok(());
    }

    backups.sort_by(|left, right| right.0.cmp(&left.0));
    for (_, path) in backups
        .into_iter()
        .skip(MAX_SESSION_VISIBILITY_REPAIR_BACKUPS)
    {
        fs::remove_dir_all(&path)
            .map_err(|error| format!("删除旧备份失败 ({}): {}", path.display(), error))?;
    }

    Ok(())
}

fn restore_instance_files_from_backup(
    data_dir: &Path,
    backup_dir: &Path,
    include_sqlite: bool,
) -> Result<(), String> {
    let files_root = backup_dir.join("files");
    if files_root.exists() {
        restore_directory_contents(&files_root, data_dir)?;
    }

    if include_sqlite {
        let _ = restore_sqlite_databases_from_backup(data_dir, backup_dir)?;
    }

    Ok(())
}

fn restore_directory_contents(source_root: &Path, target_root: &Path) -> Result<(), String> {
    let entries = fs::read_dir(source_root)
        .map_err(|error| format!("读取备份目录失败 ({}): {}", source_root.display(), error))?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!("读取备份目录项失败 ({}): {}", source_root.display(), error)
        })?;
        let source_path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "读取备份文件类型失败 ({}): {}",
                source_path.display(),
                error
            )
        })?;
        let relative = source_path
            .strip_prefix(source_root)
            .map_err(|_| format!("无法计算备份相对路径: {}", source_path.display()))?;
        let target_path = target_root.join(relative);

        if file_type.is_dir() {
            fs::create_dir_all(&target_path).map_err(|error| {
                format!("创建恢复目录失败 ({}): {}", target_path.display(), error)
            })?;
            restore_directory_contents(&source_path, &target_path)?;
            continue;
        }

        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("创建恢复父目录失败 ({}): {}", parent.display(), error))?;
        }
        fs::copy(&source_path, &target_path).map_err(|error| {
            format!(
                "恢复备份文件失败 ({} -> {}): {}",
                source_path.display(),
                target_path.display(),
                error
            )
        })?;
        modules::codex_session_file_time::restore_modified_time(
            &target_path,
            modules::codex_session_file_time::read_modified_time(&source_path),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let base_dir =
            std::env::temp_dir().join(format!("{}-{}-{}", prefix, std::process::id(), unique));
        if base_dir.exists() {
            fs::remove_dir_all(&base_dir).expect("cleanup old temp dir");
        }
        fs::create_dir_all(&base_dir).expect("create temp dir");
        base_dir
    }

    #[test]
    fn rollout_repair_updates_provider_and_preserves_session_time() {
        let data_dir = make_temp_dir("codex-session-visibility-rollout-time-test");
        let rollout_dir = data_dir.join("sessions").join("2026").join("05").join("23");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let rollout_path = rollout_dir.join("rollout-test.jsonl");
        fs::write(
            &rollout_path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"model_provider\":\"old\"}}\n{\"type\":\"event\",\"timestamp\":\"2024-01-01T00:00:00Z\"}\n",
        )
        .expect("write rollout");
        fs::write(
            data_dir.join(SESSION_INDEX_FILE),
            "{\"id\":\"s1\",\"thread_name\":\"Test\",\"updated_at\":\"2024-02-03T04:05:06Z\"}\n",
        )
        .expect("write session index");
        let polluted_modified_at = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        fs::File::open(&rollout_path)
            .expect("open rollout")
            .set_modified(polluted_modified_at)
            .expect("set polluted rollout mtime");

        let changes =
            collect_rollout_provider_changes(&data_dir, "relay").expect("collect rollout changes");
        assert_eq!(changes.len(), 1);

        repair_single_instance(&data_dir, "relay", &changes, false, false, false)
            .expect("repair rollout");

        let content = fs::read_to_string(&rollout_path).expect("read repaired rollout");
        assert!(content.contains("\"model_provider\":\"relay\""));
        assert_eq!(
            fs::metadata(&rollout_path)
                .expect("rollout metadata")
                .modified()
                .expect("rollout mtime"),
            UNIX_EPOCH + Duration::from_secs(1_704_067_200)
        );
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn rollout_repair_updates_all_session_meta_records() {
        let data_dir = make_temp_dir("codex-session-visibility-rollout-all-meta-test");
        let rollout_dir = data_dir.join("sessions").join("2026").join("06").join("14");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let rollout_path = rollout_dir.join("rollout-test.jsonl");
        fs::write(
            &rollout_path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"model_provider\":\"old-a\"}}\n{\"type\":\"event\",\"timestamp\":\"2024-01-01T00:00:00Z\"}\n{\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"model_provider\":\"old-b\"}}\n",
        )
        .expect("write rollout");

        let changes =
            collect_rollout_provider_changes(&data_dir, "relay").expect("collect rollout changes");
        assert_eq!(changes.len(), 1);
        repair_single_instance(&data_dir, "relay", &changes, false, false, false)
            .expect("repair rollout");

        let content = fs::read_to_string(&rollout_path).expect("read repaired rollout");
        assert!(!content.contains("old-a"));
        assert!(!content.contains("old-b"));
        assert_eq!(content.matches("\"model_provider\":\"relay\"").count(), 2);

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn rollout_repair_restores_session_time_without_provider_change() {
        let data_dir = make_temp_dir("codex-session-visibility-mtime-only-test");
        let rollout_dir = data_dir.join("sessions").join("2026").join("05").join("23");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let rollout_path = rollout_dir.join("rollout-test.jsonl");
        let rollout_content =
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"model_provider\":\"relay\"}}\n{\"type\":\"event\",\"timestamp\":\"2024-01-01T00:00:00Z\"}\n";
        fs::write(&rollout_path, rollout_content).expect("write rollout");
        let polluted_modified_at = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        fs::File::open(&rollout_path)
            .expect("open rollout")
            .set_modified(polluted_modified_at)
            .expect("set polluted rollout mtime");

        let changes =
            collect_rollout_provider_changes(&data_dir, "relay").expect("collect rollout changes");
        assert_eq!(changes.len(), 1);
        assert!(changes[0].updated_content.is_none());

        repair_single_instance(&data_dir, "relay", &changes, false, false, false)
            .expect("repair rollout time");

        assert_eq!(
            fs::read_to_string(&rollout_path).expect("read repaired rollout"),
            rollout_content
        );
        assert_eq!(
            fs::metadata(&rollout_path)
                .expect("rollout metadata")
                .modified()
                .expect("rollout mtime"),
            UNIX_EPOCH + Duration::from_secs(1_704_067_200)
        );
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sqlite_repair_updates_codex_dev_db_from_rollout_facts() {
        let data_dir = make_temp_dir("codex-session-visibility-sqlite-dir-test");
        let rollout_dir = data_dir.join("sessions").join("2026").join("06").join("14");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        fs::write(
            rollout_dir.join("rollout-thread-1.jsonl"),
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"old\",\"cwd\":\"\\\\\\\\?\\\\C:\\\\repo\"}}\n{\"type\":\"user_message\",\"timestamp\":\"2024-01-01T00:00:00Z\"}\n",
        )
        .expect("write rollout");

        let sqlite_dir = data_dir.join(SQLITE_DIR_NAME);
        fs::create_dir_all(&sqlite_dir).expect("create sqlite dir");
        let db_path = sqlite_dir.join(PREFERRED_SQLITE_DB_FILE);
        let connection = Connection::open(&db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    model_provider TEXT,
                    has_user_event INTEGER,
                    cwd TEXT
                )",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (id, model_provider, has_user_event, cwd)
                 VALUES ('thread-1', 'old', 0, '')",
                [],
            )
            .expect("insert row");
        drop(connection);

        let scan = count_sqlite_rows_to_update(&data_dir, "relay").expect("scan sqlite");
        assert_eq!(scan.rows_to_update, 3);

        let updated_rows = update_sqlite_provider(&data_dir, "relay").expect("update sqlite");
        assert_eq!(updated_rows, 3);

        let connection = Connection::open(&db_path).expect("reopen sqlite");
        let row = connection
            .query_row(
                "SELECT model_provider, has_user_event, cwd FROM threads WHERE id = 'thread-1'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .expect("read row");
        assert_eq!(row, ("relay".to_string(), 1, "C:/repo".to_string()));

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn session_index_repair_prefers_codex_dev_db_over_legacy_state_db() {
        let data_dir = make_temp_dir("codex-session-visibility-sqlite-preferred-test");
        let sqlite_dir = data_dir.join(SQLITE_DIR_NAME);
        fs::create_dir_all(&sqlite_dir).expect("create sqlite dir");
        let preferred_db = sqlite_dir.join(PREFERRED_SQLITE_DB_FILE);
        let legacy_db = data_dir.join(STATE_DB_FILE);
        for (db_path, title) in [
            (&preferred_db, "Current title"),
            (&legacy_db, "Legacy title"),
        ] {
            let connection = Connection::open(db_path).expect("open sqlite");
            connection
                .execute(
                    "CREATE TABLE threads (
                        id TEXT PRIMARY KEY,
                        title TEXT,
                        updated_at INTEGER
                    )",
                    [],
                )
                .expect("create threads table");
            connection
                .execute(
                    "INSERT INTO threads (id, title, updated_at) VALUES ('thread-1', ?1, 1_704_067_200)",
                    [title],
                )
                .expect("insert row");
        }

        let result = reconcile_session_index_from_sqlite(&data_dir).expect("reconcile index");
        assert_eq!(result.added_entries, 1);

        let index_map = read_session_index_map(&data_dir).expect("read session index");
        assert_eq!(
            index_map
                .get("thread-1")
                .and_then(|entry| entry.get("thread_name"))
                .and_then(JsonValue::as_str),
            Some("Current title")
        );

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sqlite_repair_marks_threads_with_first_user_message_visible() {
        let data_dir = make_temp_dir("codex-session-visibility-sqlite-test");
        let db_path = data_dir.join(STATE_DB_FILE);
        let connection = Connection::open(&db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    model_provider TEXT,
                    has_user_event INTEGER,
                    first_user_message TEXT,
                    thread_source TEXT
                )",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (id, model_provider, has_user_event, first_user_message, thread_source)
                 VALUES
                 ('matched-invisible', 'relay', 0, 'hello', ''),
                 ('old-invisible', 'old', 0, 'hi', NULL),
                 ('already-visible', 'relay', 1, 'visible', 'user'),
                 ('provider-only', '', 0, '', NULL)",
                [],
            )
            .expect("insert rows");
        drop(connection);

        let scan = count_sqlite_rows_to_update(&data_dir, "relay").expect("scan sqlite");
        assert_eq!(scan.rows_to_update, 3);
        assert!(!scan.skipped_unusable_database);

        let updated_rows = update_sqlite_provider(&data_dir, "relay").expect("update sqlite");
        assert_eq!(updated_rows, 3);

        let connection = Connection::open(&db_path).expect("reopen sqlite");
        let matched_invisible = connection
            .query_row(
                "SELECT model_provider, has_user_event, thread_source FROM threads WHERE id = 'matched-invisible'",
                [],
                |row| {
                    Ok((
                        row.get::<usize, String>(0)?,
                        row.get::<usize, i64>(1)?,
                        row.get::<usize, String>(2)?,
                    ))
                },
            )
            .expect("read matched row");
        assert_eq!(
            matched_invisible,
            ("relay".to_string(), 1, "user".to_string())
        );

        let old_invisible = connection
            .query_row(
                "SELECT model_provider, has_user_event, thread_source FROM threads WHERE id = 'old-invisible'",
                [],
                |row| {
                    Ok((
                        row.get::<usize, String>(0)?,
                        row.get::<usize, i64>(1)?,
                        row.get::<usize, String>(2)?,
                    ))
                },
            )
            .expect("read old row");
        assert_eq!(old_invisible, ("relay".to_string(), 1, "user".to_string()));

        let provider_only = connection
            .query_row(
                "SELECT model_provider, has_user_event FROM threads WHERE id = 'provider-only'",
                [],
                |row| Ok((row.get::<usize, String>(0)?, row.get::<usize, i64>(1)?)),
            )
            .expect("read provider-only row");
        assert_eq!(provider_only, ("relay".to_string(), 0));

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sqlite_repair_keeps_provider_only_schema_working() {
        let data_dir = make_temp_dir("codex-session-provider-only-sqlite-test");
        let db_path = data_dir.join(STATE_DB_FILE);
        let connection = Connection::open(&db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT)",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (id, model_provider) VALUES ('old', 'old'), ('same', 'relay')",
                [],
            )
            .expect("insert rows");
        drop(connection);

        let scan = count_sqlite_rows_to_update(&data_dir, "relay").expect("scan sqlite");
        assert_eq!(scan.rows_to_update, 1);
        let updated_rows = update_sqlite_provider(&data_dir, "relay").expect("update sqlite");
        assert_eq!(updated_rows, 1);

        let connection = Connection::open(&db_path).expect("reopen sqlite");
        let old_provider = connection
            .query_row(
                "SELECT model_provider FROM threads WHERE id = 'old'",
                [],
                |row| row.get::<usize, String>(0),
            )
            .expect("read old provider");
        assert_eq!(old_provider, "relay");

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sqlite_backup_restore_replaces_db_and_clears_sidecars() {
        let data_dir = make_temp_dir("codex-session-visibility-sqlite-backup-test");
        let db_path = data_dir.join(STATE_DB_FILE);
        let connection = Connection::open(&db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT)",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (id, model_provider) VALUES ('thread-1', 'old')",
                [],
            )
            .expect("insert old row");
        drop(connection);

        let backup_dir = backup_instance_files(&data_dir, &[], true, false, "default", "relay")
            .expect("backup db");

        let connection = Connection::open(&db_path).expect("reopen sqlite");
        connection
            .execute(
                "UPDATE threads SET model_provider = 'new' WHERE id = 'thread-1'",
                [],
            )
            .expect("mutate db after backup");
        drop(connection);
        for path in sqlite_sidecar_paths(&db_path) {
            fs::write(path, b"stale wal/shm").expect("write stale sidecar");
        }

        restore_instance_files_from_backup(&data_dir, &backup_dir, true).expect("restore db");
        for path in sqlite_sidecar_paths(&db_path) {
            assert!(
                !path.exists(),
                "stale sidecar should be removed: {:?}",
                path
            );
        }

        let connection = Connection::open(&db_path).expect("open restored sqlite");
        let provider = connection
            .query_row(
                "SELECT model_provider FROM threads WHERE id = 'thread-1'",
                [],
                |row| row.get::<usize, String>(0),
            )
            .expect("read restored provider");
        assert_eq!(provider, "old");

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn resolve_target_modified_prefers_rollout_activity_when_index_drifts() {
        let data_dir = make_temp_dir("codex-session-visibility-index-drift-test");
        let rollout_dir = data_dir.join("sessions").join("2026").join("06").join("08");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let rollout_path = rollout_dir.join("rollout-test.jsonl");
        fs::write(
            &rollout_path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"model_provider\":\"relay\"}}\n{\"type\":\"event\",\"timestamp\":\"2024-01-01T00:00:00Z\"}\n",
        )
        .expect("write rollout");
        fs::write(
            data_dir.join(SESSION_INDEX_FILE),
            "{\"id\":\"s1\",\"thread_name\":\"Test\",\"updated_at\":\"2026-03-16T23:36:58.7406859Z\"}\n",
        )
        .expect("write session index");

        let session_index_map = read_session_index_map(&data_dir).expect("read session index");
        let target =
            resolve_target_modified_at_ms(Some("s1"), &session_index_map, &rollout_path, None)
                .expect("resolve target modified");

        assert_eq!(target, 1_704_067_200_000);
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sqlite_timestamp_repair_syncs_from_rollout_activity() {
        let data_dir = make_temp_dir("codex-session-visibility-sqlite-time-test");
        let rollout_dir = data_dir.join("sessions").join("2026").join("06").join("08");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let rollout_path = rollout_dir.join("rollout-test.jsonl");
        fs::write(
            &rollout_path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"relay\"}}\n{\"type\":\"event\",\"timestamp\":\"2024-02-03T04:05:06Z\"}\n",
        )
        .expect("write rollout");
        let rollout_path_string = rollout_path.to_string_lossy().to_string();

        let db_path = data_dir.join(STATE_DB_FILE);
        let connection = Connection::open(&db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT,
                    updated_at INTEGER,
                    updated_at_ms INTEGER
                )",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (id, rollout_path, updated_at, updated_at_ms) VALUES
                 ('thread-1', ?1, 1_800_000_000, 1_800_000_000_000)",
                [rollout_path_string.as_str()],
            )
            .expect("insert row");
        drop(connection);

        assert_eq!(
            count_sqlite_thread_timestamps_to_update(&data_dir)
                .expect("count sqlite timestamp repairs"),
            1
        );

        let updated = repair_sqlite_thread_timestamps(&data_dir).expect("repair sqlite timestamps");
        assert_eq!(updated, 1);

        let connection = Connection::open(&db_path).expect("reopen sqlite");
        let (updated_at, updated_at_ms) = connection
            .query_row(
                "SELECT updated_at, updated_at_ms FROM threads WHERE id = 'thread-1'",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .expect("read repaired timestamps");
        assert_eq!(updated_at, 1_706_933_106);
        assert_eq!(updated_at_ms, 1_706_933_106_000);

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn session_index_repair_appends_missing_sqlite_threads() {
        let data_dir = make_temp_dir("codex-session-visibility-index-test");
        let db_path = data_dir.join(STATE_DB_FILE);
        let connection = Connection::open(&db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    title TEXT,
                    updated_at INTEGER
                )",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (id, title, updated_at) VALUES
                 ('indexed-thread', 'Indexed', 1_704_067_200),
                 ('missing-thread', 'Missing chat', 1_800_000_000)",
                [],
            )
            .expect("insert rows");
        drop(connection);

        fs::write(
            data_dir.join(SESSION_INDEX_FILE),
            "{\"id\":\"indexed-thread\",\"thread_name\":\"Indexed\",\"updated_at\":\"2024-01-01T00:00:00.0000000Z\"}\n",
        )
        .expect("write session index");

        let missing =
            count_missing_session_index_entries(&data_dir).expect("count missing index entries");
        assert_eq!(missing, 1);

        let result = reconcile_session_index_from_sqlite(&data_dir).expect("reconcile index");
        assert_eq!(result.added_entries, 1);
        assert_eq!(result.updated_entries, 0);

        let index_map = read_session_index_map(&data_dir).expect("read session index");
        assert!(index_map.contains_key("missing-thread"));
        assert_eq!(
            index_map
                .get("missing-thread")
                .and_then(|entry| entry.get("thread_name"))
                .and_then(JsonValue::as_str),
            Some("Missing chat")
        );
        assert_eq!(
            count_missing_session_index_entries(&data_dir).expect("recount missing index entries"),
            0
        );

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn session_index_repair_updates_stale_entries_from_rollout_activity() {
        let data_dir = make_temp_dir("codex-session-visibility-index-update-test");
        let rollout_dir = data_dir.join("sessions").join("2026").join("06").join("12");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let rollout_path = rollout_dir.join("rollout-test.jsonl");
        fs::write(
            &rollout_path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"relay\"}}\n{\"type\":\"event\",\"timestamp\":\"2024-02-03T04:05:06.789Z\"}\n",
        )
        .expect("write rollout");
        let rollout_path_string = rollout_path.to_string_lossy().to_string();

        let db_path = data_dir.join(STATE_DB_FILE);
        let connection = Connection::open(&db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    title TEXT,
                    updated_at INTEGER,
                    updated_at_ms INTEGER,
                    rollout_path TEXT
                )",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (id, title, updated_at, updated_at_ms, rollout_path)
                 VALUES ('thread-1', 'Fresh title', 1_800_000_000, 1_800_000_000_000, ?1)",
                [rollout_path_string.as_str()],
            )
            .expect("insert row");
        drop(connection);

        fs::write(
            data_dir.join(SESSION_INDEX_FILE),
            "{\"id\":\"thread-1\",\"thread_name\":\"Old title\",\"updated_at\":\"2024-01-01T00:00:00.000000Z\",\"pinned\":true}\n",
        )
        .expect("write stale session index");

        let scan = count_session_index_entries_to_repair(&data_dir).expect("count index repairs");
        assert_eq!(scan.entries_to_add, 0);
        assert_eq!(scan.entries_to_update, 1);

        let result = reconcile_session_index_from_sqlite(&data_dir).expect("reconcile index");
        assert_eq!(result.added_entries, 0);
        assert_eq!(result.updated_entries, 1);

        let index_map = read_session_index_map(&data_dir).expect("read session index");
        let entry = index_map.get("thread-1").expect("read updated entry");
        assert_eq!(
            entry.get("thread_name").and_then(JsonValue::as_str),
            Some("Fresh title")
        );
        assert_eq!(entry.get("pinned").and_then(JsonValue::as_bool), Some(true));
        assert_eq!(
            parse_session_index_updated_at_ms(entry),
            Some(1_706_933_106_789)
        );

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }
}
