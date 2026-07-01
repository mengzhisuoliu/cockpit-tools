use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::models::windsurf::{WindsurfAccount, WindsurfOAuthStartResponse};
use crate::modules::{logger, platform_adapter, platform_package};

const WINDSURF_FAST_LOCAL_MUTATION_TIMEOUT: Duration = Duration::from_secs(20);

fn ensure_windsurf_package_installed() -> Result<(), String> {
    platform_package::ensure_platform_package_installed("windsurf")
}

fn windsurf_call<T: DeserializeOwned>(method: &str, payload: Value) -> Result<T, String> {
    ensure_windsurf_package_installed()?;
    platform_adapter::call_windsurf(method, payload)
}

async fn windsurf_call_async<T>(method: &'static str, payload: Value) -> Result<T, String>
where
    T: DeserializeOwned + Send + 'static,
{
    ensure_windsurf_package_installed()?;
    tauri::async_runtime::spawn_blocking(move || platform_adapter::call_windsurf(method, payload))
        .await
        .map_err(|error| format!("Windsurf adapter 任务失败: {}", error))?
}

async fn windsurf_call_async_with_timeout<T>(
    method: &'static str,
    payload: Value,
    timeout: Duration,
) -> Result<T, String>
where
    T: DeserializeOwned + Send + 'static,
{
    ensure_windsurf_package_installed()?;
    tauri::async_runtime::spawn_blocking(move || {
        platform_adapter::call_windsurf_with_timeout(method, payload, timeout)
    })
    .await
    .map_err(|error| format!("Windsurf adapter 任务失败: {}", error))?
}

fn update_tray_menu_in_background(app: AppHandle) {
    tauri::async_runtime::spawn_blocking(move || {
        let _ = crate::modules::tray::update_tray_menu(&app);
    });
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WindsurfPasswordCredentialInput {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub source_line: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WindsurfPasswordCredentialFailure {
    pub email: String,
    pub error: String,
    pub source_line: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WindsurfPasswordBatchResult {
    pub accounts: Vec<WindsurfAccount>,
    pub success_count: usize,
    pub failed_count: usize,
    pub failures: Vec<WindsurfPasswordCredentialFailure>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SwitchResult {
    message: String,
    #[serde(default)]
    restart_error: Option<String>,
    path_missing: bool,
}

fn emit_windsurf_path_missing(app: &AppHandle, retry: Value) {
    let _ = app.emit(
        "app:path_missing",
        json!({
            "app": "windsurf",
            "retry": retry
        }),
    );
}

#[tauri::command]
pub async fn list_windsurf_accounts() -> Result<Vec<WindsurfAccount>, String> {
    windsurf_call_async_with_timeout(
        "accounts.list",
        json!({}),
        WINDSURF_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await
}

#[tauri::command]
pub async fn delete_windsurf_account(app: AppHandle, account_id: String) -> Result<(), String> {
    windsurf_call_async_with_timeout::<()>(
        "accounts.delete",
        json!({ "accountId": account_id }),
        WINDSURF_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(())
}

#[tauri::command]
pub async fn delete_windsurf_accounts(
    app: AppHandle,
    account_ids: Vec<String>,
) -> Result<(), String> {
    windsurf_call_async_with_timeout::<()>(
        "accounts.deleteMany",
        json!({ "accountIds": account_ids }),
        WINDSURF_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(())
}

#[tauri::command]
pub async fn import_windsurf_from_json(
    app: AppHandle,
    json_content: String,
) -> Result<Vec<WindsurfAccount>, String> {
    let accounts = windsurf_call_async_with_timeout(
        "accounts.importJson",
        json!({ "jsonContent": json_content }),
        WINDSURF_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(accounts)
}

#[tauri::command]
pub async fn import_windsurf_from_local(app: AppHandle) -> Result<Vec<WindsurfAccount>, String> {
    let accounts: Vec<WindsurfAccount> = windsurf_call_async_with_timeout(
        "accounts.importLocal",
        json!({}),
        WINDSURF_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(accounts)
}

#[tauri::command]
pub fn export_windsurf_accounts(account_ids: Vec<String>) -> Result<String, String> {
    windsurf_call("accounts.export", json!({ "accountIds": account_ids }))
}

#[tauri::command]
pub async fn refresh_windsurf_token(
    app: AppHandle,
    account_id: String,
) -> Result<WindsurfAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Windsurf Command] 手动刷新账号开始: account_id={}",
        account_id
    ));
    let account: WindsurfAccount =
        windsurf_call_async("accounts.refresh", json!({ "accountId": account_id })).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Windsurf Command] 手动刷新账号完成: account_id={}, elapsed={}ms",
        account.id,
        started_at.elapsed().as_millis()
    ));
    Ok(account)
}

#[tauri::command]
pub async fn refresh_all_windsurf_tokens(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[Windsurf Command] 手动批量刷新开始");
    let success_count: i32 = windsurf_call_async("accounts.refreshAll", json!({})).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Windsurf Command] 手动批量刷新完成: success={}, elapsed={}ms",
        success_count,
        started_at.elapsed().as_millis()
    ));
    Ok(success_count)
}

