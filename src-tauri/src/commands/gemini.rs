use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::models::gemini::GeminiAccount;
use crate::modules::{logger, platform_adapter, platform_package};

const GEMINI_FAST_LOCAL_MUTATION_TIMEOUT: Duration = Duration::from_secs(20);

fn ensure_gemini_package_installed() -> Result<(), String> {
    platform_package::ensure_platform_package_installed("gemini")
}

fn gemini_call<T: DeserializeOwned>(method: &str, payload: Value) -> Result<T, String> {
    ensure_gemini_package_installed()?;
    platform_adapter::call_gemini(method, payload)
}

async fn gemini_call_async<T>(method: &'static str, payload: Value) -> Result<T, String>
where
    T: DeserializeOwned + Send + 'static,
{
    ensure_gemini_package_installed()?;
    tauri::async_runtime::spawn_blocking(move || platform_adapter::call_gemini(method, payload))
        .await
        .map_err(|error| format!("Gemini adapter 任务失败: {}", error))?
}

async fn gemini_call_async_with_timeout<T>(
    method: &'static str,
    payload: Value,
    timeout: Duration,
) -> Result<T, String>
where
    T: DeserializeOwned + Send + 'static,
{
    ensure_gemini_package_installed()?;
    tauri::async_runtime::spawn_blocking(move || {
        platform_adapter::call_gemini_with_timeout(method, payload, timeout)
    })
    .await
    .map_err(|error| format!("Gemini adapter 任务失败: {}", error))?
}

fn update_tray_menu_in_background(app: AppHandle) {
    tauri::async_runtime::spawn_blocking(move || {
        let _ = crate::modules::tray::update_tray_menu(&app);
    });
}

#[tauri::command]
pub async fn list_gemini_accounts() -> Result<Vec<GeminiAccount>, String> {
    gemini_call_async_with_timeout(
        "accounts.list",
        json!({}),
        GEMINI_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await
}

#[tauri::command]
pub async fn delete_gemini_account(app: AppHandle, account_id: String) -> Result<(), String> {
    gemini_call_async_with_timeout::<()>(
        "accounts.delete",
        json!({ "accountId": account_id }),
        GEMINI_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(())
}

#[tauri::command]
pub async fn delete_gemini_accounts(
    app: AppHandle,
    account_ids: Vec<String>,
) -> Result<(), String> {
    gemini_call_async_with_timeout::<()>(
        "accounts.deleteMany",
        json!({ "accountIds": account_ids }),
        GEMINI_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(())
}

#[tauri::command]
pub async fn import_gemini_from_json(
    app: AppHandle,
    json_content: String,
) -> Result<Vec<GeminiAccount>, String> {
    let accounts = gemini_call_async_with_timeout(
        "accounts.importJson",
        json!({ "jsonContent": json_content }),
        GEMINI_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(accounts)
}

#[tauri::command]
pub async fn import_gemini_from_local(app: AppHandle) -> Result<Vec<GeminiAccount>, String> {
    let accounts: Vec<GeminiAccount> = gemini_call_async_with_timeout(
        "accounts.importLocal",
        json!({}),
        GEMINI_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(accounts)
}

#[tauri::command]
pub fn export_gemini_accounts(account_ids: Vec<String>) -> Result<String, String> {
    gemini_call("accounts.export", json!({ "accountIds": account_ids }))
}

#[tauri::command]
pub async fn refresh_gemini_token(
    app: AppHandle,
    account_id: String,
) -> Result<GeminiAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Gemini Command] 手动刷新账号开始: account_id={}",
        account_id
    ));
    let account: GeminiAccount =
        gemini_call_async("accounts.refresh", json!({ "accountId": account_id })).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Gemini Command] 手动刷新账号完成: account_id={}, elapsed={}ms",
        account.id,
        started_at.elapsed().as_millis()
    ));
    Ok(account)
}

#[tauri::command]
pub async fn refresh_all_gemini_tokens(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[Gemini Command] 手动批量刷新开始");
    let success_count: i32 = gemini_call_async("accounts.refreshAll", json!({})).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Gemini Command] 手动批量刷新完成: success={}, elapsed={}ms",
        success_count,
        started_at.elapsed().as_millis()
    ));
    Ok(success_count)
}

#[tauri::command]
pub async fn gemini_oauth_login_start() -> Result<Value, String> {
    logger::log_info("[Gemini Command] OAuth 登录开始");
    gemini_call_async("oauth.start", json!({})).await
}

#[tauri::command]
pub async fn gemini_oauth_login_complete(
    app: AppHandle,
    login_id: String,
) -> Result<GeminiAccount, String> {
    logger::log_info(&format!(
        "[Gemini Command] OAuth 等待完成: login_id={}",
        login_id
    ));
    let account: GeminiAccount =
        gemini_call_async("oauth.complete", json!({ "loginId": login_id })).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Gemini Command] OAuth 登录完成: account_id={}, email={}",
        account.id, account.email
    ));
    Ok(account)
}

#[tauri::command]
pub fn gemini_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    logger::log_info(&format!(
        "[Gemini Command] OAuth 取消: login_id={}",
        login_id.as_deref().unwrap_or("<none>")
    ));
    gemini_call("oauth.cancel", json!({ "loginId": login_id }))
}

#[tauri::command]
pub fn gemini_oauth_submit_callback_url(
    login_id: String,
    callback_url: String,
) -> Result<(), String> {
    gemini_call(
        "oauth.submitCallbackUrl",
        json!({ "loginId": login_id, "callbackUrl": callback_url }),
    )
}

#[tauri::command]
pub async fn add_gemini_account_with_token(
    app: AppHandle,
    access_token: String,
) -> Result<GeminiAccount, String> {
    let account: GeminiAccount = gemini_call_async_with_timeout(
        "accounts.addToken",
        json!({ "accessToken": access_token }),
        GEMINI_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(account)
}

#[tauri::command]
pub fn update_gemini_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<GeminiAccount, String> {
    gemini_call(
        "accounts.updateTags",
        json!({ "accountId": account_id, "tags": tags }),
    )
}

#[tauri::command]
pub fn get_gemini_accounts_index_path() -> Result<String, String> {
    gemini_call("accounts.indexPath", json!({}))
}

#[tauri::command]
pub async fn inject_gemini_account(app: AppHandle, account_id: String) -> Result<String, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Gemini Switch] 开始切换账号: account_id={}",
        account_id
    ));

    let message: String =
        gemini_call_async("switch.inject", json!({ "accountId": account_id })).await?;
    let _ = crate::modules::provider_current_state::set_current_account_id(
        "gemini",
        Some(account_id.as_str()),
    );
    let _ = crate::modules::tray::update_tray_menu(&app);

    logger::log_info(&format!(
        "[Gemini Switch] 切号成功: elapsed={}ms",
        started_at.elapsed().as_millis()
    ));
    Ok(message)
}
