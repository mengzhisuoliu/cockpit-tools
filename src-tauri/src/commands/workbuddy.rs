use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::models::codebuddy::{CodebuddyCheckinResponse, CodebuddyCheckinStatusResponse};
use crate::models::workbuddy::{WorkbuddyAccount, WorkbuddyOAuthStartResponse};
use crate::modules::{logger, platform_adapter, platform_package};

const WORKBUDDY_FAST_LOCAL_MUTATION_TIMEOUT: Duration = Duration::from_secs(20);

fn ensure_workbuddy_package_installed() -> Result<(), String> {
    platform_package::ensure_platform_package_installed("workbuddy")
}

fn workbuddy_call<T: DeserializeOwned>(method: &str, payload: Value) -> Result<T, String> {
    ensure_workbuddy_package_installed()?;
    platform_adapter::call_workbuddy(method, payload)
}

async fn workbuddy_call_async<T>(method: &'static str, payload: Value) -> Result<T, String>
where
    T: DeserializeOwned + Send + 'static,
{
    ensure_workbuddy_package_installed()?;
    tauri::async_runtime::spawn_blocking(move || platform_adapter::call_workbuddy(method, payload))
        .await
        .map_err(|error| format!("WorkBuddy adapter 任务失败: {}", error))?
}

async fn workbuddy_call_async_with_timeout<T>(
    method: &'static str,
    payload: Value,
    timeout: Duration,
) -> Result<T, String>
where
    T: DeserializeOwned + Send + 'static,
{
    ensure_workbuddy_package_installed()?;
    tauri::async_runtime::spawn_blocking(move || {
        platform_adapter::call_workbuddy_with_timeout(method, payload, timeout)
    })
    .await
    .map_err(|error| format!("WorkBuddy adapter 任务失败: {}", error))?
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

fn emit_workbuddy_path_missing(app: &AppHandle, retry: Value) {
    let _ = app.emit(
        "app:path_missing",
        json!({
            "app": "workbuddy",
            "retry": retry
        }),
    );
}

#[tauri::command]
pub async fn list_workbuddy_accounts() -> Result<Vec<WorkbuddyAccount>, String> {
    workbuddy_call_async_with_timeout(
        "accounts.list",
        json!({}),
        WORKBUDDY_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await
}

#[tauri::command]
pub async fn delete_workbuddy_account(app: AppHandle, account_id: String) -> Result<(), String> {
    workbuddy_call_async_with_timeout::<()>(
        "accounts.delete",
        json!({ "accountId": account_id }),
        WORKBUDDY_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(())
}

#[tauri::command]
pub async fn delete_workbuddy_accounts(
    app: AppHandle,
    account_ids: Vec<String>,
) -> Result<(), String> {
    workbuddy_call_async_with_timeout::<()>(
        "accounts.deleteMany",
        json!({ "accountIds": account_ids }),
        WORKBUDDY_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(())
}

#[tauri::command]
pub async fn import_workbuddy_from_json(
    app: AppHandle,
    json_content: String,
) -> Result<Vec<WorkbuddyAccount>, String> {
    let accounts = workbuddy_call_async_with_timeout(
        "accounts.importJson",
        json!({ "jsonContent": json_content }),
        WORKBUDDY_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(accounts)
}

#[tauri::command]
pub async fn import_workbuddy_from_local(app: AppHandle) -> Result<Vec<WorkbuddyAccount>, String> {
    let accounts: Vec<WorkbuddyAccount> = workbuddy_call_async_with_timeout(
        "accounts.importLocal",
        json!({}),
        WORKBUDDY_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(accounts)
}

#[tauri::command]
pub fn export_workbuddy_accounts(account_ids: Vec<String>) -> Result<String, String> {
    workbuddy_call("accounts.export", json!({ "accountIds": account_ids }))
}

#[tauri::command]
pub async fn refresh_workbuddy_token(
    app: AppHandle,
    account_id: String,
) -> Result<WorkbuddyAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[WorkBuddy Command] 手动刷新账号开始: account_id={}",
        account_id
    ));
    let account: WorkbuddyAccount =
        workbuddy_call_async("accounts.refresh", json!({ "accountId": account_id })).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[WorkBuddy Command] 手动刷新账号完成: account_id={}, email={}, elapsed={}ms",
        account.id,
        account.email,
        started_at.elapsed().as_millis()
    ));
    Ok(account)
}

#[tauri::command]
pub async fn refresh_all_workbuddy_tokens(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[WorkBuddy Command] 手动批量刷新开始");
    let success_count: i32 = workbuddy_call_async("accounts.refreshAll", json!({})).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[WorkBuddy Command] 手动批量刷新完成: success={}, elapsed={}ms",
        success_count,
        started_at.elapsed().as_millis()
    ));
    Ok(success_count)
}

