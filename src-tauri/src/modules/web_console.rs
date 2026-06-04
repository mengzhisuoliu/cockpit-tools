//! Local browser console for the full Cockpit Tools UI.
//! This is intentionally separate from `web_report`, which remains the tokened report endpoint.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};
use std::sync::{OnceLock, RwLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{timeout, Duration};
use url::Url;

use super::config::PORT_RANGE;

const DEFAULT_WEB_CONSOLE_PORT: u16 = 18081;
const MAX_HTTP_REQUEST_BYTES: usize = 2 * 1024 * 1024;
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(8);
const INDEX_HTML: &str = "index.html";

static ACTUAL_WEB_CONSOLE_PORT: OnceLock<RwLock<Option<u16>>> = OnceLock::new();

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

#[derive(Debug, Deserialize)]
struct InvokeRequest {
    cmd: String,
    #[serde(default)]
    args: Value,
}

#[derive(Debug, Serialize)]
struct InvokeResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

fn web_console_port_state() -> &'static RwLock<Option<u16>> {
    ACTUAL_WEB_CONSOLE_PORT.get_or_init(|| RwLock::new(None))
}

fn set_actual_port(port: Option<u16>) {
    if let Ok(mut guard) = web_console_port_state().write() {
        *guard = port;
    }
}

pub fn get_actual_port() -> Option<u16> {
    web_console_port_state()
        .read()
        .ok()
        .and_then(|guard| *guard)
}

pub async fn start_server() {
    let Some(dist_root) = find_frontend_dist() else {
        set_actual_port(None);
        super::logger::log_warn("[WebConsole] frontend dist directory not found, skip startup");
        return;
    };

    let mut port = DEFAULT_WEB_CONSOLE_PORT;
    let mut listener = None;
    for attempt in 0..PORT_RANGE {
        let addr = format!("127.0.0.1:{}", port);
        match TcpListener::bind(&addr).await {
            Ok(bound) => {
                listener = Some(bound);
                if attempt > 0 {
                    super::logger::log_info(&format!(
                        "[WebConsole] preferred port {} is busy, switched to {}",
                        DEFAULT_WEB_CONSOLE_PORT, port
                    ));
                }
                break;
            }
            Err(err) => {
                super::logger::log_warn(&format!(
                    "[WebConsole] failed to bind 127.0.0.1:{}: {}",
                    port, err
                ));
                port = port.saturating_add(1);
            }
        }
    }

    let Some(listener) = listener else {
        set_actual_port(None);
        super::logger::log_error("[WebConsole] no available local port");
        return;
    };

    set_actual_port(Some(port));
    super::logger::log_info(&format!(
        "[WebConsole] serving full UI at http://127.0.0.1:{}/",
        port
    ));

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let dist_root = dist_root.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_connection(stream, dist_root).await {
                        super::logger::log_warn(&format!("[WebConsole] request failed: {}", err));
                    }
                });
            }
            Err(err) => {
                super::logger::log_warn(&format!("[WebConsole] accept failed: {}", err));
            }
        }
    }
}

async fn handle_connection(mut stream: TcpStream, dist_root: PathBuf) -> Result<(), String> {
    let Some(request) = read_http_request(&mut stream).await? else {
        return Ok(());
    };

    if request.method == "OPTIONS" {
        return write_response(
            &mut stream,
            204,
            "No Content",
            "text/plain; charset=utf-8",
            b"",
        )
        .await;
    }

    if request.method == "POST" && request.path == "/__cockpit_web__/invoke" {
        return handle_invoke_request(&mut stream, &request).await;
    }

    if request.method == "GET" && request.path == "/__cockpit_web__/health" {
        let body = json!({
            "ok": true,
            "port": get_actual_port(),
            "version": env!("CARGO_PKG_VERSION"),
        });
        let body = serde_json::to_vec(&body).map_err(|err| err.to_string())?;
        return write_response(
            &mut stream,
            200,
            "OK",
            "application/json; charset=utf-8",
            &body,
        )
        .await;
    }

    if request.method != "GET" && request.method != "HEAD" {
        return write_response(
            &mut stream,
            405,
            "Method Not Allowed",
            "text/plain; charset=utf-8",
            b"method not allowed",
        )
        .await;
    }

    let file_path = resolve_static_path(&dist_root, &request.path)?;
    let (file_path, content_type) = if file_path.exists() && file_path.is_file() {
        let content_type = content_type_for_path(&file_path);
        (file_path, content_type)
    } else {
        (dist_root.join(INDEX_HTML), "text/html; charset=utf-8")
    };

    let body = tokio::fs::read(&file_path)
        .await
        .map_err(|err| format!("read {} failed: {}", file_path.display(), err))?;
    if request.method == "HEAD" {
        return write_response(&mut stream, 200, "OK", content_type, b"").await;
    }
    write_response(&mut stream, 200, "OK", content_type, &body).await
}

async fn handle_invoke_request(
    stream: &mut TcpStream,
    request: &HttpRequest,
) -> Result<(), String> {
    let invoke: InvokeRequest =
        serde_json::from_slice(&request.body).map_err(|err| format!("invalid JSON: {}", err))?;
    let response = match dispatch_invoke(&invoke.cmd, &invoke.args).await {
        Ok(value) => InvokeResponse {
            ok: true,
            value: Some(value),
            error: None,
        },
        Err(error) => InvokeResponse {
            ok: false,
            value: None,
            error: Some(Value::String(error)),
        },
    };
    let status = if response.ok { 200 } else { 400 };
    let body = serde_json::to_vec(&response).map_err(|err| err.to_string())?;
    write_response(
        stream,
        status,
        if status == 200 { "OK" } else { "Bad Request" },
        "application/json; charset=utf-8",
        &body,
    )
    .await
}

