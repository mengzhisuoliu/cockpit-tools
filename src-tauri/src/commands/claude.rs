use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use crate::models::claude::{
    ClaudeAccount, ClaudeAuthMode, ClaudeDesktopLoginStartResponse, ClaudeOAuthStartResponse,
};
use crate::modules::{claude_account, logger};

#[cfg(not(target_os = "windows"))]
fn posix_shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    let needs_quote = value.chars().any(|ch| {
        ch.is_whitespace()
            || matches!(
                ch,
                '\'' | '"' | '$' | '`' | '\\' | '&' | '|' | ';' | '<' | '>' | '(' | ')'
            )
    });
    if !needs_quote {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(target_os = "windows")]
fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(target_os = "macos")]
fn escape_applescript(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn normalize_cli_working_dir(working_dir: &str) -> Result<String, String> {
    let trimmed = working_dir.trim();
    if trimmed.is_empty() {
        return Err("请选择 Claude CLI 工作目录".to_string());
    }
    let path = Path::new(trimmed);
    if !path.is_dir() {
        return Err(format!("Claude CLI 工作目录不存在: {}", trimmed));
    }
    Ok(trimmed.to_string())
}

fn temp_claude_cli_script_path(extension: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "agtools-claude-cli-{}-{}.{}",
        std::process::id(),
        now,
        extension
    ))
}

fn is_safe_env_key(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(ch) if ch == '_' || ch.is_ascii_uppercase() => {}
        _ => return false,
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_uppercase() || ch.is_ascii_digit())
}

#[cfg(not(target_os = "windows"))]
fn write_posix_env_script(working_dir: &str, env: &[(String, String)]) -> Result<PathBuf, String> {
    let script_path = temp_claude_cli_script_path("sh");
    let script_path_text = script_path.to_string_lossy();
    let mut script = String::from("#!/bin/sh\n");
    script.push_str(&format!(
        "rm -f -- {}\n",
        posix_shell_quote(&script_path_text)
    ));
    script.push_str(&format!(
        "cd {} || exit $?\n",
        posix_shell_quote(working_dir)
    ));
    for (key, value) in env {
        if !is_safe_env_key(key) {
            continue;
        }
        script.push_str(&format!("export {}={}\n", key, posix_shell_quote(value)));
    }
    script.push_str("exec claude\n");
    fs::write(&script_path, script).map_err(|e| format!("写入 Claude CLI 临时脚本失败: {}", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&script_path, fs::Permissions::from_mode(0o600));
    }
    Ok(script_path)
}

#[cfg(target_os = "windows")]
fn write_windows_env_script(
    working_dir: &str,
    env: &[(String, String)],
) -> Result<PathBuf, String> {
    let script_path = temp_claude_cli_script_path("ps1");
    let script_path_text = script_path.to_string_lossy();
    let mut script = String::new();
    script.push_str(&format!(
        "Set-Location -LiteralPath {}\n",
        powershell_quote(working_dir)
    ));
    for (key, value) in env {
        if !is_safe_env_key(key) {
            continue;
        }
        script.push_str(&format!("$env:{} = {}\n", key, powershell_quote(value)));
    }
    script.push_str("claude\n");
    script.push_str(&format!(
        "Remove-Item -LiteralPath {} -Force -ErrorAction SilentlyContinue\n",
        powershell_quote(&script_path_text)
    ));
    fs::write(&script_path, script).map_err(|e| format!("写入 Claude CLI 临时脚本失败: {}", e))?;
    Ok(script_path)
}

fn build_claude_cli_command(working_dir: &str, env: &[(String, String)]) -> Result<String, String> {
    let working_dir = normalize_cli_working_dir(working_dir)?;
    #[cfg(target_os = "windows")]
    {
        if !env.is_empty() {
            let script_path = write_windows_env_script(&working_dir, env)?;
            let script_path_text = script_path.to_string_lossy();
            return Ok(format!("& {}", powershell_quote(script_path_text.as_ref())));
        }
        return Ok(format!(
            "Set-Location -LiteralPath {}; claude",
            powershell_quote(&working_dir)
        ));
    }

    #[cfg(not(target_os = "windows"))]
    {
        if !env.is_empty() {
            let script_path = write_posix_env_script(&working_dir, env)?;
            let script_path_text = script_path.to_string_lossy();
            return Ok(format!(
                "sh {}",
                posix_shell_quote(script_path_text.as_ref())
            ));
        }
        return Ok(format!("cd {} && claude", posix_shell_quote(&working_dir)));
    }

    #[allow(unreachable_code)]
    Err("当前系统暂不支持生成 Claude CLI 启动命令".to_string())
}

