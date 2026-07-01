use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::models::trae::TraeAccount;
use crate::modules::{logger, platform_adapter, platform_package};

const TRAE_FAST_LOCAL_MUTATION_TIMEOUT: Duration = Duration::from_secs(20);

fn ensure_trae_package_installed() -> Result<(), String> {
    platform_package::ensure_platform_package_installed("trae")
}

fn trae_call<T: DeserializeOwned>(method: &str, payload: Value) -> Result<T, String> {
    ensure_trae_package_installed()?;
    platform_adapter::call_trae(method, payload)
}

async fn trae_call_async<T>(method: &'static str, payload: Value) -> Result<T, String>
where
    T: DeserializeOwned + Send + 'static,
{
    ensure_trae_package_installed()?;
    tauri::async_runtime::spawn_blocking(move || platform_adapter::call_trae(method, payload))
        .await
        .map_err(|error| format!("Trae adapter 任务失败: {}", error))?
}

async fn trae_call_async_with_timeout<T>(
    method: &'static str,
    payload: Value,
    timeout: Duration,
) -> Result<T, String>
where
    T: DeserializeOwned + Send + 'static,
{
    ensure_trae_package_installed()?;
    tauri::async_runtime::spawn_blocking(move || {
        platform_adapter::call_trae_with_timeout(method, payload, timeout)
    })
    .await
    .map_err(|error| format!("Trae adapter 任务失败: {}", error))?
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

fn emit_trae_path_missing(app: &AppHandle, retry: Value) {
    let _ = app.emit(
        "app:path_missing",
        json!({
            "app": "trae",
            "retry": retry
        }),
    );
}

#[tauri::command]
pub async fn list_trae_accounts() -> Result<Vec<TraeAccount>, String> {
    trae_call_async_with_timeout("accounts.list", json!({}), TRAE_FAST_LOCAL_MUTATION_TIMEOUT).await
}

#[tauri::command]
pub async fn delete_trae_account(app: AppHandle, account_id: String) -> Result<(), String> {
    trae_call_async_with_timeout::<()>(
        "accounts.delete",
        json!({ "accountId": account_id }),
        TRAE_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(())
}

#[tauri::command]
pub async fn delete_trae_accounts(app: AppHandle, account_ids: Vec<String>) -> Result<(), String> {
    trae_call_async_with_timeout::<()>(
        "accounts.deleteMany",
        json!({ "accountIds": account_ids }),
        TRAE_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(())
}

#[tauri::command]
pub async fn import_trae_from_json(
    app: AppHandle,
    json_content: String,
) -> Result<Vec<TraeAccount>, String> {
    let accounts = trae_call_async_with_timeout(
        "accounts.importJson",
        json!({ "jsonContent": json_content }),
        TRAE_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(accounts)
}

#[tauri::command]
pub async fn import_trae_from_local(app: AppHandle) -> Result<Vec<TraeAccount>, String> {
    let accounts: Vec<TraeAccount> = trae_call_async_with_timeout(
        "accounts.importLocal",
        json!({}),
        TRAE_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(accounts)
}

#[tauri::command]
pub fn export_trae_accounts(account_ids: Vec<String>) -> Result<String, String> {
    trae_call("accounts.export", json!({ "accountIds": account_ids }))
}

#[tauri::command]
pub async fn refresh_trae_token(app: AppHandle, account_id: String) -> Result<TraeAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Trae Command] 手动刷新账号开始: account_id={}",
        account_id
    ));
    let account: TraeAccount =
        trae_call_async("accounts.refresh", json!({ "accountId": account_id })).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Trae Command] 手动刷新账号完成: account_id={}, elapsed={}ms",
        account.id,
        started_at.elapsed().as_millis()
    ));
    Ok(account)
}

#[tauri::command]
pub async fn refresh_all_trae_tokens(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[Trae Command] 手动批量刷新开始");
    let success_count: i32 = trae_call_async("accounts.refreshAll", json!({})).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Trae Command] 手动批量刷新完成: success={}, elapsed={}ms",
        success_count,
        started_at.elapsed().as_millis()
    ));
    Ok(success_count)
}

#[tauri::command]
pub async fn add_trae_account_with_token(
    app: AppHandle,
    access_token: String,
) -> Result<TraeAccount, String> {
    let account: TraeAccount = trae_call_async_with_timeout(
        "accounts.addToken",
        json!({ "accessToken": access_token }),
        TRAE_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(account)
}

#[tauri::command]
pub fn update_trae_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<TraeAccount, String> {
    trae_call(
        "accounts.updateTags",
        json!({ "accountId": account_id, "tags": tags }),
    )
}

#[tauri::command]
pub fn get_trae_accounts_index_path() -> Result<String, String> {
    trae_call("accounts.indexPath", json!({}))
}

#[tauri::command]
pub async fn trae_oauth_login_start() -> Result<Value, String> {
    logger::log_info("[Trae Command] OAuth 登录开始");
    trae_call_async("oauth.start", json!({})).await
}

#[tauri::command]
pub async fn trae_oauth_login_complete(
    app: AppHandle,
    login_id: String,
) -> Result<TraeAccount, String> {
    logger::log_info(&format!(
        "[Trae Command] OAuth 等待完成: login_id={}",
        login_id
    ));
    let account: TraeAccount =
        trae_call_async("oauth.complete", json!({ "loginId": login_id })).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Trae Command] OAuth 登录完成: account_id={}, email={}",
        account.id, account.email
    ));
    Ok(account)
}

#[tauri::command]
pub fn trae_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    logger::log_info(&format!(
        "[Trae Command] OAuth 取消: login_id={}",
        login_id.as_deref().unwrap_or("<none>")
    ));
    trae_call("oauth.cancel", json!({ "loginId": login_id }))
}

#[tauri::command]
pub fn trae_oauth_submit_callback_url(
    login_id: String,
    callback_url: String,
) -> Result<(), String> {
    trae_call(
        "oauth.submitCallbackUrl",
        json!({ "loginId": login_id, "callbackUrl": callback_url }),
    )
}

#[tauri::command]
pub async fn inject_trae_account(app: AppHandle, account_id: String) -> Result<String, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Trae Switch] 开始切换账号: account_id={}",
        account_id
    ));

    let result: SwitchResult =
        trae_call_async("switch.inject", json!({ "accountId": account_id })).await?;
    let _ = crate::modules::provider_current_state::set_current_account_id(
        "trae",
        Some(account_id.as_str()),
    );
    let _ = crate::modules::tray::update_tray_menu(&app);

    if result.path_missing {
        emit_trae_path_missing(&app, json!({ "kind": "default" }));
        if let Some(error) = result.restart_error.as_deref() {
            logger::log_warn(&format!("[Trae Switch] 切号完成但启动失败: err={}", error));
        }
        return Ok(result.message);
    }

    logger::log_info(&format!(
        "[Trae Switch] 切号成功: elapsed={}ms",
        started_at.elapsed().as_millis()
    ));
    Ok(result.message)
}
