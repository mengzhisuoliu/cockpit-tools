use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::models::codebuddy::{CodebuddyAccount, CodebuddyOAuthStartResponse};
use crate::modules::{logger, platform_adapter, platform_package};

const CODEBUDDY_CN_FAST_LOCAL_MUTATION_TIMEOUT: Duration = Duration::from_secs(20);

fn ensure_codebuddy_cn_package_installed() -> Result<(), String> {
    platform_package::ensure_platform_package_installed("codebuddy_cn")
}

fn codebuddy_cn_call<T: DeserializeOwned>(method: &str, payload: Value) -> Result<T, String> {
    ensure_codebuddy_cn_package_installed()?;
    platform_adapter::call_codebuddy_cn(method, payload)
}

async fn codebuddy_cn_call_async<T>(method: &'static str, payload: Value) -> Result<T, String>
where
    T: DeserializeOwned + Send + 'static,
{
    ensure_codebuddy_cn_package_installed()?;
    tauri::async_runtime::spawn_blocking(move || {
        platform_adapter::call_codebuddy_cn(method, payload)
    })
    .await
    .map_err(|error| format!("CodeBuddy CN adapter 任务失败: {}", error))?
}

async fn codebuddy_cn_call_async_with_timeout<T>(
    method: &'static str,
    payload: Value,
    timeout: Duration,
) -> Result<T, String>
where
    T: DeserializeOwned + Send + 'static,
{
    ensure_codebuddy_cn_package_installed()?;
    tauri::async_runtime::spawn_blocking(move || {
        platform_adapter::call_codebuddy_cn_with_timeout(method, payload, timeout)
    })
    .await
    .map_err(|error| format!("CodeBuddy CN adapter 任务失败: {}", error))?
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

fn emit_codebuddy_cn_path_missing(app: &AppHandle, retry: Value) {
    let _ = app.emit(
        "app:path_missing",
        json!({
            "app": "codebuddy_cn",
            "retry": retry
        }),
    );
}

#[tauri::command]
pub async fn list_codebuddy_cn_accounts() -> Result<Vec<CodebuddyAccount>, String> {
    codebuddy_cn_call_async_with_timeout(
        "accounts.list",
        json!({}),
        CODEBUDDY_CN_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await
}

#[tauri::command]
pub async fn delete_codebuddy_cn_account(app: AppHandle, account_id: String) -> Result<(), String> {
    codebuddy_cn_call_async_with_timeout::<()>(
        "accounts.delete",
        json!({ "accountId": account_id }),
        CODEBUDDY_CN_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(())
}

#[tauri::command]
pub async fn delete_codebuddy_cn_accounts(
    app: AppHandle,
    account_ids: Vec<String>,
) -> Result<(), String> {
    codebuddy_cn_call_async_with_timeout::<()>(
        "accounts.deleteMany",
        json!({ "accountIds": account_ids }),
        CODEBUDDY_CN_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(())
}

#[tauri::command]
pub async fn import_codebuddy_cn_from_json(
    app: AppHandle,
    json_content: String,
) -> Result<Vec<CodebuddyAccount>, String> {
    let accounts = codebuddy_cn_call_async_with_timeout(
        "accounts.importJson",
        json!({ "jsonContent": json_content }),
        CODEBUDDY_CN_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(accounts)
}

#[tauri::command]
pub async fn import_codebuddy_cn_from_local(
    app: AppHandle,
) -> Result<Vec<CodebuddyAccount>, String> {
    let accounts: Vec<CodebuddyAccount> = codebuddy_cn_call_async_with_timeout(
        "accounts.importLocal",
        json!({}),
        CODEBUDDY_CN_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(accounts)
}

#[tauri::command]
pub fn export_codebuddy_cn_accounts(account_ids: Vec<String>) -> Result<String, String> {
    codebuddy_cn_call("accounts.export", json!({ "accountIds": account_ids }))
}

#[tauri::command]
pub async fn refresh_codebuddy_cn_token(
    app: AppHandle,
    account_id: String,
) -> Result<CodebuddyAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[CodeBuddy CN Command] 手动刷新账号开始: account_id={}",
        account_id
    ));
    let account: CodebuddyAccount =
        codebuddy_cn_call_async("accounts.refresh", json!({ "accountId": account_id })).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[CodeBuddy CN Command] 手动刷新账号完成: account_id={}, email={}, elapsed={}ms",
        account.id,
        account.email,
        started_at.elapsed().as_millis()
    ));
    Ok(account)
}

#[tauri::command]
pub async fn refresh_all_codebuddy_cn_tokens(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[CodeBuddy CN Command] 手动批量刷新开始");
    let success_count: i32 = codebuddy_cn_call_async("accounts.refreshAll", json!({})).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[CodeBuddy CN Command] 手动批量刷新完成: success={}, elapsed={}ms",
        success_count,
        started_at.elapsed().as_millis()
    ));
    Ok(success_count)
}