fn execute_claude_cli_command(command: &str, terminal: Option<String>) -> Result<String, String> {
    let config = crate::modules::config::get_user_config();
    let terminal = terminal
        .unwrap_or(config.default_terminal)
        .trim()
        .to_string();

    #[cfg(target_os = "macos")]
    {
        let is_iterm = terminal.to_lowercase().contains("iterm");
        let is_terminal_app = terminal == "system" || terminal.is_empty() || terminal == "Terminal";
        let app_name = if is_terminal_app {
            "Terminal"
        } else {
            &terminal
        };

        let script = if is_iterm {
            format!(
                "tell application \"iTerm\"
                    activate
                    if not (exists window 1) then
                        create window with default profile
                        tell current session of current window
                            write text \"{}\"
                        end tell
                    else
                        tell current window
                            create tab with default profile
                            tell current session
                                write text \"{}\"
                            end tell
                        end tell
                    end if
                end tell",
                escape_applescript(command),
                escape_applescript(command)
            )
        } else if is_terminal_app {
            format!(
                "tell application \"Terminal\"
                    activate
                    do script \"{}\"
                end tell",
                escape_applescript(command)
            )
        } else {
            return Err(format!(
                "当前终端暂不支持直接执行：{}。请改用 Terminal 或 iTerm2。",
                terminal
            ));
        };

        let output = Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| format!("打开终端失败 ({}): {}", app_name, e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("终端执行失败: {}", stderr.trim()));
        }
        return Ok(format!("已在 {} 执行 Claude CLI 命令", app_name));
    }

    #[cfg(target_os = "windows")]
    {
        let terminal_key = terminal.to_ascii_lowercase();
        let shell = if terminal_key == "pwsh" {
            "pwsh"
        } else {
            "powershell"
        };
        let mut cmd = if terminal_key == "wt" {
            let mut command_process = Command::new("wt");
            command_process.args([
                shell,
                "-NoExit",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                command,
            ]);
            command_process
        } else {
            let mut command_process = Command::new("cmd");
            command_process.args([
                "/C",
                "start",
                "",
                shell,
                "-NoExit",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                command,
            ]);
            command_process
        };

        cmd.spawn().map_err(|e| format!("打开终端失败: {}", e))?;
        return Ok("已打开 Claude CLI 终端窗口".to_string());
    }

    #[cfg(target_os = "linux")]
    {
        let shell_command = format!("{}; exec bash", command);
        let mut cmd = if terminal == "system" || terminal.is_empty() {
            Command::new("x-terminal-emulator")
        } else {
            Command::new(&terminal)
        };

        cmd.args(["-e", "bash", "-lc", &shell_command])
            .spawn()
            .or_else(|_| {
                if terminal == "system" || terminal.is_empty() {
                    Command::new("gnome-terminal")
                        .args(["--", "bash", "-lc", &shell_command])
                        .spawn()
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "指定终端未找到",
                    ))
                }
            })
            .or_else(|_| {
                if terminal == "system" || terminal.is_empty() {
                    Command::new("konsole")
                        .args(["-e", "bash", "-lc", &shell_command])
                        .spawn()
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "指定终端未找到",
                    ))
                }
            })
            .or_else(|_| Command::new("sh").args(["-lc", command]).spawn())
            .map_err(|e| format!("执行 Claude CLI 命令失败: {}", e))?;
        return Ok("已执行 Claude CLI 命令".to_string());
    }

    #[allow(unreachable_code)]
    Err("Claude CLI 终端执行仅支持 macOS、Windows 和 Linux".to_string())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeCliLaunchInfo {
    pub account_id: String,
    pub account_email: String,
    pub working_dir: String,
    pub launch_command: String,
}

fn prepare_claude_cli_launch(
    account_id: &str,
    working_dir: &str,
) -> Result<(ClaudeAccount, String, String), String> {
    let account = claude_account::load_account(account_id)
        .ok_or_else(|| format!("Claude account not found: {}", account_id))?;
    if account.auth_mode == ClaudeAuthMode::DesktopOAuth {
        return Err(
            "Claude Desktop 登录态不能启动 Claude Code CLI，请使用 OAuth / Setup Token 账号。"
                .to_string(),
        );
    }
    let normalized_working_dir = normalize_cli_working_dir(working_dir)?;
    let cli_env = claude_account::build_api_key_cli_env(&account)?;
    let command = build_claude_cli_command(&normalized_working_dir, &cli_env)?;
    if account.auth_mode != ClaudeAuthMode::ApiKey {
        claude_account::inject_to_claude_config(account_id, None)?;
    }
    crate::modules::provider_current_state::set_current_account_id("claude_cli", Some(account_id))?;
    Ok((account, normalized_working_dir, command))
}

