use crate::models;
use crate::modules;
use serde::de::DeserializeOwned;
use serde_json::json;
use tauri::AppHandle;

async fn antigravity_import_call<T>(
    method: &'static str,
    payload: serde_json::Value,
) -> Result<T, String>
where
    T: DeserializeOwned + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        modules::platform_adapter::call_antigravity_series(method, payload)
    })
    .await
    .map_err(|error| format!("Antigravity adapter task failed: {}", error))?
}

fn update_tray_menu_in_background(app: AppHandle) {
    tauri::async_runtime::spawn_blocking(move || {
        let _ = crate::modules::tray::update_tray_menu(&app);
    });
}

#[tauri::command]
pub async fn import_from_old_tools() -> Result<Vec<models::Account>, String> {
    antigravity_import_call("accounts.importOldTools", json!({})).await
}

#[tauri::command]
pub async fn import_from_local(app: AppHandle) -> Result<models::Account, String> {
    let account = antigravity_import_call("accounts.importLocal", json!({})).await?;
    update_tray_menu_in_background(app);
    Ok(account)
}

#[tauri::command]
pub async fn import_from_json(json_content: String) -> Result<Vec<models::Account>, String> {
    antigravity_import_call(
        "accounts.importJson",
        json!({ "jsonContent": json_content }),
    )
    .await
}

#[tauri::command]
pub async fn import_from_files(
    file_paths: Vec<String>,
) -> Result<modules::import::FileImportResult, String> {
    antigravity_import_call("accounts.importFiles", json!({ "filePaths": file_paths })).await
}

#[tauri::command]
pub async fn export_accounts(account_ids: Vec<String>) -> Result<String, String> {
    antigravity_import_call("accounts.export", json!({ "accountIds": account_ids })).await
}