#[tauri::command]
pub async fn codebuddy_cn_oauth_login_start() -> Result<CodebuddyOAuthStartResponse, String> {
    logger::log_info("[CodeBuddy CN Command] OAuth 登录开始");
    codebuddy_cn_call_async("oauth.start", json!({})).await
}

#[tauri::command]
pub async fn codebuddy_cn_oauth_login_complete(
    app: AppHandle,
    login_id: String,
) -> Result<CodebuddyAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[CodeBuddy CN Command] OAuth 等待完成: login_id={}",
        login_id
    ));
    let account: CodebuddyAccount =
        codebuddy_cn_call_async("oauth.complete", json!({ "loginId": login_id })).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[CodeBuddy CN Command] OAuth 登录完成: account_id={}, email={}, elapsed={}ms",
        account.id,
        account.email,
        started_at.elapsed().as_millis()
    ));
    Ok(account)
}

#[tauri::command]
pub fn codebuddy_cn_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    logger::log_info(&format!(
        "[CodeBuddy CN Command] OAuth 取消: login_id={}",
        login_id.as_deref().unwrap_or("<none>")
    ));
    codebuddy_cn_call("oauth.cancel", json!({ "loginId": login_id }))
}

#[tauri::command]
pub async fn add_codebuddy_cn_account_with_token(
    app: AppHandle,
    access_token: String,
) -> Result<CodebuddyAccount, String> {
    let account: CodebuddyAccount = codebuddy_cn_call_async_with_timeout(
        "accounts.addToken",
        json!({ "accessToken": access_token }),
        CODEBUDDY_CN_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(account)
}

#[tauri::command]
pub fn update_codebuddy_cn_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<CodebuddyAccount, String> {
    codebuddy_cn_call(
        "accounts.updateTags",
        json!({ "accountId": account_id, "tags": tags }),
    )
}

#[tauri::command]
pub fn get_codebuddy_cn_accounts_index_path() -> Result<String, String> {
    codebuddy_cn_call("accounts.indexPath", json!({}))
}

#[tauri::command]
pub async fn inject_codebuddy_cn_to_vscode(
    app: AppHandle,
    account_id: String,
) -> Result<String, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[CodeBuddy CN Switch] 开始切换账号: account_id={}",
        account_id
    ));

    let result: SwitchResult =
        codebuddy_cn_call_async("switch.inject", json!({ "accountId": account_id })).await?;
    let _ = crate::modules::provider_current_state::set_current_account_id(
        "codebuddy_cn",
        Some(account_id.as_str()),
    );
    let _ = crate::modules::tray::update_tray_menu(&app);

    if result.path_missing {
        emit_codebuddy_cn_path_missing(&app, json!({ "kind": "default" }));
        if let Some(error) = result.restart_error.as_deref() {
            logger::log_warn(&format!(
                "[CodeBuddy CN Switch] 切号完成但启动失败: err={}",
                error
            ));
        }
        return Ok(result.message);
    }

    logger::log_info(&format!(
        "[CodeBuddy CN Switch] 切号成功: elapsed={}ms",
        started_at.elapsed().as_millis()
    ));
    Ok(result.message)
}

#[tauri::command]
pub async fn sync_codebuddy_cn_to_workbuddy(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[CodeBuddy CN -> WorkBuddy] 开始同步账号");
    let synced_count: i32 = codebuddy_cn_call_async("accounts.syncToWorkbuddy", json!({})).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[CodeBuddy CN -> WorkBuddy] 同步完成: count={}, elapsed={}ms",
        synced_count,
        started_at.elapsed().as_millis()
    ));
    Ok(synced_count)
}
