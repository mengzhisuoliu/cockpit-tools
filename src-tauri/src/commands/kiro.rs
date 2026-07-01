use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::models::kiro::{KiroAccount, KiroOAuthStartResponse};
use crate::modules::{logger, platform_adapter, platform_package};

const KIRO_FAST_LOCAL_MUTATION_TIMEOUT: Duration = Duration::from_secs(20);

fn ensure_kiro_package_installed() -> Result<(), String> {
    platform_package::ensure_platform_package_installed("kiro")
}

fn kiro_call<T: DeserializeOwned>(method: &str, payload: Value) -> Result<T, String> {
    ensure_kiro_package_installed()?;
    platform_adapter::call_kiro(method, payload)
}

async fn kiro_call_async<T>(method: &'static str, payload: Value) -> Result<T, String>
where
    T: DeserializeOwned + Send + 'static,
{
    ensure_kiro_package_installed()?;
    tauri::async_runtime::spawn_blocking(move || platform_adapter::call_kiro(method, payload))
        .await
        .map_err(|error| format!("Kiro adapter 任务失败: {}", error))?
}

async fn kiro_call_async_with_timeout<T>(
    method: &'static str,
    payload: Value,
    timeout: Duration,
) -> Result<T, String>
where
    T: DeserializeOwned + Send + 'static,
{
    ensure_kiro_package_installed()?;
    tauri::async_runtime::spawn_blocking(move || {
        platform_adapter::call_kiro_with_timeout(method, payload, timeout)
    })
    .await
    .map_err(|error| format!("Kiro adapter 任务失败: {}", error))?
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

fn emit_kiro_path_missing(app: &AppHandle, retry: Value) {
    let _ = app.emit(
        "app:path_missing",
        json!({
            "app": "kiro",
            "retry": retry
        }),
    );
}

#[tauri::command]
pub async fn list_kiro_accounts() -> Result<Vec<KiroAccount>, String> {
    kiro_call_async_with_timeout("accounts.list", json!({}), KIRO_FAST_LOCAL_MUTATION_TIMEOUT).await
}

#[tauri::command]
pub async fn delete_kiro_account(app: AppHandle, account_id: String) -> Result<(), String> {
    kiro_call_async_with_timeout::<()>(
        "accounts.delete",
        json!({ "accountId": account_id }),
        KIRO_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(())
}

#[tauri::command]
pub async fn delete_kiro_accounts(app: AppHandle, account_ids: Vec<String>) -> Result<(), String> {
    kiro_call_async_with_timeout::<()>(
        "accounts.deleteMany",
        json!({ "accountIds": account_ids }),
        KIRO_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(())
}

#[tauri::command]
pub async fn import_kiro_from_json(
    app: AppHandle,
    json_content: String,
) -> Result<Vec<KiroAccount>, String> {
    let accounts = kiro_call_async_with_timeout(
        "accounts.importJson",
        json!({ "jsonContent": json_content }),
        KIRO_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(accounts)
}

#[tauri::command]
pub async fn import_kiro_from_local(app: AppHandle) -> Result<Vec<KiroAccount>, String> {
    let accounts: Vec<KiroAccount> = kiro_call_async_with_timeout(
        "accounts.importLocal",
        json!({}),
        KIRO_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(accounts)
}

#[tauri::command]
pub fn export_kiro_accounts(account_ids: Vec<String>) -> Result<String, String> {
    kiro_call("accounts.export", json!({ "accountIds": account_ids }))
}

#[tauri::command]
pub async fn refresh_kiro_token(app: AppHandle, account_id: String) -> Result<KiroAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Kiro Command] 手动刷新账号开始: account_id={}",
        account_id
    ));
    let account: KiroAccount =
        kiro_call_async("accounts.refresh", json!({ "accountId": account_id })).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Kiro Command] 手动刷新账号完成: account_id={}, elapsed={}ms",
        account.id,
        started_at.elapsed().as_millis()
    ));
    Ok(account)
}

#[tauri::command]
pub async fn refresh_all_kiro_tokens(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[Kiro Command] 手动批量刷新开始");
    let success_count: i32 = kiro_call_async("accounts.refreshAll", json!({})).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Kiro Command] 手动批量刷新完成: success={}, elapsed={}ms",
        success_count,
        started_at.elapsed().as_millis()
    ));
    Ok(success_count)
}

#[tauri::command]
pub async fn kiro_oauth_login_start() -> Result<KiroOAuthStartResponse, String> {
    logger::log_info("Kiro OAuth start 命令触发");
    kiro_call_async("oauth.start", json!({})).await
}

#[tauri::command]
pub async fn kiro_oauth_login_complete(
    app: AppHandle,
    login_id: String,
) -> Result<KiroAccount, String> {
    logger::log_info(&format!(
        "Kiro OAuth complete 命令触发: login_id={}",
        login_id
    ));
    let account: KiroAccount =
        kiro_call_async("oauth.complete", json!({ "loginId": login_id })).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "Kiro OAuth complete 成功: account_id={}, email={}",
        account.id, account.email
    ));
    Ok(account)
}

#[tauri::command]
pub fn kiro_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    logger::log_info(&format!(
        "Kiro OAuth cancel 命令触发: login_id={}",
        login_id.as_deref().unwrap_or("<none>")
    ));
    kiro_call("oauth.cancel", json!({ "loginId": login_id }))
}

#[tauri::command]
pub fn kiro_oauth_submit_callback_url(
    login_id: String,
    callback_url: String,
) -> Result<(), String> {
    kiro_call(
        "oauth.submitCallbackUrl",
        json!({ "loginId": login_id, "callbackUrl": callback_url }),
    )
}

#[tauri::command]
pub async fn add_kiro_account_with_token(
    app: AppHandle,
    access_token: String,
) -> Result<KiroAccount, String> {
    let account: KiroAccount = kiro_call_async_with_timeout(
        "accounts.addToken",
        json!({ "accessToken": access_token }),
        KIRO_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(account)
}

#[tauri::command]
pub async fn update_kiro_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<KiroAccount, String> {
    kiro_call(
        "accounts.updateTags",
        json!({ "accountId": account_id, "tags": tags }),
    )
}

#[tauri::command]
pub fn get_kiro_accounts_index_path() -> Result<String, String> {
    kiro_call("accounts.indexPath", json!({}))
}

#[tauri::command]
pub async fn inject_kiro_to_vscode(app: AppHandle, account_id: String) -> Result<String, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Kiro Switch] 开始切换账号: account_id={}",
        account_id
    ));

    let result: SwitchResult =
        kiro_call_async("switch.inject", json!({ "accountId": account_id })).await?;
    let _ = crate::modules::provider_current_state::set_current_account_id(
        "kiro",
        Some(account_id.as_str()),
    );
    let _ = crate::modules::tray::update_tray_menu(&app);

    if result.path_missing {
        emit_kiro_path_missing(
            &app,
            json!({ "kind": "switchAccount", "accountId": account_id }),
        );
        if let Some(error) = result.restart_error.as_deref() {
            logger::log_warn(&format!("[Kiro Switch] 切号完成但启动失败: err={}", error));
        }
        return Ok(result.message);
    }

    logger::log_info(&format!(
        "[Kiro Switch] 切号成功: elapsed={}ms",
        started_at.elapsed().as_millis()
    ));
    Ok(result.message)
}
