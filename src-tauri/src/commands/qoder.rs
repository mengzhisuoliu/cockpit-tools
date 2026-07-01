use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::models::qoder::QoderAccount;
use crate::modules::{logger, platform_adapter, platform_package};

const QODER_FAST_LOCAL_MUTATION_TIMEOUT: Duration = Duration::from_secs(20);

fn ensure_qoder_package_installed() -> Result<(), String> {
    platform_package::ensure_platform_package_installed("qoder")
}

fn qoder_call<T: DeserializeOwned>(method: &str, payload: Value) -> Result<T, String> {
    ensure_qoder_package_installed()?;
    platform_adapter::call_qoder(method, payload)
}

async fn qoder_call_async<T>(method: &'static str, payload: Value) -> Result<T, String>
where
    T: DeserializeOwned + Send + 'static,
{
    ensure_qoder_package_installed()?;
    tauri::async_runtime::spawn_blocking(move || platform_adapter::call_qoder(method, payload))
        .await
        .map_err(|error| format!("Qoder adapter 任务失败: {}", error))?
}

async fn qoder_call_async_with_timeout<T>(
    method: &'static str,
    payload: Value,
    timeout: Duration,
) -> Result<T, String>
where
    T: DeserializeOwned + Send + 'static,
{
    ensure_qoder_package_installed()?;
    tauri::async_runtime::spawn_blocking(move || {
        platform_adapter::call_qoder_with_timeout(method, payload, timeout)
    })
    .await
    .map_err(|error| format!("Qoder adapter 任务失败: {}", error))?
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

fn emit_qoder_path_missing(app: &AppHandle, retry: Value) {
    let _ = app.emit(
        "app:path_missing",
        json!({
            "app": "qoder",
            "retry": retry
        }),
    );
}

#[tauri::command]
pub async fn list_qoder_accounts() -> Result<Vec<QoderAccount>, String> {
    qoder_call_async_with_timeout(
        "accounts.list",
        json!({}),
        QODER_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await
}

#[tauri::command]
pub async fn delete_qoder_account(app: AppHandle, account_id: String) -> Result<(), String> {
    qoder_call_async_with_timeout::<()>(
        "accounts.delete",
        json!({ "accountId": account_id }),
        QODER_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(())
}

#[tauri::command]
pub async fn delete_qoder_accounts(app: AppHandle, account_ids: Vec<String>) -> Result<(), String> {
    qoder_call_async_with_timeout::<()>(
        "accounts.deleteMany",
        json!({ "accountIds": account_ids }),
        QODER_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(())
}

#[tauri::command]
pub async fn import_qoder_from_json(
    app: AppHandle,
    json_content: String,
) -> Result<Vec<QoderAccount>, String> {
    let accounts = qoder_call_async_with_timeout(
        "accounts.importJson",
        json!({ "jsonContent": json_content }),
        QODER_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(accounts)
}

#[tauri::command]
pub async fn import_qoder_from_local(app: AppHandle) -> Result<Vec<QoderAccount>, String> {
    let accounts: Vec<QoderAccount> = qoder_call_async_with_timeout(
        "accounts.importLocal",
        json!({}),
        QODER_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(accounts)
}

#[tauri::command]
pub async fn qoder_oauth_login_start() -> Result<Value, String> {
    logger::log_info("[Qoder Command] OAuth 登录开始");
    qoder_call_async("oauth.start", json!({})).await
}

#[tauri::command]
pub fn qoder_oauth_login_peek() -> Result<Option<Value>, String> {
    qoder_call("oauth.peek", json!({}))
}

#[tauri::command]
pub async fn qoder_oauth_login_complete(
    app: AppHandle,
    login_id: String,
) -> Result<QoderAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Qoder Command] OAuth 等待完成: login_id={}",
        login_id
    ));
    let account: QoderAccount =
        qoder_call_async("oauth.complete", json!({ "loginId": login_id })).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Qoder Command] OAuth 登录完成: account_id={}, email={}, elapsed={}ms",
        account.id,
        account.email,
        started_at.elapsed().as_millis()
    ));
    Ok(account)
}

#[tauri::command]
pub fn qoder_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    logger::log_info(&format!(
        "[Qoder Command] OAuth 取消: login_id={}",
        login_id.as_deref().unwrap_or("<none>")
    ));
    qoder_call("oauth.cancel", json!({ "loginId": login_id }))
}

#[tauri::command]
pub fn export_qoder_accounts(account_ids: Vec<String>) -> Result<String, String> {
    qoder_call("accounts.export", json!({ "accountIds": account_ids }))
}

#[tauri::command]
pub async fn refresh_qoder_token(
    app: AppHandle,
    account_id: String,
) -> Result<QoderAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Qoder Command] 手动刷新账号开始: account_id={}",
        account_id
    ));
    let account: QoderAccount =
        qoder_call_async("accounts.refresh", json!({ "accountId": account_id })).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Qoder Command] 手动刷新账号完成: account_id={}, elapsed={}ms",
        account.id,
        started_at.elapsed().as_millis()
    ));
    Ok(account)
}

#[tauri::command]
pub async fn refresh_all_qoder_tokens(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[Qoder Command] 手动批量刷新开始");
    let success_count: i32 = qoder_call_async("accounts.refreshAll", json!({})).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Qoder Command] 手动批量刷新完成: success={}, elapsed={}ms",
        success_count,
        started_at.elapsed().as_millis()
    ));
    Ok(success_count)
}

#[tauri::command]
pub async fn inject_qoder_account(app: AppHandle, account_id: String) -> Result<String, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Qoder Switch] 开始切换账号: account_id={}",
        account_id
    ));

    let result: SwitchResult =
        qoder_call_async("switch.inject", json!({ "accountId": account_id })).await?;
    let _ = crate::modules::provider_current_state::set_current_account_id(
        "qoder",
        Some(account_id.as_str()),
    );
    let _ = crate::modules::tray::update_tray_menu(&app);

    if result.path_missing {
        emit_qoder_path_missing(&app, json!({ "kind": "default" }));
        if let Some(error) = result.restart_error.as_deref() {
            logger::log_warn(&format!("[Qoder Switch] 切号完成但启动失败: err={}", error));
        }
        return Ok(result.message);
    }

    logger::log_info(&format!(
        "[Qoder Switch] 切号成功: elapsed={}ms",
        started_at.elapsed().as_millis()
    ));
    Ok(result.message)
}

#[tauri::command]
pub fn update_qoder_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<QoderAccount, String> {
    qoder_call(
        "accounts.updateTags",
        json!({ "accountId": account_id, "tags": tags }),
    )
}

#[tauri::command]
pub fn get_qoder_accounts_index_path() -> Result<String, String> {
    qoder_call("accounts.indexPath", json!({}))
}