#[tauri::command]
pub async fn windsurf_oauth_login_start() -> Result<WindsurfOAuthStartResponse, String> {
    logger::log_info("Windsurf OAuth start 命令触发");
    windsurf_call_async("oauth.start", json!({})).await
}

#[tauri::command]
pub async fn windsurf_oauth_login_complete(
    app: AppHandle,
    login_id: String,
) -> Result<WindsurfAccount, String> {
    logger::log_info(&format!(
        "Windsurf OAuth complete 命令触发: login_id={}",
        login_id
    ));
    let account: WindsurfAccount =
        windsurf_call_async("oauth.complete", json!({ "loginId": login_id })).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "Windsurf OAuth complete 成功: account_id={}, login={}",
        account.id, account.github_login
    ));
    Ok(account)
}

#[tauri::command]
pub fn windsurf_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    logger::log_info(&format!(
        "Windsurf OAuth cancel 命令触发: login_id={}",
        login_id.as_deref().unwrap_or("<none>")
    ));
    windsurf_call("oauth.cancel", json!({ "loginId": login_id }))
}

#[tauri::command]
pub fn windsurf_oauth_submit_callback_url(
    login_id: String,
    callback_url: String,
) -> Result<(), String> {
    windsurf_call(
        "oauth.submitCallbackUrl",
        json!({ "loginId": login_id, "callbackUrl": callback_url }),
    )
}

#[tauri::command]
pub async fn add_windsurf_account_with_token(
    app: AppHandle,
    github_access_token: String,
) -> Result<WindsurfAccount, String> {
    let account: WindsurfAccount = windsurf_call_async_with_timeout(
        "accounts.addToken",
        json!({ "githubAccessToken": github_access_token }),
        WINDSURF_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(account)
}

#[tauri::command]
pub async fn add_windsurf_account_with_password(
    app: AppHandle,
    email: String,
    password: String,
) -> Result<WindsurfAccount, String> {
    logger::log_info("[Windsurf Command] 邮箱密码登录开始");
    let account: WindsurfAccount = windsurf_call_async(
        "accounts.addPassword",
        json!({ "email": email, "password": password }),
    )
    .await?;
    logger::log_info(&format!(
        "[Windsurf Command] 邮箱密码登录成功: account_id={}, login={}",
        account.id, account.github_login
    ));
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub async fn add_windsurf_accounts_with_password(
    app: AppHandle,
    credentials: Vec<WindsurfPasswordCredentialInput>,
) -> Result<WindsurfPasswordBatchResult, String> {
    logger::log_info(&format!(
        "[Windsurf Command] 批量邮箱密码登录开始: count={}",
        credentials.len()
    ));
    let result: WindsurfPasswordBatchResult = windsurf_call_async(
        "accounts.addPasswordBatch",
        json!({ "credentials": credentials }),
    )
    .await?;
    if !result.accounts.is_empty() {
        let _ = crate::modules::tray::update_tray_menu(&app);
    }
    Ok(result)
}

#[tauri::command]
pub async fn update_windsurf_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<WindsurfAccount, String> {
    windsurf_call(
        "accounts.updateTags",
        json!({ "accountId": account_id, "tags": tags }),
    )
}

#[tauri::command]
pub fn get_windsurf_accounts_index_path() -> Result<String, String> {
    windsurf_call("accounts.indexPath", json!({}))
}

#[tauri::command]
pub async fn inject_windsurf_to_vscode(
    app: AppHandle,
    account_id: String,
) -> Result<String, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Windsurf Switch] 开始切换账号: account_id={}",
        account_id
    ));

    let result: SwitchResult =
        windsurf_call_async("switch.inject", json!({ "accountId": account_id })).await?;
    let _ = crate::modules::provider_current_state::set_current_account_id(
        "windsurf",
        Some(account_id.as_str()),
    );
    let _ = crate::modules::tray::update_tray_menu(&app);

    if result.path_missing {
        emit_windsurf_path_missing(&app, json!({ "kind": "default" }));
        if let Some(error) = result.restart_error.as_deref() {
            logger::log_warn(&format!(
                "[Windsurf Switch] 切号完成但启动失败: err={}",
                error
            ));
        }
        return Ok(result.message);
    }

    logger::log_info(&format!(
        "[Windsurf Switch] 切号成功: elapsed={}ms",
        started_at.elapsed().as_millis()
    ));
    Ok(result.message)
}