async fn dispatch_invoke(cmd: &str, args: &Value) -> Result<Value, String> {
    match cmd {
        "plugin:app|version" => Ok(Value::String(env!("CARGO_PKG_VERSION").to_string())),
        "plugin:app|name" => Ok(Value::String("Cockpit Tools".to_string())),
        "plugin:app|identifier" => Ok(Value::String("com.jlcodes.cockpit-tools".to_string())),
        "plugin:app|tauri_version" => Ok(Value::String("2".to_string())),
        "plugin:event|listen" => Ok(json!(1)),
        "plugin:event|unlisten" | "plugin:event|emit" | "plugin:event|emit_to" => Ok(Value::Null),
        "plugin:window|get_all_windows" => Ok(json!([{ "label": "main" }])),
        "plugin:webview|get_all_webviews" => {
            Ok(json!([{ "label": "main", "windowLabel": "main" }]))
        }
        "plugin:window|start_dragging"
        | "plugin:window|set_theme"
        | "plugin:webview|set_webview_zoom"
        | "plugin:webview|set_zoom" => Ok(Value::Null),

        "list_accounts" => to_value(crate::commands::account::list_accounts().await),
        "get_current_account" => to_value(crate::commands::account::get_current_account().await),
        "set_current_account" => to_value(
            crate::commands::account::set_current_account(app_handle()?, arg(args, "accountId")?)
                .await,
        ),
        "fetch_account_quota" => to_value(
            crate::commands::account::fetch_account_quota(arg(args, "accountId")?)
                .await
                .map_err(|err| err.to_string()),
        ),
        "refresh_all_quotas" => {
            to_value(crate::commands::account::refresh_all_quotas(app_handle()?).await)
        }
        "refresh_current_quota" => {
            to_value(crate::commands::account::refresh_current_quota(app_handle()?).await)
        }
        "switch_account" => to_value(
            crate::commands::account::switch_account(
                app_handle()?,
                arg(args, "accountId")?,
                opt_arg(args, "runtimeTarget")?,
            )
            .await,
        ),
        "update_account_tags" => to_value(
            crate::commands::account::update_account_tags(
                arg(args, "accountId")?,
                arg(args, "tags")?,
            )
            .await,
        ),
        "update_account_notes" => to_value(
            crate::commands::account::update_account_notes(
                arg(args, "accountId")?,
                arg(args, "notes")?,
            )
            .await,
        ),
        "load_account_groups" => to_value(crate::commands::account::load_account_groups().await),
        "save_account_groups" => {
            to_value(crate::commands::account::save_account_groups(arg(args, "data")?).await)
        }

        "list_codex_accounts" => to_value(crate::commands::codex::list_codex_accounts()),
        "get_current_codex_account" => {
            to_value(crate::commands::codex::get_current_codex_account())
        }
        "refresh_current_codex_quota" => {
            to_value(crate::commands::codex::refresh_current_codex_quota(app_handle()?).await)
        }
        "refresh_codex_quota" => to_value(
            crate::commands::codex::refresh_codex_quota(app_handle()?, arg(args, "accountId")?)
                .await,
        ),
        "refresh_codex_subscription_info" => to_value(
            crate::commands::codex::refresh_codex_subscription_info(
                app_handle()?,
                arg(args, "accountId")?,
            )
            .await,
        ),
        "refresh_codex_account_profile" => to_value(
            crate::commands::codex::refresh_codex_account_profile(arg(args, "accountId")?).await,
        ),
        "refresh_all_codex_quotas" => {
            to_value(crate::commands::codex::refresh_all_codex_quotas(app_handle()?).await)
        }
        "switch_codex_account" => to_value(
            crate::commands::codex::switch_codex_account(app_handle()?, arg(args, "accountId")?)
                .await,
        ),
        "load_codex_account_groups" => {
            to_value(crate::commands::codex::load_codex_account_groups().await)
        }
        "save_codex_account_groups" => {
            to_value(crate::commands::codex::save_codex_account_groups(arg(args, "data")?).await)
        }
        "get_codex_quick_config" => to_value(crate::commands::codex::get_codex_quick_config()),
        "save_codex_quick_config" => to_value(crate::commands::codex::save_codex_quick_config(
            opt_arg(args, "modelContextWindow")?,
            opt_arg(args, "autoCompactTokenLimit")?,
        )),
        "get_codex_app_speed_config" => {
            to_value(crate::commands::codex::get_codex_app_speed_config())
        }
        "save_codex_app_speed" => to_value(crate::commands::codex::save_codex_app_speed(arg(
            args, "speed",
        )?)),
        "get_codex_api_service_app_speed_config" => {
            to_value(crate::commands::codex::get_codex_api_service_app_speed_config())
        }
        "save_codex_api_service_app_speed" => to_value(
            crate::commands::codex::save_codex_api_service_app_speed(arg(args, "speed")?),
        ),
        "codex_local_access_get_state" => {
            to_value(crate::commands::codex::codex_local_access_get_state().await)
        }

        "list_github_copilot_accounts" => {
            to_value(crate::commands::github_copilot::list_github_copilot_accounts())
        }
        "refresh_github_copilot_token" => to_value(
            crate::commands::github_copilot::refresh_github_copilot_token(
                app_handle()?,
                arg(args, "accountId")?,
            )
            .await,
        ),
        "refresh_all_github_copilot_tokens" => to_value(
            crate::commands::github_copilot::refresh_all_github_copilot_tokens(app_handle()?).await,
        ),
        "inject_github_copilot_to_vscode" => to_value(
            crate::commands::github_copilot::inject_github_copilot_to_vscode(
                app_handle()?,
                arg(args, "accountId")?,
            )
            .await,
        ),
        "update_github_copilot_account_tags" => to_value(
            crate::commands::github_copilot::update_github_copilot_account_tags(
                arg(args, "accountId")?,
                arg(args, "tags")?,
            )
            .await,
        ),
        "list_windsurf_accounts" => to_value(crate::commands::windsurf::list_windsurf_accounts()),
        "refresh_windsurf_token" => to_value(
            crate::commands::windsurf::refresh_windsurf_token(
                app_handle()?,
                arg(args, "accountId")?,
            )
            .await,
        ),
        "refresh_all_windsurf_tokens" => {
            to_value(crate::commands::windsurf::refresh_all_windsurf_tokens(app_handle()?).await)
        }
        "inject_windsurf_to_vscode" => to_value(
            crate::commands::windsurf::inject_windsurf_to_vscode(
                app_handle()?,
                arg(args, "accountId")?,
            )
            .await,
        ),
        "update_windsurf_account_tags" => to_value(
            crate::commands::windsurf::update_windsurf_account_tags(
                arg(args, "accountId")?,
                arg(args, "tags")?,
            )
            .await,
        ),
        "list_kiro_accounts" => to_value(crate::commands::kiro::list_kiro_accounts()),
        "refresh_kiro_token" => to_value(
            crate::commands::kiro::refresh_kiro_token(app_handle()?, arg(args, "accountId")?).await,
        ),
        "refresh_all_kiro_tokens" => {
            to_value(crate::commands::kiro::refresh_all_kiro_tokens(app_handle()?).await)
        }
        "inject_kiro_to_vscode" => to_value(
            crate::commands::kiro::inject_kiro_to_vscode(app_handle()?, arg(args, "accountId")?)
                .await,
        ),
        "update_kiro_account_tags" => to_value(
            crate::commands::kiro::update_kiro_account_tags(
                arg(args, "accountId")?,
                arg(args, "tags")?,
            )
            .await,
        ),
        "list_cursor_accounts" => to_value(crate::commands::cursor::list_cursor_accounts()),
        "refresh_cursor_token" => to_value(
            crate::commands::cursor::refresh_cursor_token(app_handle()?, arg(args, "accountId")?)
                .await,
        ),
        "refresh_all_cursor_tokens" => {
            to_value(crate::commands::cursor::refresh_all_cursor_tokens(app_handle()?).await)
        }
        "inject_cursor_account" => to_value(
            crate::commands::cursor::inject_cursor_account(app_handle()?, arg(args, "accountId")?)
                .await,
        ),
        "update_cursor_account_tags" => to_value(
            crate::commands::cursor::update_cursor_account_tags(
                arg(args, "accountId")?,
                arg(args, "tags")?,
            )
            .await,
        ),
        "list_gemini_accounts" => to_value(crate::commands::gemini::list_gemini_accounts()),
        "refresh_gemini_token" => to_value(
            crate::commands::gemini::refresh_gemini_token(app_handle()?, arg(args, "accountId")?)
                .await,
        ),
        "refresh_all_gemini_tokens" => {
            to_value(crate::commands::gemini::refresh_all_gemini_tokens(app_handle()?).await)
        }
        "inject_gemini_account" => to_value(crate::commands::gemini::inject_gemini_account(
            app_handle()?,
            arg(args, "accountId")?,
        )),
        "update_gemini_account_tags" => {
            to_value(crate::commands::gemini::update_gemini_account_tags(
                arg(args, "accountId")?,
                arg(args, "tags")?,
            ))
        }
        "list_codebuddy_accounts" => {
            to_value(crate::commands::codebuddy::list_codebuddy_accounts())
        }
        "refresh_codebuddy_token" => to_value(
            crate::commands::codebuddy::refresh_codebuddy_token(
                app_handle()?,
                arg(args, "accountId")?,
            )
            .await,
        ),
        "refresh_all_codebuddy_tokens" => {
            to_value(crate::commands::codebuddy::refresh_all_codebuddy_tokens(app_handle()?).await)
        }
        "inject_codebuddy_to_vscode" => to_value(
            crate::commands::codebuddy::inject_codebuddy_to_vscode(
                app_handle()?,
                arg(args, "accountId")?,
            )
            .await,
        ),
        "update_codebuddy_account_tags" => to_value(
            crate::commands::codebuddy::update_codebuddy_account_tags(
                arg(args, "accountId")?,
                arg(args, "tags")?,
            )
            .await,
        ),
        "list_codebuddy_cn_accounts" => {
            to_value(crate::commands::codebuddy_cn::list_codebuddy_cn_accounts())
        }
        "refresh_codebuddy_cn_token" => to_value(
            crate::commands::codebuddy_cn::refresh_codebuddy_cn_token(
                app_handle()?,
                arg(args, "accountId")?,
            )
            .await,
        ),
        "refresh_all_codebuddy_cn_tokens" => to_value(
            crate::commands::codebuddy_cn::refresh_all_codebuddy_cn_tokens(app_handle()?).await,
        ),
        "inject_codebuddy_cn_to_vscode" => to_value(
            crate::commands::codebuddy_cn::inject_codebuddy_cn_to_vscode(
                app_handle()?,
                arg(args, "accountId")?,
            )
            .await,
        ),
        "update_codebuddy_cn_account_tags" => to_value(
            crate::commands::codebuddy_cn::update_codebuddy_cn_account_tags(
                arg(args, "accountId")?,
                arg(args, "tags")?,
            )
            .await,
        ),
        "list_qoder_accounts" => to_value(crate::commands::qoder::list_qoder_accounts()),
        "refresh_qoder_token" => to_value(
            crate::commands::qoder::refresh_qoder_token(app_handle()?, arg(args, "accountId")?)
                .await,
        ),
        "refresh_all_qoder_tokens" => {
            to_value(crate::commands::qoder::refresh_all_qoder_tokens(app_handle()?).await)
        }
        "inject_qoder_account" => to_value(
            crate::commands::qoder::inject_qoder_account(app_handle()?, arg(args, "accountId")?)
                .await,
        ),
        "update_qoder_account_tags" => to_value(crate::commands::qoder::update_qoder_account_tags(
            arg(args, "accountId")?,
            arg(args, "tags")?,
        )),
        "list_trae_accounts" => to_value(crate::commands::trae::list_trae_accounts()),
        "refresh_trae_token" => to_value(
            crate::commands::trae::refresh_trae_token(app_handle()?, arg(args, "accountId")?).await,
        ),
        "refresh_all_trae_tokens" => {
            to_value(crate::commands::trae::refresh_all_trae_tokens(app_handle()?).await)
        }
        "inject_trae_account" => to_value(
            crate::commands::trae::inject_trae_account(app_handle()?, arg(args, "accountId")?)
                .await,
        ),
        "update_trae_account_tags" => to_value(
            crate::commands::trae::update_trae_account_tags(
                arg(args, "accountId")?,
                arg(args, "tags")?,
            )
            .await,
        ),
        "list_workbuddy_accounts" => {
            to_value(crate::commands::workbuddy::list_workbuddy_accounts())
        }
        "refresh_workbuddy_token" => to_value(
            crate::commands::workbuddy::refresh_workbuddy_token(
                app_handle()?,
                arg(args, "accountId")?,
            )
            .await,
        ),
        "refresh_all_workbuddy_tokens" => {
            to_value(crate::commands::workbuddy::refresh_all_workbuddy_tokens(app_handle()?).await)
        }
        "inject_workbuddy_to_vscode" => to_value(
            crate::commands::workbuddy::inject_workbuddy_to_vscode(
                app_handle()?,
                arg(args, "accountId")?,
            )
            .await,
        ),
        "update_workbuddy_account_tags" => to_value(
            crate::commands::workbuddy::update_workbuddy_account_tags(
                arg(args, "accountId")?,
                arg(args, "tags")?,
            )
            .await,
        ),
        "list_zed_accounts" => to_value(crate::commands::zed::list_zed_accounts()),
        "refresh_zed_token" => to_value(
            crate::commands::zed::refresh_zed_token(app_handle()?, arg(args, "accountId")?).await,
        ),
        "refresh_all_zed_tokens" => {
            to_value(crate::commands::zed::refresh_all_zed_tokens(app_handle()?).await)
        }
        "inject_zed_account" => to_value(
            crate::commands::zed::inject_zed_account(app_handle()?, arg(args, "accountId")?).await,
        ),
        "update_zed_account_tags" => to_value(crate::commands::zed::update_zed_account_tags(
            arg(args, "accountId")?,
            arg(args, "tags")?,
        )),

        "get_provider_current_account_id" => to_value(
            crate::commands::provider_current::get_provider_current_account_id(
                app_handle()?,
                arg(args, "platform")?,
            )
            .await,
        ),

        "get_network_config" => to_value(crate::commands::system::get_network_config()),
        "save_network_config" => to_value(crate::commands::system::save_network_config(
            arg(args, "wsEnabled")?,
            arg(args, "wsPort")?,
            opt_arg(args, "reportEnabled")?,
            opt_arg(args, "reportPort")?,
            opt_arg(args, "reportToken")?,
            opt_arg(args, "globalProxyEnabled")?,
            opt_arg(args, "globalProxyUrl")?,
            opt_arg(args, "globalProxyNoProxy")?,
        )),
        "get_general_config" => {
            to_value(crate::commands::system::get_general_config(app_handle()?))
        }
        "save_general_config" => dispatch_save_general_config(args),
        "get_available_terminals" => {
            to_value(crate::commands::system::get_available_terminals().await)
        }
        "set_app_path" => to_value(crate::commands::system::set_app_path(
            arg(args, "app")?,
            arg(args, "path")?,
        )),
        "set_codex_launch_on_switch" => to_value(
            crate::commands::system::set_codex_launch_on_switch(arg(args, "enabled")?),
        ),
        "set_codex_local_access_entry_visible" => to_value(
            crate::commands::system::set_codex_local_access_entry_visible(arg(args, "enabled")?),
        ),
        "save_tray_platform_layout" => {
            to_value(crate::commands::system::save_tray_platform_layout(
                app_handle()?,
                arg(args, "sortMode")?,
                arg(args, "orderedPlatformIds")?,
                arg(args, "trayPlatformIds")?,
                opt_arg(args, "orderedEntryIds")?,
                opt_arg(args, "platformGroups")?,
            ))
        }
        "set_wakeup_override" => to_value(crate::commands::system::set_wakeup_override(arg(
            args, "enabled",
        )?)),
        "external_import_take_pending" => {
            serialize_value(crate::commands::system::external_import_take_pending())
        }
        "external_import_fetch_import_url" => to_value(
            crate::commands::system::external_import_fetch_import_url(arg(args, "importUrl")?)
                .await,
        ),
        "detect_app_path" => to_value(crate::commands::system::detect_app_path(
            arg(args, "app")?,
            opt_arg(args, "force")?,
        )),
        "get_antigravity_installed_version_info" => to_value(
            crate::commands::system::get_antigravity_installed_version_info(
                opt_arg(args, "runtimeTarget")?,
                opt_arg(args, "scanMode")?,
            )
            .await,
        ),
        "get_auto_backup_settings" => to_value(crate::commands::system::get_auto_backup_settings()),
        "save_auto_backup_settings" => {
            to_value(crate::commands::system::save_auto_backup_settings(
                arg(args, "enabled")?,
                arg(args, "includeAccounts")?,
                arg(args, "includeConfig")?,
                arg(args, "retentionDays")?,
            ))
        }
        "update_auto_backup_last_run" => to_value(
            crate::commands::system::update_auto_backup_last_run(opt_arg(args, "lastBackupAt")?),
        ),
        "write_auto_backup_file" => to_value(crate::commands::system::write_auto_backup_file(
            arg(args, "fileName")?,
            arg(args, "content")?,
        )),
        "read_auto_backup_file" => to_value(crate::commands::system::read_auto_backup_file(arg(
            args, "fileName",
        )?)),
        "copy_auto_backup_file" => to_value(crate::commands::system::copy_auto_backup_file(
            arg(args, "fileName")?,
            arg(args, "targetPath")?,
        )),
        "list_auto_backup_files" => to_value(crate::commands::system::list_auto_backup_files()),
        "delete_auto_backup_file" => to_value(crate::commands::system::delete_auto_backup_file(
            arg(args, "fileName")?,
        )),
        "cleanup_auto_backup_files" => to_value(
            crate::commands::system::cleanup_auto_backup_files(arg(args, "retentionDays")?),
        ),
        "open_auto_backup_dir" => to_value(crate::commands::system::open_auto_backup_dir()),
        "open_data_folder" => to_value(crate::commands::system::open_data_folder().await),
        "open_folder" => to_value(crate::commands::system::open_folder(arg(args, "path")?).await),
        "show_floating_card_window" => to_value(
            crate::commands::system::show_floating_card_window(app_handle()?),
        ),
        "show_instance_floating_card_window" => {
            to_value(crate::commands::system::show_instance_floating_card_window(
                app_handle()?,
                arg(args, "context")?,
            ))
        }
        "get_floating_card_context" => to_value(
            crate::commands::system::get_floating_card_context(arg(args, "windowLabel")?),
        ),
        "hide_floating_card_window" => to_value(
            crate::commands::system::hide_floating_card_window(app_handle()?),
        ),
        "hide_current_floating_card_window" => Ok(Value::Null),
        "set_floating_card_always_on_top" => {
            to_value(crate::commands::system::set_floating_card_always_on_top(
                app_handle()?,
                arg(args, "alwaysOnTop")?,
            ))
        }
        "set_current_floating_card_window_always_on_top" => Ok(Value::Null),
        "set_floating_card_confirm_on_close" => {
            to_value(crate::commands::system::set_floating_card_confirm_on_close(
                arg(args, "confirmOnClose")?,
            ))
        }
        "save_floating_card_position" => to_value(
            crate::commands::system::save_floating_card_position(arg(args, "x")?, arg(args, "y")?),
        ),
        "show_main_window_and_navigate" => {
            to_value(crate::commands::system::show_main_window_and_navigate(
                app_handle()?,
                arg(args, "page")?,
            ))
        }
        "logs_get_snapshot" => to_value(crate::commands::logs::logs_get_snapshot(
            opt_arg(args, "fileName")?,
            Some(arg_or(args, "lineLimit", 500usize)?),
        )),
        "logs_open_log_directory" => to_value(crate::commands::logs::logs_open_log_directory()),

        "wakeup_ensure_runtime_ready" => {
            to_value(crate::commands::wakeup::wakeup_ensure_runtime_ready(
                opt_arg(args, "officialLsVersionMode")?,
            ))
        }
        "wakeup_set_official_ls_version_mode" => to_value(
            crate::commands::wakeup::wakeup_set_official_ls_version_mode(opt_arg(args, "mode")?),
        ),
        "trigger_wakeup" => to_value(
            crate::commands::wakeup::trigger_wakeup(
                arg(args, "accountId")?,
                arg(args, "model")?,
                opt_arg(args, "prompt")?,
                opt_arg(args, "maxOutputTokens")?,
                opt_arg(args, "cancelScopeId")?,
                opt_arg(args, "officialLsVersionMode")?,
            )
            .await,
        ),
        "fetch_available_models" => {
            to_value(crate::commands::wakeup::fetch_available_models().await)
        }
        "wakeup_validate_crontab" => to_value(crate::commands::wakeup::wakeup_validate_crontab(
            arg(args, "expr")?,
        )),
        "wakeup_sync_state" => to_value(
            crate::commands::wakeup::wakeup_sync_state(
                app_handle()?,
                arg(args, "enabled")?,
                arg(args, "tasks")?,
                opt_arg(args, "officialLsVersionMode")?,
                opt_arg(args, "runStartupTasks")?,
            )
            .await,
        ),
        "wakeup_run_enabled_tasks" => to_value(
            crate::commands::wakeup::wakeup_run_enabled_tasks(
                app_handle()?,
                opt_arg(args, "triggerSource")?,
                opt_arg(args, "officialLsVersionMode")?,
            )
            .await,
        ),
        "wakeup_load_history" => to_value(crate::commands::wakeup::wakeup_load_history()),
        "wakeup_add_history" => to_value(crate::commands::wakeup::wakeup_add_history(arg(
            args, "items",
        )?)),
        "wakeup_clear_history" => to_value(crate::commands::wakeup::wakeup_clear_history()),
        "wakeup_cancel_scope" => to_value(crate::commands::wakeup::wakeup_cancel_scope(arg(
            args,
            "cancelScopeId",
        )?)),
        "wakeup_release_scope" => to_value(crate::commands::wakeup::wakeup_release_scope(arg(
            args,
            "cancelScopeId",
        )?)),
        "wakeup_verification_load_state" => {
            to_value(crate::commands::wakeup::wakeup_verification_load_state())
        }
        "wakeup_verification_load_history" => {
            to_value(crate::commands::wakeup::wakeup_verification_load_history())
        }
        "wakeup_verification_delete_history" => to_value(
            crate::commands::wakeup::wakeup_verification_delete_history(arg(args, "batchIds")?),
        ),
        "wakeup_verification_run_batch" => to_value(
            crate::commands::wakeup::wakeup_verification_run_batch(
                app_handle()?,
                arg(args, "accountIds")?,
                arg(args, "model")?,
                opt_arg(args, "prompt")?,
                opt_arg(args, "maxOutputTokens")?,
                opt_arg(args, "officialLsVersionMode")?,
            )
            .await,
        ),
        "confirm_wakeup_task" => to_value(
            crate::commands::wakeup::confirm_wakeup_task(app_handle()?, arg(args, "taskId")?).await,
        ),
        "cancel_wakeup_task" => {
            to_value(crate::commands::wakeup::cancel_wakeup_task(arg(args, "taskId")?).await)
        }
        "check_wakeup_timeouts" => {
            to_value(crate::commands::wakeup::check_wakeup_timeouts(app_handle()?).await)
        }

        "codex_wakeup_get_cli_status" => {
            to_value(crate::commands::codex::codex_wakeup_get_cli_status())
        }
        "codex_wakeup_update_runtime_config" => {
            to_value(crate::commands::codex::codex_wakeup_update_runtime_config(
                opt_arg(args, "codexCliPath")?,
                opt_arg(args, "nodePath")?,
            ))
        }
        "codex_wakeup_get_overview" => {
            to_value(crate::commands::codex::codex_wakeup_get_overview())
        }
        "codex_wakeup_get_state" => to_value(crate::commands::codex::codex_wakeup_get_state()),
        "codex_wakeup_save_state" => to_value(crate::commands::codex::codex_wakeup_save_state(
            arg(args, "enabled")?,
            arg(args, "tasks")?,
            arg(args, "modelPresets")?,
            arg(args, "modelPresetMigrations")?,
        )),
        "codex_wakeup_load_history" => {
            to_value(crate::commands::codex::codex_wakeup_load_history())
        }
        "codex_wakeup_clear_history" => {
            to_value(crate::commands::codex::codex_wakeup_clear_history())
        }
        "codex_wakeup_cancel_scope" => to_value(crate::commands::codex::codex_wakeup_cancel_scope(
            arg(args, "cancelScopeId")?,
        )),
        "codex_wakeup_release_scope" => to_value(
            crate::commands::codex::codex_wakeup_release_scope(arg(args, "cancelScopeId")?),
        ),
        "codex_wakeup_test" => to_value(
            crate::commands::codex::codex_wakeup_test(
                app_handle()?,
                arg(args, "accountIds")?,
                opt_arg(args, "prompt")?,
                opt_arg(args, "model")?,
                opt_arg(args, "modelDisplayName")?,
                opt_arg(args, "modelReasoningEffort")?,
                opt_arg(args, "runId")?,
                opt_arg(args, "cancelScopeId")?,
            )
            .await,
        ),
        "codex_wakeup_run_task" => to_value(
            crate::commands::codex::codex_wakeup_run_task(
                app_handle()?,
                arg(args, "taskId")?,
                opt_arg(args, "runId")?,
            )
            .await,
        ),
        "codex_wakeup_run_enabled_tasks" => to_value(
            crate::commands::codex::codex_wakeup_run_enabled_tasks(
                app_handle()?,
                opt_arg(args, "triggerType")?,
            )
            .await,
        ),

        "get_update_settings" => to_value(crate::commands::update::get_update_settings()),
        "save_update_settings" => to_value(crate::commands::update::save_update_settings(arg(
            args, "settings",
        )?)),
        "should_check_updates" => to_value(crate::commands::update::should_check_updates()),
        "update_last_check_time" => to_value(crate::commands::update::update_last_check_time()),
        "check_version_jump" => to_value(crate::commands::update::check_version_jump()),
        "get_release_history" => to_value(crate::commands::update::get_release_history(
            opt_arg(args, "locale")?,
            opt_arg(args, "limit")?,
        )),
        "update_log" => to_value(crate::commands::update::update_log(
            arg(args, "level")?,
            arg(args, "message")?,
        )),
        "get_update_runtime_info" => to_value(crate::commands::update::get_update_runtime_info()),

        "announcement_get_state" => {
            to_value(crate::commands::announcement::announcement_get_state().await)
        }
        "announcement_mark_as_read" => to_value(
            crate::commands::announcement::announcement_mark_as_read(arg(args, "id")?).await,
        ),
        "announcement_mark_all_as_read" => {
            to_value(crate::commands::announcement::announcement_mark_all_as_read().await)
        }
        "announcement_force_refresh" => {
            to_value(crate::commands::announcement::announcement_force_refresh().await)
        }
        "announcement_get_top_right_ad" => {
            to_value(crate::commands::announcement::announcement_get_top_right_ad().await)
        }

        "get_group_settings" => to_value(crate::commands::group::get_group_settings()),
        "get_display_groups" => to_value(crate::commands::group::get_display_groups()),

        "codex_get_instance_defaults" => {
            to_value(crate::commands::codex_instance::codex_get_instance_defaults().await)
        }
        "codex_list_instances" => {
            to_value(crate::commands::codex_instance::codex_list_instances().await)
        }
        "github_copilot_get_instance_defaults" => to_value(
            crate::commands::github_copilot_instance::github_copilot_get_instance_defaults().await,
        ),
        "github_copilot_list_instances" => to_value(
            crate::commands::github_copilot_instance::github_copilot_list_instances().await,
        ),
        "windsurf_get_instance_defaults" => {
            to_value(crate::commands::windsurf_instance::windsurf_get_instance_defaults().await)
        }
        "windsurf_list_instances" => {
            to_value(crate::commands::windsurf_instance::windsurf_list_instances().await)
        }
        "kiro_get_instance_defaults" => {
            to_value(crate::commands::kiro_instance::kiro_get_instance_defaults().await)
        }
        "kiro_list_instances" => {
            to_value(crate::commands::kiro_instance::kiro_list_instances().await)
        }
        "cursor_get_instance_defaults" => {
            to_value(crate::commands::cursor_instance::cursor_get_instance_defaults().await)
        }
        "cursor_list_instances" => {
            to_value(crate::commands::cursor_instance::cursor_list_instances().await)
        }
        "gemini_get_instance_defaults" => {
            to_value(crate::commands::gemini_instance::gemini_get_instance_defaults().await)
        }
        "gemini_list_instances" => {
            to_value(crate::commands::gemini_instance::gemini_list_instances().await)
        }
        "codebuddy_get_instance_defaults" => {
            to_value(crate::commands::codebuddy_instance::codebuddy_get_instance_defaults().await)
        }
        "codebuddy_list_instances" => {
            to_value(crate::commands::codebuddy_instance::codebuddy_list_instances().await)
        }
        "codebuddy_cn_get_instance_defaults" => to_value(
            crate::commands::codebuddy_cn_instance::codebuddy_cn_get_instance_defaults().await,
        ),
        "codebuddy_cn_list_instances" => {
            to_value(crate::commands::codebuddy_cn_instance::codebuddy_cn_list_instances().await)
        }
        "qoder_get_instance_defaults" => {
            to_value(crate::commands::qoder_instance::qoder_get_instance_defaults().await)
        }
        "qoder_list_instances" => {
            to_value(crate::commands::qoder_instance::qoder_list_instances().await)
        }
        "trae_get_instance_defaults" => {
            to_value(crate::commands::trae_instance::trae_get_instance_defaults().await)
        }
        "trae_list_instances" => {
            to_value(crate::commands::trae_instance::trae_list_instances().await)
        }
        "workbuddy_get_instance_defaults" => {
            to_value(crate::commands::workbuddy_instance::workbuddy_get_instance_defaults().await)
        }
        "workbuddy_list_instances" => {
            to_value(crate::commands::workbuddy_instance::workbuddy_list_instances().await)
        }

        other => Err(format!(
            "Command '{}' is not exposed through the local web console yet",
            other
        )),
    }
}