#[tauri::command]
pub fn list_claude_accounts() -> Result<Vec<ClaudeAccount>, String> {
    claude_account::list_accounts_checked()
}

#[tauri::command]
pub fn delete_claude_account(account_id: String) -> Result<(), String> {
    claude_account::remove_account(&account_id)
}

#[tauri::command]
pub fn delete_claude_accounts(account_ids: Vec<String>) -> Result<(), String> {
    claude_account::remove_accounts(&account_ids)
}

#[tauri::command]
pub async fn import_claude_from_json(
    app: AppHandle,
    json_content: String,
) -> Result<Vec<ClaudeAccount>, String> {
    let accounts = claude_account::import_from_json(&json_content)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(accounts)
}

#[tauri::command]
pub async fn import_claude_api_key(
    app: AppHandle,
    api_key: String,
    account_name: Option<String>,
    api_base_url: Option<String>,
    api_provider_id: Option<String>,
    api_provider_name: Option<String>,
    api_provider_source_tag: Option<String>,
    api_provider_website: Option<String>,
    api_provider_api_key_url: Option<String>,
    api_key_field: Option<String>,
    api_model_catalog: Option<Vec<String>>,
    api_extra_env: Option<BTreeMap<String, String>>,
) -> Result<ClaudeAccount, String> {
    let account = claude_account::import_api_key(
        &api_key,
        account_name.as_deref(),
        claude_account::ClaudeApiKeyProviderConfig {
            api_base_url,
            api_provider_id,
            api_provider_name,
            api_provider_source_tag,
            api_provider_website,
            api_provider_api_key_url,
            api_key_field,
            api_model_catalog,
            api_extra_env,
        },
    )?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub fn claude_oauth_login_prepare() -> Result<ClaudeOAuthStartResponse, String> {
    claude_account::start_oauth_login()
}

#[tauri::command]
pub async fn claude_oauth_login_start(app: AppHandle) -> Result<ClaudeOAuthStartResponse, String> {
    let response = claude_account::start_oauth_login()?;
    if let Err(error) = app
        .opener()
        .open_url(&response.verification_uri, None::<String>)
    {
        let _ = claude_account::cancel_oauth_login(Some(response.login_id.as_str()));
        return Err(format!("打开 Claude OAuth 授权页失败: {}", error));
    }
    Ok(response)
}

#[tauri::command]
pub async fn claude_oauth_login_complete(
    app: AppHandle,
    login_id: String,
    callback_or_code: String,
    email_hint: Option<String>,
) -> Result<ClaudeAccount, String> {
    let account =
        claude_account::complete_oauth_login(&login_id, &callback_or_code, email_hint.as_deref())
            .await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub fn claude_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    claude_account::cancel_oauth_login(login_id.as_deref())
}

#[tauri::command]
pub async fn import_claude_desktop_from_local(
    app: AppHandle,
    account_name: Option<String>,
) -> Result<ClaudeAccount, String> {
    let account = claude_account::import_desktop_from_local(account_name.as_deref())?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub async fn import_claude_cli_from_local(app: AppHandle) -> Result<ClaudeAccount, String> {
    let account = claude_account::import_cli_from_local()?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub async fn claude_desktop_login_start(
    _app: AppHandle,
) -> Result<ClaudeDesktopLoginStartResponse, String> {
    claude_account::start_desktop_login()
}

#[tauri::command]
pub async fn claude_desktop_login_complete(
    app: AppHandle,
    login_id: String,
    account_name: Option<String>,
) -> Result<ClaudeAccount, String> {
    let account = claude_account::complete_desktop_login(&login_id, account_name.as_deref())?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub fn claude_desktop_login_cancel(login_id: Option<String>) -> Result<(), String> {
    claude_account::cancel_desktop_login(login_id.as_deref())
}

#[tauri::command]
pub fn export_claude_accounts(account_ids: Vec<String>) -> Result<String, String> {
    claude_account::export_accounts(&account_ids)
}

#[tauri::command]
pub async fn refresh_claude_quota(
    app: AppHandle,
    account_id: String,
) -> Result<ClaudeAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Claude Command] 手动刷新账号开始: account_id={}",
        account_id
    ));

    let account = claude_account::refresh_account_quota(&account_id).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Claude Command] 刷新完成: account_id={}, email={}, elapsed={}ms",
        account.id,
        account.email,
        started_at.elapsed().as_millis()
    ));
    Ok(account)
}

