use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::models::cursor::CursorAccount;
use crate::modules::{logger, platform_adapter, platform_package};

const CURSOR_FAST_LOCAL_MUTATION_TIMEOUT: Duration = Duration::from_secs(20);

fn ensure_cursor_package_installed() -> Result<(), String> {
    platform_package::ensure_platform_package_installed("cursor")
}

fn cursor_call<T: DeserializeOwned>(method: &str, payload: Value) -> Result<T, String> {
    ensure_cursor_package_installed()?;
    platform_adapter::call_cursor(method, payload)
}

async fn cursor_call_async<T>(method: &'static str, payload: Value) -> Result<T, String>
where
    T: DeserializeOwned + Send + 'static,
{
    ensure_cursor_package_installed()?;
    tauri::async_runtime::spawn_blocking(move || platform_adapter::call_cursor(method, payload))
        .await
        .map_err(|error| format!("Cursor adapter 任务失败: {}", error))?
}

async fn cursor_call_async_with_timeout<T>(
    method: &'static str,
    payload: Value,
    timeout: Duration,
) -> Result<T, String>
where
    T: DeserializeOwned + Send + 'static,
{
    ensure_cursor_package_installed()?;
    tauri::async_runtime::spawn_blocking(move || {
        platform_adapter::call_cursor_with_timeout(method, payload, timeout)
    })
    .await
    .map_err(|error| format!("Cursor adapter 任务失败: {}", error))?
}

fn update_tray_menu_in_background(app: AppHandle) {
    tauri::async_runtime::spawn_blocking(move || {
        let _ = crate::modules::tray::update_tray_menu(&app);
    });
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SwitchResult {
    message: String,
    #[serde(default)]
    restart_error: Option<String>,
    path_missing: bool,
}

fn emit_cursor_path_missing(app: &AppHandle, retry: Value) {
    let _ = app.emit(
        "app:path_missing",
        json!({
            "app": "cursor",
            "retry": retry
        }),
    );
}

#[tauri::command]
pub async fn list_cursor_accounts() -> Result<Vec<CursorAccount>, String> {
    cursor_call_async_with_timeout(
        "accounts.list",
        json!({}),
        CURSOR_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await
}

#[tauri::command]
pub async fn delete_cursor_account(app: AppHandle, account_id: String) -> Result<(), String> {
    cursor_call_async_with_timeout::<()>(
        "accounts.delete",
        json!({ "accountId": account_id }),
        CURSOR_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(())
}

#[tauri::command]
pub async fn delete_cursor_accounts(
    app: AppHandle,
    account_ids: Vec<String>,
) -> Result<(), String> {
    cursor_call_async_with_timeout::<()>(
        "accounts.deleteMany",
        json!({ "accountIds": account_ids }),
        CURSOR_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(())
}

#[tauri::command]
pub async fn import_cursor_from_json(
    app: AppHandle,
    json_content: String,
) -> Result<Vec<CursorAccount>, String> {
    let accounts = cursor_call_async_with_timeout(
        "accounts.importJson",
        json!({ "jsonContent": json_content }),
        CURSOR_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(accounts)
}

#[tauri::command]
pub async fn import_cursor_from_local(app: AppHandle) -> Result<Vec<CursorAccount>, String> {
    let accounts: Vec<CursorAccount> = cursor_call_async_with_timeout(
        "accounts.importLocal",
        json!({}),
        CURSOR_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(accounts)
}

#[tauri::command]
pub fn export_cursor_accounts(account_ids: Vec<String>) -> Result<String, String> {
    cursor_call("accounts.export", json!({ "accountIds": account_ids }))
}

#[tauri::command]
pub async fn refresh_cursor_token(
    app: AppHandle,
    account_id: String,
) -> Result<CursorAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Cursor Command] 手动刷新账号开始: account_id={}",
        account_id
    ));
    let account: CursorAccount =
        cursor_call_async("accounts.refresh", json!({ "accountId": account_id })).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Cursor Command] 手动刷新账号完成: account_id={}, elapsed={}ms",
        account.id,
        started_at.elapsed().as_millis()
    ));
    Ok(account)
}

#[tauri::command]
pub async fn refresh_all_cursor_tokens(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[Cursor Command] 手动批量刷新开始");
    let success_count: i32 = cursor_call_async("accounts.refreshAll", json!({})).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Cursor Command] 手动批量刷新完成: success={}, elapsed={}ms",
        success_count,
        started_at.elapsed().as_millis()
    ));
    Ok(success_count)
}

#[tauri::command]
pub async fn add_cursor_account_with_token(
    app: AppHandle,
    access_token: String,
) -> Result<CursorAccount, String> {
    let account: CursorAccount = cursor_call_async_with_timeout(
        "accounts.addToken",
        json!({ "accessToken": access_token }),
        CURSOR_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(account)
}

#[tauri::command]
pub fn update_cursor_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<CursorAccount, String> {
    cursor_call(
        "accounts.updateTags",
        json!({ "accountId": account_id, "tags": tags }),
    )
}

#[tauri::command]
pub fn get_cursor_accounts_index_path() -> Result<String, String> {
    cursor_call("accounts.indexPath", json!({}))
}

#[tauri::command]
pub async fn cursor_oauth_login_start() -> Result<Value, String> {
    logger::log_info("[Cursor Command] OAuth 登录开始");
    cursor_call_async("oauth.start", json!({})).await
}

#[tauri::command]
pub async fn cursor_oauth_login_complete(
    app: AppHandle,
    login_id: String,
) -> Result<CursorAccount, String> {
    logger::log_info(&format!(
        "[Cursor Command] OAuth 等待完成: login_id={}",
        login_id
    ));
    let account: CursorAccount =
        cursor_call_async("oauth.complete", json!({ "loginId": login_id })).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Cursor Command] OAuth 登录完成: account_id={}, email={}",
        account.id, account.email
    ));
    Ok(account)
}

#[tauri::command]
pub fn cursor_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    logger::log_info(&format!(
        "[Cursor Command] OAuth 取消: login_id={}",
        login_id.as_deref().unwrap_or("<none>")
    ));
    cursor_call("oauth.cancel", json!({ "loginId": login_id }))
}

#[tauri::command]
pub async fn inject_cursor_account(app: AppHandle, account_id: String) -> Result<String, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Cursor Switch] 开始切换账号: account_id={}",
        account_id
    ));

    let result: SwitchResult =
        cursor_call_async("switch.inject", json!({ "accountId": account_id })).await?;
    let _ = crate::modules::provider_current_state::set_current_account_id(
        "cursor",
        Some(account_id.as_str()),
    );
    let _ = crate::modules::tray::update_tray_menu(&app);

    if result.path_missing {
        emit_cursor_path_missing(&app, json!({ "kind": "default" }));
        if let Some(error) = result.restart_error.as_deref() {
            logger::log_warn(&format!(
                "[Cursor Switch] 切号完成但启动失败: err={}",
                error
            ));
        }
        return Ok(result.message);
    }

    logger::log_info(&format!(
        "[Cursor Switch] 切号成功: elapsed={}ms",
        started_at.elapsed().as_millis()
    ));
    Ok(result.message)
}