fn to_value<T: Serialize>(result: Result<T, String>) -> Result<Value, String> {
    serde_json::to_value(result?).map_err(|err| format!("serialize response failed: {}", err))
}

fn serialize_value<T: Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|err| format!("serialize response failed: {}", err))
}

fn dispatch_save_general_config(args: &Value) -> Result<Value, String> {
    to_value(crate::commands::system::save_general_config(
        app_handle()?,
        arg(args, "language")?,
        opt_arg(args, "defaultTerminal")?,
        arg(args, "theme")?,
        opt_arg(args, "uiScale")?,
        arg(args, "autoRefreshMinutes")?,
        arg(args, "codexAutoRefreshMinutes")?,
        opt_arg(args, "zedAutoRefreshMinutes")?,
        opt_arg(args, "ghcpAutoRefreshMinutes")?,
        opt_arg(args, "windsurfAutoRefreshMinutes")?,
        opt_arg(args, "kiroAutoRefreshMinutes")?,
        opt_arg(args, "cursorAutoRefreshMinutes")?,
        opt_arg(args, "geminiAutoRefreshMinutes")?,
        opt_arg(args, "geminiSyncWsl")?,
        opt_arg(args, "codebuddyAutoRefreshMinutes")?,
        opt_arg(args, "codebuddyCnAutoRefreshMinutes")?,
        opt_arg(args, "workbuddyAutoRefreshMinutes")?,
        opt_arg(args, "qoderAutoRefreshMinutes")?,
        opt_arg(args, "traeAutoRefreshMinutes")?,
        arg(args, "closeBehavior")?,
        opt_arg(args, "minimizeBehavior")?,
        opt_arg(args, "hideDockIcon")?,
        opt_arg(args, "trayIconStyle")?,
        opt_arg(args, "floatingCardShowOnStartup")?,
        opt_arg(args, "floatingCardAlwaysOnTop")?,
        opt_arg(args, "appAutoLaunchEnabled")?,
        opt_arg(args, "antigravityStartupWakeupEnabled")?,
        opt_arg(args, "antigravityStartupWakeupDelaySeconds")?,
        opt_arg(args, "codexStartupWakeupEnabled")?,
        opt_arg(args, "codexStartupWakeupDelaySeconds")?,
        opt_arg(args, "floatingCardConfirmOnClose")?,
        arg(args, "opencodeAppPath")?,
        arg(args, "antigravityAppPath")?,
        arg(args, "codexAppPath")?,
        opt_arg(args, "codexSpecifiedAppPath")?,
        opt_arg(args, "zedAppPath")?,
        arg(args, "vscodeAppPath")?,
        opt_arg(args, "windsurfAppPath")?,
        opt_arg(args, "kiroAppPath")?,
        opt_arg(args, "cursorAppPath")?,
        opt_arg(args, "codebuddyAppPath")?,
        opt_arg(args, "codebuddyCnAppPath")?,
        opt_arg(args, "qoderAppPath")?,
        opt_arg(args, "traeAppPath")?,
        opt_arg(args, "workbuddyAppPath")?,
        arg(args, "opencodeSyncOnSwitch")?,
        opt_arg(args, "opencodeAuthOverwriteOnSwitch")?,
        opt_arg(args, "ghcpOpencodeSyncOnSwitch")?,
        opt_arg(args, "ghcpOpencodeAuthOverwriteOnSwitch")?,
        opt_arg(args, "ghcpLaunchOnSwitch")?,
        opt_arg(args, "openclawAuthOverwriteOnSwitch")?,
        arg(args, "codexLaunchOnSwitch")?,
        opt_arg(args, "codexRestartSpecifiedAppOnSwitch")?,
        opt_arg(args, "codexLocalAccessEntryVisible")?,
        opt_arg(args, "antigravityDualSwitchNoRestartEnabled")?,
        opt_arg(args, "autoSwitchEnabled")?,
        opt_arg(args, "autoSwitchThreshold")?,
        opt_arg(args, "autoSwitchCreditsEnabled")?,
        opt_arg(args, "autoSwitchCreditsThreshold")?,
        opt_arg(args, "autoSwitchScopeMode")?,
        opt_arg(args, "autoSwitchSelectedGroupIds")?,
        opt_arg(args, "autoSwitchAccountScopeMode")?,
        opt_arg(args, "autoSwitchSelectedAccountIds")?,
        opt_arg(args, "codexAutoSwitchEnabled")?,
        opt_arg(args, "codexAutoSwitchPrimaryThreshold")?,
        opt_arg(args, "codexAutoSwitchSecondaryThreshold")?,
        opt_arg(args, "codexAutoSwitchAccountScopeMode")?,
        opt_arg(args, "codexAutoSwitchSelectedAccountIds")?,
        opt_arg(args, "quotaAlertEnabled")?,
        opt_arg(args, "quotaAlertThreshold")?,
        opt_arg(args, "codexQuotaAlertEnabled")?,
        opt_arg(args, "codexQuotaAlertThreshold")?,
        opt_arg(args, "zedQuotaAlertEnabled")?,
        opt_arg(args, "zedQuotaAlertThreshold")?,
        opt_arg(args, "codexQuotaAlertPrimaryThreshold")?,
        opt_arg(args, "codexQuotaAlertSecondaryThreshold")?,
        opt_arg(args, "ghcpQuotaAlertEnabled")?,
        opt_arg(args, "ghcpQuotaAlertThreshold")?,
        opt_arg(args, "windsurfQuotaAlertEnabled")?,
        opt_arg(args, "windsurfQuotaAlertThreshold")?,
        opt_arg(args, "kiroQuotaAlertEnabled")?,
        opt_arg(args, "kiroQuotaAlertThreshold")?,
        opt_arg(args, "cursorQuotaAlertEnabled")?,
        opt_arg(args, "cursorQuotaAlertThreshold")?,
        opt_arg(args, "geminiQuotaAlertEnabled")?,
        opt_arg(args, "geminiQuotaAlertThreshold")?,
        opt_arg(args, "codebuddyQuotaAlertEnabled")?,
        opt_arg(args, "codebuddyQuotaAlertThreshold")?,
        opt_arg(args, "codebuddyCnQuotaAlertEnabled")?,
        opt_arg(args, "codebuddyCnQuotaAlertThreshold")?,
        opt_arg(args, "qoderQuotaAlertEnabled")?,
        opt_arg(args, "qoderQuotaAlertThreshold")?,
        opt_arg(args, "traeQuotaAlertEnabled")?,
        opt_arg(args, "traeQuotaAlertThreshold")?,
        opt_arg(args, "workbuddyQuotaAlertEnabled")?,
        opt_arg(args, "workbuddyQuotaAlertThreshold")?,
    ))
}