#[tauri::command]
pub async fn refresh_all_claude_quotas(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[Claude Command] 批量刷新开始");
    let results = claude_account::refresh_all_quotas().await?;
    let success_count = results.iter().filter(|(_, item)| item.is_ok()).count();
    let failed_count = results.len().saturating_sub(success_count);
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Claude Command] 批量刷新完成: success={}, failed={}, elapsed={}ms",
        success_count,
        failed_count,
        started_at.elapsed().as_millis()
    ));
    Ok(success_count as i32)
}

#[tauri::command]
pub fn update_claude_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<ClaudeAccount, String> {
    claude_account::update_account_tags(&account_id, tags)
}

#[tauri::command]
pub fn update_claude_account_plan(
    account_id: String,
    plan_type: Option<String>,
) -> Result<ClaudeAccount, String> {
    claude_account::update_account_plan(&account_id, plan_type.as_deref())
}

#[tauri::command]
pub fn update_claude_account_note(
    account_id: String,
    note: Option<String>,
) -> Result<ClaudeAccount, String> {
    claude_account::update_account_note(&account_id, note.as_deref())
}

#[tauri::command]
pub fn get_claude_accounts_index_path() -> Result<String, String> {
    claude_account::accounts_index_path_string()
}

#[tauri::command]
pub fn claude_get_cli_launch_command(
    app: AppHandle,
    account_id: String,
    working_dir: String,
) -> Result<ClaudeCliLaunchInfo, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Claude CLI] 准备启动命令: account_id={}, working_dir={}",
        account_id, working_dir
    ));

    let (account, normalized_working_dir, command) =
        prepare_claude_cli_launch(&account_id, &working_dir)?;
    let _ = crate::modules::tray::update_tray_menu(&app);

    logger::log_info(&format!(
        "[Claude CLI] 启动命令已准备: account_id={}, email={}, elapsed={}ms",
        account.id,
        account.email,
        started_at.elapsed().as_millis()
    ));

    Ok(ClaudeCliLaunchInfo {
        account_id: account.id,
        account_email: account.email,
        working_dir: normalized_working_dir,
        launch_command: command,
    })
}

#[tauri::command]
pub fn claude_execute_cli_launch_command(
    app: AppHandle,
    account_id: String,
    working_dir: String,
    terminal: Option<String>,
) -> Result<String, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Claude CLI] 开始终端执行: account_id={}, working_dir={}",
        account_id, working_dir
    ));

    let (account, _normalized_working_dir, command) =
        prepare_claude_cli_launch(&account_id, &working_dir)?;
    let result = execute_claude_cli_command(&command, terminal)?;
    let _ = crate::modules::tray::update_tray_menu(&app);

    logger::log_info(&format!(
        "[Claude CLI] 终端执行完成: account_id={}, email={}, elapsed={}ms",
        account.id,
        account.email,
        started_at.elapsed().as_millis()
    ));
    Ok(result)
}

#[tauri::command]
pub fn claude_launch_cli(
    app: AppHandle,
    account_id: String,
    working_dir: String,
    terminal: Option<String>,
) -> Result<String, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Claude CLI] 开始启动: account_id={}, working_dir={}",
        account_id, working_dir
    ));

    let (account, _normalized_working_dir, command) =
        prepare_claude_cli_launch(&account_id, &working_dir)?;
    let result = execute_claude_cli_command(&command, terminal)?;
    let _ = crate::modules::tray::update_tray_menu(&app);

    logger::log_info(&format!(
        "[Claude CLI] 启动完成: account_id={}, email={}, elapsed={}ms",
        account.id,
        account.email,
        started_at.elapsed().as_millis()
    ));
    Ok(result)
}

#[tauri::command]
pub fn switch_claude_account(app: AppHandle, account_id: String) -> Result<String, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Claude Switch] 开始切换账号: account_id={}",
        account_id
    ));

    let account = claude_account::load_account(&account_id)
        .ok_or_else(|| format!("Claude account not found: {}", account_id))?;
    claude_account::inject_to_claude(&account_id)?;
    crate::modules::provider_current_state::set_current_account_id(
        "claude",
        Some(account_id.as_str()),
    )?;
    let _ = crate::modules::tray::update_tray_menu(&app);

    logger::log_info(&format!(
        "[Claude Switch] 切号成功: account_id={}, email={}, elapsed={}ms",
        account.id,
        account.email,
        started_at.elapsed().as_millis()
    ));
    Ok(format!("切换完成: {}", account.email))
}