#[tauri::command]
pub async fn workbuddy_oauth_login_start() -> Result<WorkbuddyOAuthStartResponse, String> {
    logger::log_info("[WorkBuddy Command] OAuth 登录开始");
    workbuddy_call_async("oauth.start", json!({})).await
}

#[tauri::command]
pub async fn workbuddy_oauth_login_complete(
    app: AppHandle,
    login_id: String,
) -> Result<WorkbuddyAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[WorkBuddy Command] OAuth 等待完成: login_id={}",
        login_id
    ));
    let account: WorkbuddyAccount =
        workbuddy_call_async("oauth.complete", json!({ "loginId": login_id })).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[WorkBuddy Command] OAuth 登录完成: account_id={}, email={}, elapsed={}ms",
        account.id,
        account.email,
        started_at.elapsed().as_millis()
    ));
    Ok(account)
}

#[tauri::command]
pub fn workbuddy_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    logger::log_info(&format!(
        "[WorkBuddy Command] OAuth 取消: login_id={}",
        login_id.as_deref().unwrap_or("<none>")
    ));
    workbuddy_call("oauth.cancel", json!({ "loginId": login_id }))
}

#[tauri::command]
pub async fn add_workbuddy_account_with_token(
    app: AppHandle,
    access_token: String,
) -> Result<WorkbuddyAccount, String> {
    let account: WorkbuddyAccount = workbuddy_call_async_with_timeout(
        "accounts.addToken",
        json!({ "accessToken": access_token }),
        WORKBUDDY_FAST_LOCAL_MUTATION_TIMEOUT,
    )
    .await?;
    update_tray_menu_in_background(app);
    Ok(account)
}

#[tauri::command]
pub async fn update_workbuddy_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<WorkbuddyAccount, String> {
    workbuddy_call(
        "accounts.updateTags",
        json!({ "accountId": account_id, "tags": tags }),
    )
}

#[tauri::command]
pub fn get_workbuddy_accounts_index_path() -> Result<String, String> {
    workbuddy_call("accounts.indexPath", json!({}))
}

#[tauri::command]
pub async fn inject_workbuddy_to_vscode(
    app: AppHandle,
    account_id: String,
) -> Result<String, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[WorkBuddy Switch] 开始切换账号: account_id={}",
        account_id
    ));

    let result: SwitchResult =
        workbuddy_call_async("switch.inject", json!({ "accountId": account_id })).await?;
    let _ = crate::modules::provider_current_state::set_current_account_id(
        "workbuddy",
        Some(account_id.as_str()),
    );
    let _ = crate::modules::tray::update_tray_menu(&app);

    if result.path_missing {
        emit_workbuddy_path_missing(&app, json!({ "kind": "default" }));
    }
    if let Some(err) = result.restart_error.as_deref() {
        logger::log_warn(&format!(
            "[WorkBuddy Switch] 切号完成但启动失败: account_id={}, elapsed={}ms, error={}",
            account_id,
            started_at.elapsed().as_millis(),
            err
        ));
    } else {
        logger::log_info(&format!(
            "[WorkBuddy Switch] 切号成功: account_id={}, elapsed={}ms",
            account_id,
            started_at.elapsed().as_millis()
        ));
    }

    Ok(result.message)
}

#[tauri::command]
pub async fn sync_workbuddy_to_codebuddy_cn(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[WorkBuddy -> CodeBuddy CN] 开始同步账号");
    let synced_count: i32 = workbuddy_call_async("accounts.syncToCodebuddyCn", json!({})).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[WorkBuddy -> CodeBuddy CN] 同步完成: count={}, elapsed={}ms",
        synced_count,
        started_at.elapsed().as_millis()
    ));
    Ok(synced_count)
}

#[tauri::command]
pub async fn get_checkin_status_workbuddy(
    account_id: String,
) -> Result<CodebuddyCheckinStatusResponse, String> {
    workbuddy_call_async("checkin.status", json!({ "accountId": account_id })).await
}

#[tauri::command]
pub async fn checkin_workbuddy(
    app: AppHandle,
    account_id: String,
) -> Result<CodebuddyCheckinResponse, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[WorkBuddy Checkin] 执行签到开始: account_id={}",
        account_id
    ));
    let response: CodebuddyCheckinResponse =
        workbuddy_call_async("checkin.perform", json!({ "accountId": account_id })).await?;

    if response.success {
        let _ = crate::modules::tray::update_tray_menu(&app);
        let _ = app.emit(
            "workbuddy:checkin_completed",
            json!({
                "accountId": account_id,
                "success": true,
                "reward": response.reward.clone(),
            }),
        );
    }

    logger::log_info(&format!(
        "[WorkBuddy Checkin] 执行签到完成: success={}, elapsed={}ms",
        response.success,
        started_at.elapsed().as_millis()
    ));
    Ok(response)
}