fn app_handle() -> Result<tauri::AppHandle, String> {
    crate::get_app_handle()
        .cloned()
        .ok_or_else(|| "App runtime is not available".to_string())
}

fn arg<T: DeserializeOwned>(args: &Value, key: &str) -> Result<T, String> {
    let value = args
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing argument '{}'", key))?;
    serde_json::from_value(value).map_err(|err| format!("invalid argument '{}': {}", key, err))
}

fn arg_or<T: DeserializeOwned>(args: &Value, key: &str, default: T) -> Result<T, String> {
    match args.get(key) {
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|err| format!("invalid argument '{}': {}", key, err)),
        None => Ok(default),
    }
}

fn opt_arg<T: DeserializeOwned>(args: &Value, key: &str) -> Result<Option<T>, String> {
    match args.get(key) {
        Some(Value::Null) | None => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|err| format!("invalid argument '{}': {}", key, err)),
    }
}

async fn read_http_request(stream: &mut TcpStream) -> Result<Option<HttpRequest>, String> {
    let mut buffer = Vec::new();
    let mut temp = [0u8; 4096];
    let header_end = loop {
        let read = timeout(REQUEST_READ_TIMEOUT, stream.read(&mut temp))
            .await
            .map_err(|_| "request read timed out".to_string())?
            .map_err(|err| err.to_string())?;
        if read == 0 {
            if buffer.is_empty() {
                return Ok(None);
            }
            return Err("connection closed before headers completed".to_string());
        }
        buffer.extend_from_slice(&temp[..read]);
        if buffer.len() > MAX_HTTP_REQUEST_BYTES {
            return Err("request too large".to_string());
        }
        if let Some(pos) = find_header_end(&buffer) {
            break pos;
        }
    };

    let header_text =
        String::from_utf8(buffer[..header_end].to_vec()).map_err(|err| err.to_string())?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| "missing request line".to_string())?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap_or("").to_string();
    let raw_path = request_parts.next().unwrap_or("/");
    let path = normalize_request_path(raw_path)?;
    let mut content_length = 0usize;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            if name == "content-length" {
                content_length = value
                    .parse::<usize>()
                    .map_err(|_| "invalid content-length".to_string())?;
            }
        }
    }

    if content_length > MAX_HTTP_REQUEST_BYTES {
        return Err("request body too large".to_string());
    }

    let body_start = header_end + 4;
    let mut body = buffer.get(body_start..).unwrap_or_default().to_vec();
    while body.len() < content_length {
        let read = timeout(REQUEST_READ_TIMEOUT, stream.read(&mut temp))
            .await
            .map_err(|_| "request body read timed out".to_string())?
            .map_err(|err| err.to_string())?;
        if read == 0 {
            return Err("connection closed before body completed".to_string());
        }
        body.extend_from_slice(&temp[..read]);
        if body.len() > MAX_HTTP_REQUEST_BYTES {
            return Err("request body too large".to_string());
        }
    }
    body.truncate(content_length);

    Ok(Some(HttpRequest { method, path, body }))
}

fn normalize_request_path(raw_path: &str) -> Result<String, String> {
    let url = Url::parse(&format!("http://127.0.0.1{}", raw_path))
        .map_err(|err| format!("invalid request path: {}", err))?;
    Ok(url.path().to_string())
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

async fn write_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
) -> Result<(), String> {
    let headers = format!(
        "HTTP/1.1 {} {}\r\ncontent-type: {}\r\ncontent-length: {}\r\ncache-control: no-store\r\nx-content-type-options: nosniff\r\naccess-control-allow-origin: http://127.0.0.1:{}\r\naccess-control-allow-methods: GET,POST,OPTIONS\r\naccess-control-allow-headers: content-type\r\nconnection: close\r\n\r\n",
        status,
        reason,
        content_type,
        body.len(),
        get_actual_port().unwrap_or(DEFAULT_WEB_CONSOLE_PORT)
    );
    stream
        .write_all(headers.as_bytes())
        .await
        .map_err(|err| err.to_string())?;
    stream
        .write_all(body)
        .await
        .map_err(|err| err.to_string())?;
    stream.shutdown().await.map_err(|err| err.to_string())
}

fn find_frontend_dist() -> Option<PathBuf> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let candidates = [
        manifest_dir.join("../dist"),
        std::env::current_dir().ok()?.join("dist"),
        std::env::current_dir().ok()?.join("../dist"),
    ];

    candidates
        .into_iter()
        .map(|path| normalize_path(&path))
        .find(|path| path.join(INDEX_HTML).exists())
}

fn normalize_path(path: &Path) -> PathBuf {
    path.components().fold(PathBuf::new(), |mut acc, part| {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                acc.pop();
            }
            other => acc.push(other.as_os_str()),
        }
        acc
    })
}

fn resolve_static_path(root: &Path, request_path: &str) -> Result<PathBuf, String> {
    let path = if request_path == "/" || request_path.is_empty() {
        INDEX_HTML.to_string()
    } else {
        request_path.trim_start_matches('/').to_string()
    };
    let decoded =
        urlencoding::decode(&path).map_err(|err| format!("invalid URL encoding: {}", err))?;
    let mut result = PathBuf::from(root);
    for segment in decoded.split('/') {
        if segment.is_empty() {
            continue;
        }
        if segment == "." || segment == ".." || segment.contains('\\') {
            return Err("invalid static path".to_string());
        }
        result.push(segment);
    }
    Ok(result)
}

fn content_type_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}
