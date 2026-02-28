use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Utc};
use rcgen::generate_simple_self_signed;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::{Child, Command};
use tokio::sync::{oneshot, Mutex as TokioMutex, Notify};
use tokio::time::{timeout, Duration};
use tokio_rustls::TlsAcceptor;

const START_CASCADE_PATH: &str = "/exa.language_server_pb.LanguageServerService/StartCascade";
const SEND_USER_CASCADE_MESSAGE_PATH: &str =
    "/exa.language_server_pb.LanguageServerService/SendUserCascadeMessage";
const GET_CASCADE_TRAJECTORY_PATH: &str =
    "/exa.language_server_pb.LanguageServerService/GetCascadeTrajectory";
const DELETE_CASCADE_TRAJECTORY_PATH: &str =
    "/exa.language_server_pb.LanguageServerService/DeleteCascadeTrajectory";
pub const INTERNAL_PREPARE_START_CONTEXT_PATH: &str = "/__ag_internal__/wakeup/prepareStartContext";
pub const INTERNAL_HEALTH_CHECK_PATH: &str = "/__ag_internal__/wakeup/health";

const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HTTP_REQUEST_BYTES: usize = 512 * 1024;
const DEFAULT_THINKING_TEXT: &str = "Thinking";
const OFFICIAL_LS_START_TIMEOUT: Duration = Duration::from_secs(15);
const OFFICIAL_LS_POLL_TIMEOUT: Duration = Duration::from_secs(60);
const OFFICIAL_LS_POLL_INTERVAL: Duration = Duration::from_millis(250);
const OFFICIAL_LS_CLOUD_CODE_DAILY: &str = "https://daily-cloudcode-pa.googleapis.com";
const OFFICIAL_LS_CLOUD_CODE_PROD: &str = "https://cloudcode-pa.googleapis.com";
const OFFICIAL_LS_APP_DATA_DIR_PREFIX: &str = "antigravity-cockpit-tools-wakeup-ls";
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

static LOCAL_GATEWAY_BASE_URL: OnceLock<TokioMutex<Option<String>>> = OnceLock::new();

#[derive(Debug, Clone)]
struct PreparedStartContext {
    account_id: String,
    model: Option<String>,
    max_output_tokens: Option<u32>,
    prepared_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct ConversationState {
    trajectory_id: String,
    account_id: Option<String>,
    model: Option<String>,
    max_output_tokens: u32,
    prompt_text: Option<String>,
    cascade_status: String,
    updated_at: DateTime<Utc>,
    processing_started_at: Option<DateTime<Utc>>,
    processing_finished_at: Option<DateTime<Utc>>,
    planner_thinking: Option<String>,
    wakeup_result: Option<crate::modules::wakeup::WakeupResponse>,
    error_message: Option<String>,
}

impl ConversationState {
    fn new(_cascade_id: String, prepared: Option<PreparedStartContext>) -> Self {
        let now = Utc::now();
        let trajectory_id = format!("traj_{}", uuid::Uuid::new_v4().simple());
        let (account_id, model, max_output_tokens) = if let Some(ctx) = prepared {
            (
                Some(ctx.account_id),
                ctx.model,
                ctx.max_output_tokens.unwrap_or(0),
            )
        } else {
            (None, None, 0)
        };

        Self {
            trajectory_id,
            account_id,
            model,
            max_output_tokens,
            prompt_text: None,
            cascade_status: "IDLE".to_string(),
            updated_at: now,
            processing_started_at: None,
            processing_finished_at: None,
            planner_thinking: None,
            wakeup_result: None,
            error_message: None,
        }
    }

    fn mark_send_started(
        &mut self,
        prompt_text: String,
        model: Option<String>,
        max_output_tokens: Option<u32>,
    ) {
        if self.model.is_none() {
            self.model = model;
        }
        if self.max_output_tokens == 0 {
            self.max_output_tokens = max_output_tokens.unwrap_or(0);
        }
        self.prompt_text = Some(prompt_text);
        self.cascade_status = "RUNNING".to_string();
        self.processing_started_at = Some(Utc::now());
        self.processing_finished_at = None;
        self.planner_thinking = Some(DEFAULT_THINKING_TEXT.to_string());
        self.wakeup_result = None;
        self.error_message = None;
        self.updated_at = Utc::now();
    }

    fn mark_send_success(&mut self, resp: crate::modules::wakeup::WakeupResponse) {
        self.cascade_status = "IDLE".to_string();
        self.processing_finished_at = Some(Utc::now());
        self.wakeup_result = Some(resp);
        self.error_message = None;
        self.updated_at = Utc::now();
    }

    fn mark_send_failure(&mut self, err: String) {
        self.cascade_status = "IDLE".to_string();
        self.processing_finished_at = Some(Utc::now());
        self.wakeup_result = None;
        self.error_message = Some(err);
        self.updated_at = Utc::now();
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrepareStartContextRequest {
    account_id: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SendMessageItem {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    item: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendUserCascadeMessageRequest {
    cascade_id: String,
    #[serde(default)]
    items: Vec<SendMessageItem>,
    #[serde(default)]
    cascade_config: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CascadeIdRequest {
    cascade_id: String,
}

fn conversations() -> &'static Mutex<HashMap<String, ConversationState>> {
    static CONVERSATIONS: OnceLock<Mutex<HashMap<String, ConversationState>>> = OnceLock::new();
    CONVERSATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn pending_start_contexts() -> &'static Mutex<VecDeque<PreparedStartContext>> {
    static PENDING: OnceLock<Mutex<VecDeque<PreparedStartContext>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn pop_prepared_start_context() -> Result<Option<PreparedStartContext>, String> {
    let mut guard = pending_start_contexts()
        .lock()
        .map_err(|_| "准备上下文锁失败".to_string())?;
    while let Some(front) = guard.front() {
        if Utc::now()
            .signed_duration_since(front.prepared_at)
            .num_seconds()
            > 60
        {
            let _ = guard.pop_front();
            continue;
        }
        break;
    }
    Ok(guard.pop_front())
}

fn parse_json_object_body(body: &[u8], name: &str) -> Result<Value, (u16, String)> {
    if body.is_empty() {
        return Ok(json!({}));
    }

    let payload: Value =
        serde_json::from_slice(body).map_err(|e| (400, format!("{} 请求体无效: {}", name, e)))?;
    if !payload.is_object() {
        return Err((400, format!("{} 请求体必须为 JSON object", name)));
    }
    Ok(payload)
}

fn extract_required_cascade_id(payload: &Value, name: &str) -> Result<String, (u16, String)> {
    let cascade_id = payload
        .get("cascadeId")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| (400, format!("{} 缺少 cascadeId", name)))?;
    Ok(cascade_id.to_string())
}

fn get_official_ls_session(cascade_id: &str) -> Result<Arc<OfficialLsCascadeSession>, String> {
    let guard = official_ls_sessions()
        .lock()
        .map_err(|_| "官方 LS 会话映射锁失败".to_string())?;
    guard
        .get(cascade_id)
        .cloned()
        .ok_or_else(|| format!("会话不存在: {}", cascade_id))
}

fn remove_official_ls_session(
    cascade_id: &str,
) -> Result<Option<Arc<OfficialLsCascadeSession>>, String> {
    let mut guard = official_ls_sessions()
        .lock()
        .map_err(|_| "官方 LS 会话映射锁失败".to_string())?;
    Ok(guard.remove(cascade_id))
}

async fn shutdown_official_ls_session(session: &Arc<OfficialLsCascadeSession>) {
    let mut process_guard = session.process.lock().await;
    if let Some(mut process) = process_guard.take() {
        process.shutdown().await;
    }
}

async fn start_official_ls_cascade_session(
    prepared: PreparedStartContext,
    start_body: &Value,
) -> Result<(Value, Arc<OfficialLsCascadeSession>), String> {
    let (_account, token) = ensure_wakeup_account_token(&prepared.account_id).await?;
    let mut ls = start_official_ls_process(&prepared.account_id, &token).await?;
    let client = build_official_ls_local_client(30)?;
    let base_url = format!("https://127.0.0.1:{}", ls.started.https_port);
    let start_resp = match post_json_to_official_ls(
        &client,
        &base_url,
        &ls.ls_csrf_token,
        START_CASCADE_PATH,
        start_body,
    )
    .await
    {
        Ok(v) => v,
        Err(err) => {
            ls.shutdown().await;
            return Err(err);
        }
    };

    start_resp
        .get("cascadeId")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "官方 LS StartCascade 未返回 cascadeId".to_string())?;

    let session = Arc::new(OfficialLsCascadeSession {
        account_id: prepared.account_id,
        client,
        base_url,
        ls_csrf_token: ls.ls_csrf_token.clone(),
        process: TokioMutex::new(Some(ls)),
    });

    Ok((start_resp, session))
}

async fn proxy_official_ls_session_json_request(
    session: &OfficialLsCascadeSession,
    path: &str,
    body: &Value,
) -> Result<Value, String> {
    post_json_to_official_ls(
        &session.client,
        &session.base_url,
        &session.ls_csrf_token,
        path,
        body,
    )
    .await
}

fn trim_non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn json_response(status_code: u16, status_text: &str, body: &Value) -> Vec<u8> {
    let body_bytes = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\n\r\n",
        status_code,
        status_text,
        body_bytes.len()
    );
    let mut resp = headers.into_bytes();
    resp.extend_from_slice(&body_bytes);
    resp
}

fn text_response(status_code: u16, status_text: &str, body: &str, content_type: &str) -> Vec<u8> {
    let body_bytes = body.as_bytes();
    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\n\r\n",
        status_code, status_text, content_type, body_bytes.len()
    );
    let mut resp = headers.into_bytes();
    resp.extend_from_slice(body_bytes);
    resp
}

fn options_response() -> Vec<u8> {
    text_response(200, "OK", "", "text/plain; charset=utf-8")
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|idx| idx + 4)
}

fn parse_content_length(header_bytes: &[u8]) -> Result<usize, String> {
    let header_text = String::from_utf8_lossy(header_bytes);
    for line in header_text.lines() {
        let mut parts = line.splitn(2, ':');
        let Some(name) = parts.next() else { continue };
        let Some(value) = parts.next() else { continue };
        if name.trim().eq_ignore_ascii_case("content-length") {
            return value
                .trim()
                .parse::<usize>()
                .map_err(|e| format!("非法 Content-Length: {}", e));
        }
    }
    Ok(0)
}

async fn read_http_request<R>(stream: &mut R) -> Result<Vec<u8>, String>
where
    R: AsyncRead + Unpin,
{
    let mut buffer = Vec::with_capacity(4096);
    let mut chunk = [0u8; 2048];
    let mut header_end: Option<usize> = None;
    let mut content_length: usize = 0;

    loop {
        let bytes_read = timeout(REQUEST_READ_TIMEOUT, stream.read(&mut chunk))
            .await
            .map_err(|_| "读取网关请求超时".to_string())?
            .map_err(|e| format!("读取网关请求失败: {}", e))?;

        if bytes_read == 0 {
            break;
        }

        buffer.extend_from_slice(&chunk[..bytes_read]);
        if buffer.len() > MAX_HTTP_REQUEST_BYTES {
            return Err("请求体过大".to_string());
        }

        if header_end.is_none() {
            if let Some(end) = find_header_end(&buffer) {
                content_length = parse_content_length(&buffer[..end])?;
                header_end = Some(end);
            }
        }

        if let Some(end) = header_end {
            if buffer.len() >= end.saturating_add(content_length) {
                return Ok(buffer[..(end + content_length)].to_vec());
            }
        }
    }

    Err("请求不完整".to_string())
}

#[derive(Debug)]
struct ParsedRequest {
    method: String,
    target: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn parse_http_request(raw: &[u8]) -> Result<ParsedRequest, String> {
    let Some(header_end) = find_header_end(raw) else {
        return Err("缺少 HTTP 头结束标记".to_string());
    };
    let header_text = String::from_utf8_lossy(&raw[..header_end]);
    let mut lines = header_text.lines();
    let request_line = lines.next().ok_or_else(|| "请求行为空".to_string())?.trim();

    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| "请求行缺少 method".to_string())?
        .to_string();
    let target = parts
        .next()
        .ok_or_else(|| "请求行缺少 target".to_string())?
        .to_string();

    let mut headers = HashMap::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, ':');
        let Some(name) = parts.next() else { continue };
        let Some(value) = parts.next() else { continue };
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }

    Ok(ParsedRequest {
        method,
        target,
        headers,
        body: raw[header_end..].to_vec(),
    })
}

fn normalize_path(target: &str) -> String {
    if target.starts_with("http://") || target.starts_with("https://") {
        if let Ok(url) = url::Url::parse(target) {
            return url.path().to_string();
        }
    }
    if let Ok(url) = url::Url::parse(&format!("http://localhost{}", target)) {
        return url.path().to_string();
    }
    target.to_string()
}

fn rpc_method_name_from_path(path: &str) -> &str {
    let last = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path);
    last.split(':').next().unwrap_or(last)
}

fn path_matches_rpc_method(path: &str, method_name: &str) -> bool {
    rpc_method_name_from_path(path) == method_name
}

#[derive(Debug, Clone, Copy)]
struct OfficialLsStartedInfo {
    https_port: u16,
    http_port: u16,
    lsp_port: u16,
}

struct OfficialLsExtensionServerState {
    csrf_token: String,
    uss_oauth_topic_bytes: Vec<u8>,
    empty_topic_bytes: Vec<u8>,
    started_sender: Mutex<Option<oneshot::Sender<OfficialLsStartedInfo>>>,
    shutdown_notify: Arc<Notify>,
}

struct OfficialLsExtensionServerHandle {
    port: u16,
    csrf_token: String,
    started_receiver: oneshot::Receiver<OfficialLsStartedInfo>,
    shutdown_notify: Arc<Notify>,
    task: tokio::task::JoinHandle<()>,
}

struct OfficialLsProcessHandle {
    child: Child,
    stdout_task: Option<tokio::task::JoinHandle<()>>,
    stderr_task: Option<tokio::task::JoinHandle<()>>,
    extension_server: OfficialLsExtensionServerHandle,
    started: OfficialLsStartedInfo,
    ls_csrf_token: String,
}

struct OfficialLsCascadeSession {
    account_id: String,
    client: reqwest::Client,
    base_url: String,
    ls_csrf_token: String,
    process: TokioMutex<Option<OfficialLsProcessHandle>>,
}

fn official_ls_sessions() -> &'static Mutex<HashMap<String, Arc<OfficialLsCascadeSession>>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, Arc<OfficialLsCascadeSession>>>> =
        OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

const APP_PATH_NOT_FOUND_PREFIX: &str = "APP_PATH_NOT_FOUND:";

fn app_path_missing_error(app: &str) -> String {
    format!("{}{}", APP_PATH_NOT_FOUND_PREFIX, app)
}

#[cfg(target_os = "macos")]
fn normalize_macos_app_root(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let path_str = path.to_string_lossy();
    let app_idx = path_str.find(".app")?;
    let root = &path_str[..app_idx + 4];
    let root_path = std::path::PathBuf::from(root);
    if root_path.exists() {
        Some(root_path)
    } else {
        None
    }
}

fn resolve_configured_antigravity_root(path_str: &str) -> Option<std::path::PathBuf> {
    let raw = path_str.trim();
    if raw.is_empty() {
        return None;
    }

    let original_path = std::path::PathBuf::from(raw);
    let resolved_path =
        std::fs::canonicalize(&original_path).unwrap_or_else(|_| original_path.clone());

    #[cfg(target_os = "macos")]
    {
        if let Some(root) = normalize_macos_app_root(&original_path)
            .or_else(|| normalize_macos_app_root(&resolved_path))
        {
            return Some(root);
        }
    }

    if !resolved_path.exists() {
        return None;
    }
    if resolved_path.is_file() {
        return resolved_path.parent().map(std::path::Path::to_path_buf);
    }
    if resolved_path.is_dir() {
        return Some(resolved_path);
    }
    None
}

fn antigravity_extension_dir(root: &std::path::Path) -> std::path::PathBuf {
    #[cfg(target_os = "macos")]
    {
        return root
            .join("Contents")
            .join("Resources")
            .join("app")
            .join("extensions")
            .join("antigravity");
    }
    #[cfg(not(target_os = "macos"))]
    {
        return root
            .join("resources")
            .join("app")
            .join("extensions")
            .join("antigravity");
    }
}

fn antigravity_extension_bin_dir(root: &std::path::Path) -> std::path::PathBuf {
    antigravity_extension_dir(root).join("bin")
}

fn find_official_ls_binary_under(root: &std::path::Path) -> Option<String> {
    let bin_dir = antigravity_extension_bin_dir(root);

    #[cfg(target_os = "windows")]
    let preferred = [
        "language_server_windows_x64.exe",
        "language_server_windows_arm64.exe",
        "language_server_windows.exe",
    ];
    #[cfg(target_os = "macos")]
    let preferred = [
        "language_server_macos_arm",
        "language_server_macos_x64",
        "language_server_macos",
        "language_server_darwin_arm64",
        "language_server_darwin_x64",
        "language_server_darwin",
        "language_server",
    ];
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let preferred = [
        "language_server_linux_x64",
        "language_server_linux_arm64",
        "language_server_linux",
        "language_server",
    ];

    for name in preferred {
        let candidate = bin_dir.join(name);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }

    let mut dynamic_candidates: Vec<std::path::PathBuf> = Vec::new();
    let bin_dir = antigravity_extension_bin_dir(root);
    let entries = std::fs::read_dir(bin_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        if !lower.starts_with("language_server") {
            continue;
        }
        #[cfg(target_os = "windows")]
        if !lower.ends_with(".exe") {
            continue;
        }
        dynamic_candidates.push(path);
    }

    dynamic_candidates.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    dynamic_candidates
        .into_iter()
        .next()
        .map(|path| path.to_string_lossy().to_string())
}

fn resolve_official_ls_binary_from_config() -> Result<String, String> {
    let user_config = crate::modules::config::get_user_config();
    let antigravity_path = user_config.antigravity_app_path.trim();
    let root = resolve_configured_antigravity_root(antigravity_path)
        .ok_or_else(|| app_path_missing_error("antigravity"))?;
    find_official_ls_binary_under(&root).ok_or_else(|| app_path_missing_error("antigravity"))
}

fn official_ls_binary_path() -> Result<String, String> {
    if let Ok(v) = std::env::var("AG_WAKEUP_OFFICIAL_LS_BINARY_PATH") {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    resolve_official_ls_binary_from_config()
}

pub fn ensure_official_ls_binary_ready() -> Result<String, String> {
    official_ls_binary_path()
}

fn official_ls_cloud_code_endpoint(token: &crate::models::token::TokenData) -> &'static str {
    if token.is_gcp_tos == Some(true) {
        OFFICIAL_LS_CLOUD_CODE_PROD
    } else {
        OFFICIAL_LS_CLOUD_CODE_DAILY
    }
}

fn official_antigravity_info_plist_path() -> &'static str {
    "/Applications/Antigravity.app/Contents/Info.plist"
}

fn official_antigravity_extension_path() -> String {
    let user_config = crate::modules::config::get_user_config();
    if let Some(root) = resolve_configured_antigravity_root(user_config.antigravity_app_path.trim())
    {
        let ext_path = antigravity_extension_dir(&root);
        if ext_path.exists() {
            return ext_path.to_string_lossy().to_string();
        }
        return root.to_string_lossy().to_string();
    }

    let default_path =
        "/Applications/Antigravity.app/Contents/Resources/app/extensions/antigravity";
    if std::path::Path::new(default_path).exists() {
        return default_path.to_string();
    }
    "/Applications/Antigravity.app".to_string()
}

fn official_antigravity_app_version() -> String {
    if let Ok(v) = std::env::var("AG_WAKEUP_OFFICIAL_APP_VERSION") {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    static CACHE: OnceLock<String> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let output = std::process::Command::new("plutil")
                .arg("-p")
                .arg(official_antigravity_info_plist_path())
                .output();
            match output {
                Ok(out) if out.status.success() => {
                    let text = String::from_utf8_lossy(&out.stdout);
                    for line in text.lines() {
                        let line = line.trim();
                        if !line.starts_with("\"CFBundleShortVersionString\"") {
                            continue;
                        }
                        if let Some(version) = line.split("=>").nth(1) {
                            let version = version.trim().trim_matches('"');
                            if !version.is_empty() {
                                return version.to_string();
                            }
                        }
                    }
                    "1.19.5".to_string()
                }
                _ => "1.19.5".to_string(),
            }
        })
        .clone()
}

fn build_official_ls_metadata_bytes() -> Vec<u8> {
    use crate::utils::protobuf::{encode_len_delim_field, encode_varint};

    let mut out = Vec::new();
    let push_str = |buf: &mut Vec<u8>, field_num: u32, value: &str| {
        if value.is_empty() {
            return;
        }
        buf.extend(encode_len_delim_field(field_num, value.as_bytes()));
    };

    // exa.codeium_common_pb.Metadata
    // 1=ide_name, 7=ide_version, 12=extension_name, 17=extension_path, 4=locale, 24=device_fingerprint
    push_str(&mut out, 1, "Antigravity");
    push_str(&mut out, 7, &official_antigravity_app_version());
    push_str(&mut out, 12, "antigravity");
    push_str(&mut out, 17, &official_antigravity_extension_path());
    push_str(
        &mut out,
        4,
        &std::env::var("LANG")
            .ok()
            .and_then(|v| v.split('.').next().map(|s| s.replace('_', "-")))
            .unwrap_or_else(|| "zh-CN".to_string()),
    );
    push_str(&mut out, 24, &uuid::Uuid::new_v4().to_string());

    // ensure the message is not empty to avoid LS startup error
    if out.is_empty() {
        out.extend(encode_varint(0));
    }

    out
}

fn build_uss_oauth_topic_bytes(token: &crate::models::token::TokenData) -> Vec<u8> {
    let expiry = token.expiry_timestamp.max(0);
    let oauth_info = crate::utils::protobuf::create_oauth_info(
        &token.access_token,
        &token.refresh_token,
        expiry,
    );
    let oauth_info_b64 = general_purpose::STANDARD.encode(oauth_info);

    // exa.unified_state_sync_pb.Topic
    // Topic.data -> map<string, Row>
    // Row.value stores base64(oauth_info)
    let row = crate::utils::protobuf::encode_string_field(1, &oauth_info_b64);
    let entry = [
        crate::utils::protobuf::encode_string_field(1, "oauthTokenInfoSentinelKey"),
        crate::utils::protobuf::encode_len_delim_field(2, &row),
    ]
    .concat();
    crate::utils::protobuf::encode_len_delim_field(1, &entry)
}

fn build_unified_state_sync_update_initial_state(topic_bytes: &[u8]) -> Vec<u8> {
    // exa.extension_server_pb.UnifiedStateSyncUpdate: field 1 = initial_state (Topic)
    crate::utils::protobuf::encode_len_delim_field(1, topic_bytes)
}

fn parse_official_ls_started_request(body: &[u8]) -> Result<OfficialLsStartedInfo, String> {
    let mut offset = 0usize;
    let mut https_port: Option<u16> = None;
    let mut http_port: Option<u16> = None;
    let mut lsp_port: Option<u16> = None;

    while offset < body.len() {
        let (tag, new_offset) = crate::utils::protobuf::read_varint(body, offset)?;
        let wire_type = (tag & 7) as u8;
        let field_num = (tag >> 3) as u32;

        match (field_num, wire_type) {
            (1, 0) => {
                let (v, end) = crate::utils::protobuf::read_varint(body, new_offset)?;
                https_port = u16::try_from(v).ok();
                offset = end;
                continue;
            }
            (2, 0) => {
                let (v, end) = crate::utils::protobuf::read_varint(body, new_offset)?;
                lsp_port = u16::try_from(v).ok();
                offset = end;
                continue;
            }
            (5, 0) => {
                let (v, end) = crate::utils::protobuf::read_varint(body, new_offset)?;
                http_port = u16::try_from(v).ok();
                offset = end;
                continue;
            }
            _ => {}
        }

        offset = crate::utils::protobuf::skip_field(body, new_offset, wire_type)?;
    }

    Ok(OfficialLsStartedInfo {
        https_port: https_port
            .ok_or_else(|| "LanguageServerStarted 缺少 https_port".to_string())?,
        http_port: http_port.unwrap_or(0),
        lsp_port: lsp_port.unwrap_or(0),
    })
}

fn parse_subscribe_topic_from_connect_body(body: &[u8]) -> Result<String, String> {
    let payload = decode_connect_request_first_message(body)?;
    let mut offset = 0usize;
    while offset < payload.len() {
        let (tag, new_offset) = crate::utils::protobuf::read_varint(payload, offset)?;
        let wire_type = (tag & 7) as u8;
        let field_num = (tag >> 3) as u32;
        if field_num == 1 && wire_type == 2 {
            let (len, content_offset) = crate::utils::protobuf::read_varint(payload, new_offset)?;
            let len = len as usize;
            let end = content_offset + len;
            if end > payload.len() {
                return Err("SubscribeToUnifiedStateSyncTopic 请求体长度非法".to_string());
            }
            let topic = std::str::from_utf8(&payload[content_offset..end])
                .map_err(|e| format!("topic UTF-8 解码失败: {}", e))?;
            return Ok(topic.to_string());
        }
        offset = crate::utils::protobuf::skip_field(payload, new_offset, wire_type)?;
    }
    Err("SubscribeToUnifiedStateSyncTopic 缺少 topic".to_string())
}

fn decode_connect_request_first_message(body: &[u8]) -> Result<&[u8], String> {
    if body.len() < 5 {
        return Err("Connect 请求体过短".to_string());
    }
    let flags = body[0];
    if flags & 0x01 != 0 {
        return Err("暂不支持压缩的 Connect 请求".to_string());
    }
    let len = u32::from_be_bytes([body[1], body[2], body[3], body[4]]) as usize;
    let start = 5usize;
    let end = start + len;
    if end > body.len() {
        return Err("Connect 请求帧长度非法".to_string());
    }
    Ok(&body[start..end])
}

fn encode_connect_envelope(flags: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + payload.len());
    out.push(flags);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

fn encode_connect_message_envelope(payload: &[u8]) -> Vec<u8> {
    encode_connect_envelope(0, payload)
}

fn encode_connect_end_ok_envelope() -> Vec<u8> {
    encode_connect_envelope(0x02, br#"{}"#)
}

fn binary_http_response(
    status_code: u16,
    status_text: &str,
    content_type: &str,
    body: &[u8],
) -> Vec<u8> {
    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status_code,
        status_text,
        content_type,
        body.len()
    );
    let mut resp = headers.into_bytes();
    resp.extend_from_slice(body);
    resp
}

fn extension_unary_response(request_content_type: &str, proto_body: &[u8]) -> Vec<u8> {
    let content_type_lc = request_content_type.to_ascii_lowercase();
    if content_type_lc.starts_with("application/connect+proto") {
        let body = encode_connect_message_envelope(proto_body);
        return binary_http_response(200, "OK", "application/connect+proto", &body);
    }

    binary_http_response(
        200,
        "OK",
        if request_content_type.is_empty() {
            "application/proto"
        } else {
            request_content_type
        },
        proto_body,
    )
}

fn chunked_http_stream_headers(status_code: u16, status_text: &str, content_type: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n",
        status_code, status_text, content_type
    )
    .into_bytes()
}

fn encode_chunked_bytes(payload: &[u8]) -> Vec<u8> {
    let mut out = format!("{:X}\r\n", payload.len()).into_bytes();
    out.extend_from_slice(payload);
    out.extend_from_slice(b"\r\n");
    out
}

fn encode_chunked_final() -> Vec<u8> {
    b"0\r\n\r\n".to_vec()
}

fn build_official_ls_local_client(timeout_secs: u64) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .no_proxy()
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| format!("创建官方 LS 本地客户端失败: {}", e))
}

async fn post_json_to_official_ls(
    client: &reqwest::Client,
    base_url: &str,
    csrf_token: &str,
    path: &str,
    body: &Value,
) -> Result<Value, String> {
    let url = format!("{}{}", base_url.trim_end_matches('/'), path);
    let resp = client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header("x-codeium-csrf-token", csrf_token)
        .json(body)
        .send()
        .await
        .map_err(|e| format!("官方 LS 请求失败: {} ({})", e, path))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!(
            "官方 LS 返回错误: {} - {} ({})",
            status, text, path
        ));
    }

    resp.json::<Value>()
        .await
        .map_err(|e| format!("官方 LS 响应解析失败: {} ({})", e, path))
}

fn extract_wakeup_response_from_official_ls_trajectory(
    get_resp: &Value,
    duration_ms: u64,
) -> Option<crate::modules::wakeup::WakeupResponse> {
    let steps = get_resp
        .get("trajectory")
        .and_then(|v| v.get("steps"))
        .and_then(|v| v.as_array())?;

    for step in steps.iter().rev() {
        let step_case = step
            .get("step")
            .and_then(|v| v.get("case"))
            .and_then(|v| v.as_str());
        if step_case != Some("plannerResponse") {
            continue;
        }

        let value = step.get("step").and_then(|v| v.get("value"))?;
        let reply = value
            .get("modifiedResponse")
            .and_then(|v| v.as_str())
            .or_else(|| value.get("response").and_then(|v| v.as_str()))?
            .trim()
            .to_string();
        if reply.is_empty() {
            continue;
        }

        return Some(crate::modules::wakeup::WakeupResponse {
            reply,
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
            trace_id: None,
            response_id: None,
            duration_ms,
        });
    }

    None
}

fn extract_official_ls_error_from_trajectory(get_resp: &Value) -> Option<String> {
    let steps = get_resp
        .get("trajectory")
        .and_then(|v| v.get("steps"))
        .and_then(|v| v.as_array())?;

    for step in steps.iter().rev() {
        let step_case = step
            .get("step")
            .and_then(|v| v.get("case"))
            .and_then(|v| v.as_str());
        if step_case != Some("errorMessage") {
            continue;
        }
        let msg = step
            .get("step")
            .and_then(|v| v.get("value"))
            .and_then(|v| v.get("userErrorMessage"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())?;
        return Some(msg.to_string());
    }
    None
}

fn json_value_to_non_empty_string(value: &Value) -> Option<String> {
    match value {
        Value::String(v) => {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Value::Number(v) => Some(v.to_string()),
        _ => None,
    }
}

fn parse_prompt_from_items(items: &[SendMessageItem]) -> String {
    let mut parts: Vec<String> = Vec::new();

    for item in items {
        if let Some(text) = item.text.as_deref() {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                parts.push(trimmed.to_string());
                continue;
            }
        }

        if let Some(item_json) = item.item.as_ref() {
            if let Some(text) = item_json
                .get("chunk")
                .and_then(|v| v.get("case"))
                .and_then(|v| v.as_str())
                .filter(|v| *v == "text")
                .and_then(|_| item_json.get("chunk"))
                .and_then(|v| v.get("value"))
                .and_then(|v| v.as_str())
            {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                    continue;
                }
            }

            if let Some(text) = item_json.get("text").and_then(|v| v.as_str()) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
            }
        }
    }

    parts.join("\n")
}

fn json_value_to_u32(value: &Value) -> Option<u32> {
    match value {
        Value::Number(n) => n.as_u64().and_then(|v| u32::try_from(v).ok()),
        Value::String(s) => s.trim().parse::<u32>().ok(),
        _ => None,
    }
}

fn parse_model_and_max_tokens_from_cascade_config(
    cascade_config: &Option<Value>,
) -> (Option<String>, Option<u32>) {
    let Some(cfg) = cascade_config.as_ref() else {
        return (None, None);
    };

    let planner = cfg.get("plannerConfig");
    let requested = planner.and_then(|v| v.get("requestedModel"));
    let model = requested.and_then(|requested| {
        requested
            .get("alias")
            .and_then(json_value_to_non_empty_string)
            .or_else(|| {
                requested
                    .get("model")
                    .and_then(json_value_to_non_empty_string)
            })
    });

    let max_output_tokens = planner
        .and_then(|planner| planner.get("maxOutputTokens"))
        .and_then(json_value_to_u32)
        .or_else(|| {
            cfg.get("checkpointConfig")
                .and_then(|checkpoint| checkpoint.get("maxOutputTokens"))
                .and_then(json_value_to_u32)
        });

    (model, max_output_tokens)
}

fn proto_timestamp(ts: DateTime<Utc>) -> Value {
    json!({
        "seconds": ts.timestamp().to_string(),
        "nanos": ts.timestamp_subsec_nanos(),
    })
}

fn step_metadata(convo: &ConversationState, step_index: usize, at: DateTime<Utc>) -> Value {
    json!({
        "createdAt": proto_timestamp(at),
        "viewableAt": proto_timestamp(at),
        "sourceTrajectoryStepInfo": {
            "trajectoryId": convo.trajectory_id,
            "stepIndex": step_index,
        }
    })
}

fn build_user_input_step(convo: &ConversationState, prompt_text: &str) -> Value {
    let at = convo.processing_started_at.unwrap_or(convo.updated_at);
    json!({
        "status": "DONE",
        "step": {
            "case": "userInput",
            "value": {
                "isQueuedMessage": false,
                "items": [
                    {
                        "chunk": {
                            "case": "text",
                            "value": prompt_text
                        }
                    }
                ],
                "media": [],
                "artifactComments": [],
                "fileDiffComments": [],
                "fileComments": []
            }
        },
        "metadata": step_metadata(convo, 0, at),
    })
}

fn planner_thinking_duration_value(convo: &ConversationState) -> Option<Value> {
    let start = convo.processing_started_at?;
    let end = convo.processing_finished_at.unwrap_or(convo.updated_at);
    let duration = end.signed_duration_since(start);
    let seconds = duration.num_seconds().max(0);
    Some(json!({
        "seconds": seconds.to_string(),
        "nanos": 0,
    }))
}

fn build_planner_response_running_step(convo: &ConversationState) -> Value {
    let at = convo.updated_at;
    let thinking = convo
        .planner_thinking
        .clone()
        .unwrap_or_else(|| DEFAULT_THINKING_TEXT.to_string());
    json!({
        "status": "RUNNING",
        "step": {
            "case": "plannerResponse",
            "value": {
                "thinking": thinking
            }
        },
        "metadata": step_metadata(convo, 1, at),
    })
}

fn build_planner_response_done_step(
    convo: &ConversationState,
    resp: &crate::modules::wakeup::WakeupResponse,
) -> Value {
    let at = convo.processing_finished_at.unwrap_or(convo.updated_at);
    let mut value = json!({
        "modifiedResponse": resp.reply,
        "recitationMetadata": { "recitations": [] }
    });

    if let Some(thinking) = convo.planner_thinking.as_ref() {
        if let Some(obj) = value.as_object_mut() {
            obj.insert("thinking".to_string(), json!(thinking));
        }
    }
    if let Some(duration) = planner_thinking_duration_value(convo) {
        if let Some(obj) = value.as_object_mut() {
            obj.insert("thinkingDuration".to_string(), duration);
        }
    }

    json!({
        "status": "DONE",
        "step": {
            "case": "plannerResponse",
            "value": value
        },
        "metadata": step_metadata(convo, 1, at),
    })
}

fn build_error_message_step(convo: &ConversationState, message: &str) -> Value {
    let at = convo.processing_finished_at.unwrap_or(convo.updated_at);
    json!({
        "status": "ERROR",
        "step": {
            "case": "errorMessage",
            "value": {
                "shouldShowUser": true,
                "userErrorMessage": message
            }
        },
        "metadata": step_metadata(convo, 1, at),
    })
}

fn build_trajectory_steps(convo: &ConversationState) -> Vec<Value> {
    let mut steps = Vec::new();

    if let Some(prompt_text) = convo.prompt_text.as_deref() {
        steps.push(build_user_input_step(convo, prompt_text));

        if convo.cascade_status == "RUNNING" {
            steps.push(build_planner_response_running_step(convo));
        } else if let Some(resp) = convo.wakeup_result.as_ref() {
            steps.push(build_planner_response_done_step(convo, resp));
        } else if let Some(err) = convo.error_message.as_deref() {
            steps.push(build_error_message_step(convo, err));
        }
    }

    steps
}

enum OfficialLsExtensionAction {
    Close(Vec<u8>),
    HoldStream {
        content_type: String,
        first_message: Vec<u8>,
        shutdown_notify: Arc<Notify>,
    },
}

async fn handle_official_ls_extension_connection<S>(
    mut stream: S,
    state: Arc<OfficialLsExtensionServerState>,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let action = match read_http_request(&mut stream).await {
        Ok(raw) => match parse_http_request(&raw) {
            Ok(parsed) => route_official_ls_extension_request(parsed, state.clone()).await,
            Err(err) => OfficialLsExtensionAction::Close(text_response(
                400,
                "Bad Request",
                &err,
                "text/plain; charset=utf-8",
            )),
        },
        Err(err) => OfficialLsExtensionAction::Close(text_response(
            400,
            "Bad Request",
            &err,
            "text/plain; charset=utf-8",
        )),
    };

    match action {
        OfficialLsExtensionAction::Close(resp) => {
            let _ = stream.write_all(&resp).await;
            let _ = stream.flush().await;
            let _ = stream.shutdown().await;
        }
        OfficialLsExtensionAction::HoldStream {
            content_type,
            first_message,
            shutdown_notify,
        } => {
            let headers = chunked_http_stream_headers(200, "OK", &content_type);
            let _ = stream.write_all(&headers).await;
            let _ = stream
                .write_all(&encode_chunked_bytes(&encode_connect_message_envelope(
                    &first_message,
                )))
                .await;
            let _ = stream.flush().await;

            shutdown_notify.notified().await;

            let _ = stream
                .write_all(&encode_chunked_bytes(&encode_connect_end_ok_envelope()))
                .await;
            let _ = stream.write_all(&encode_chunked_final()).await;
            let _ = stream.flush().await;
            let _ = stream.shutdown().await;
        }
    }
}

async fn route_official_ls_extension_request(
    parsed: ParsedRequest,
    state: Arc<OfficialLsExtensionServerState>,
) -> OfficialLsExtensionAction {
    let path = normalize_path(&parsed.target);
    let method = parsed.method.to_ascii_uppercase();
    let content_type = parsed
        .headers
        .get("content-type")
        .cloned()
        .unwrap_or_else(|| "application/proto".to_string());
    let request_csrf = parsed
        .headers
        .get("x-codeium-csrf-token")
        .cloned()
        .unwrap_or_default();

    if method == "OPTIONS" {
        return OfficialLsExtensionAction::Close(text_response(
            200,
            "OK",
            "",
            "text/plain; charset=utf-8",
        ));
    }
    if method != "POST" {
        return OfficialLsExtensionAction::Close(text_response(
            405,
            "Method Not Allowed",
            "Only POST is supported",
            "text/plain; charset=utf-8",
        ));
    }
    if request_csrf != state.csrf_token {
        return OfficialLsExtensionAction::Close(text_response(
            403,
            "Forbidden",
            "Invalid CSRF token",
            "text/plain; charset=utf-8",
        ));
    }

    if path_matches_rpc_method(&path, "LanguageServerStarted") {
        match parse_official_ls_started_request(&parsed.body) {
            Ok(started) => {
                if let Ok(mut guard) = state.started_sender.lock() {
                    if let Some(tx) = guard.take() {
                        let _ = tx.send(started);
                    }
                }
                return OfficialLsExtensionAction::Close(extension_unary_response(
                    &content_type,
                    &[],
                ));
            }
            Err(err) => {
                crate::modules::logger::log_error(&format!(
                    "[WakeupGateway] 官方 LS LanguageServerStarted 解析失败: {}",
                    err
                ));
                return OfficialLsExtensionAction::Close(text_response(
                    400,
                    "Bad Request",
                    &err,
                    "text/plain; charset=utf-8",
                ));
            }
        }
    }

    if path_matches_rpc_method(&path, "SubscribeToUnifiedStateSyncTopic") {
        let topic = match parse_subscribe_topic_from_connect_body(&parsed.body) {
            Ok(v) => v,
            Err(err) => {
                crate::modules::logger::log_error(&format!(
                    "[WakeupGateway] 官方 LS SubscribeToUnifiedStateSyncTopic 解析失败: {}",
                    err
                ));
                return OfficialLsExtensionAction::Close(text_response(
                    400,
                    "Bad Request",
                    &err,
                    "text/plain; charset=utf-8",
                ));
            }
        };

        let topic_bytes = match topic.as_str() {
            "uss-oauth" => &state.uss_oauth_topic_bytes,
            "uss-enterprisePreferences" | "uss-agentPreferences" => &state.empty_topic_bytes,
            _ => &state.empty_topic_bytes,
        };
        let update = build_unified_state_sync_update_initial_state(topic_bytes);

        return OfficialLsExtensionAction::HoldStream {
            content_type: "application/connect+proto".to_string(),
            first_message: update,
            shutdown_notify: state.shutdown_notify.clone(),
        };
    }

    // Minimal unary implementations to keep the official LS alive for wakeup usage.
    if path_matches_rpc_method(&path, "IsAgentManagerEnabled") {
        let body = [
            crate::utils::protobuf::encode_varint((1 << 3) as u64),
            vec![1u8],
        ]
        .concat();
        return OfficialLsExtensionAction::Close(extension_unary_response(&content_type, &body));
    }

    // 官方扩展会周期性探测 Chrome DevTools MCP URL；对唤醒场景返回空字符串即可。
    if path_matches_rpc_method(&path, "GetChromeDevtoolsMcpUrl") {
        let body = crate::utils::protobuf::encode_string_field(1, "");
        return OfficialLsExtensionAction::Close(extension_unary_response(&content_type, &body));
    }

    // 唤醒场景不提供终端 shell 能力，返回默认值（false/empty）。
    if path_matches_rpc_method(&path, "CheckTerminalShellSupport") {
        return OfficialLsExtensionAction::Close(extension_unary_response(&content_type, &[]));
    }

    // 唤醒场景不使用浏览器 onboarding，返回默认端口 0。
    if path_matches_rpc_method(&path, "GetBrowserOnboardingPort") {
        return OfficialLsExtensionAction::Close(extension_unary_response(&content_type, &[]));
    }

    let empty_ok_paths = [
        "/PushUnifiedStateSyncUpdate",
        "/GetSecretValue",
        "/StoreSecretValue",
        "/LogEvent",
        "/RecordError",
        "/RestartUserStatusUpdater",
        "/OpenSetting",
        "/PlaySound",
        "/BroadcastConversationDeletion",
    ];

    if empty_ok_paths.iter().any(|suffix| {
        path.ends_with(suffix) || path_matches_rpc_method(&path, suffix.trim_start_matches('/'))
    }) {
        return OfficialLsExtensionAction::Close(extension_unary_response(&content_type, &[]));
    }

    crate::modules::logger::log_warn(&format!(
        "[WakeupGateway] 官方 LS 调用了未实现扩展接口: {}",
        path
    ));
    OfficialLsExtensionAction::Close(binary_http_response(
        200,
        "OK",
        if content_type.is_empty() {
            "application/proto"
        } else {
            &content_type
        },
        &[],
    ))
}

async fn run_official_ls_extension_server(
    listener: TcpListener,
    state: Arc<OfficialLsExtensionServerState>,
) {
    loop {
        tokio::select! {
            _ = state.shutdown_notify.notified() => {
                break;
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        let state = state.clone();
                        tokio::spawn(async move {
                            handle_official_ls_extension_connection(stream, state).await;
                        });
                    }
                    Err(err) => {
                        crate::modules::logger::log_error(&format!(
                            "[WakeupGateway] 官方 LS 扩展服务 accept 失败: {}",
                            err
                        ));
                        break;
                    }
                }
            }
        }
    }
}

async fn start_official_ls_extension_server(
    token: &crate::models::token::TokenData,
) -> Result<OfficialLsExtensionServerHandle, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("启动官方 LS 扩展服务失败（绑定端口）: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("读取官方 LS 扩展服务端口失败: {}", e))?
        .port();
    let csrf_token = uuid::Uuid::new_v4().to_string();
    let (started_sender, started_receiver) = oneshot::channel();
    let shutdown_notify = Arc::new(Notify::new());
    let state = Arc::new(OfficialLsExtensionServerState {
        csrf_token: csrf_token.clone(),
        uss_oauth_topic_bytes: build_uss_oauth_topic_bytes(token),
        empty_topic_bytes: Vec::new(),
        started_sender: Mutex::new(Some(started_sender)),
        shutdown_notify: shutdown_notify.clone(),
    });
    let task = tokio::spawn(run_official_ls_extension_server(listener, state));

    Ok(OfficialLsExtensionServerHandle {
        port,
        csrf_token,
        started_receiver,
        shutdown_notify,
        task,
    })
}

fn spawn_ls_log_task<R>(reader: R, tag: &'static str) -> tokio::task::JoinHandle<()>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            crate::modules::logger::log_info(&format!(
                "[WakeupGateway][OfficialLS][{}] {}",
                tag, trimmed
            ));
        }
    })
}

impl OfficialLsProcessHandle {
    async fn shutdown(&mut self) {
        self.extension_server.shutdown_notify.notify_waiters();
        self.extension_server.task.abort();

        if let Some(task) = self.stdout_task.take() {
            task.abort();
        }
        if let Some(task) = self.stderr_task.take() {
            task.abort();
        }

        let _ = self.child.start_kill();
        let _ = timeout(Duration::from_secs(2), self.child.wait()).await;
    }
}

async fn start_official_ls_process(
    account_id: &str,
    token: &crate::models::token::TokenData,
) -> Result<OfficialLsProcessHandle, String> {
    let binary_path = official_ls_binary_path()?;
    let mut extension_server = start_official_ls_extension_server(token).await?;
    let ls_csrf = uuid::Uuid::new_v4().to_string();
    let cloud_code_endpoint = official_ls_cloud_code_endpoint(token);
    let app_data_dir = format!(
        "{}-{}",
        OFFICIAL_LS_APP_DATA_DIR_PREFIX,
        account_id.chars().take(8).collect::<String>()
    );

    let mut cmd = Command::new(&binary_path);
    cmd.arg("--enable_lsp")
        .arg("--random_port")
        .arg("--csrf_token")
        .arg(&ls_csrf)
        .arg("--extension_server_port")
        .arg(extension_server.port.to_string())
        .arg("--extension_server_csrf_token")
        .arg(&extension_server.csrf_token)
        .arg("--cloud_code_endpoint")
        .arg(cloud_code_endpoint)
        .arg("--app_data_dir")
        .arg(&app_data_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动官方 Language Server 失败: {}", e))?;

    if let Some(stdout) = child.stdout.take() {
        crate::modules::logger::log_info(&format!(
            "[WakeupGateway] 官方 LS 已启动进程，等待回调: account_id={}",
            account_id
        ));
        let stdout_task = spawn_ls_log_task(stdout, "stdout");
        let stderr_task = child
            .stderr
            .take()
            .map(|stderr| spawn_ls_log_task(stderr, "stderr"));

        if let Some(mut stdin) = child.stdin.take() {
            let metadata = build_official_ls_metadata_bytes();
            stdin
                .write_all(&metadata)
                .await
                .map_err(|e| format!("写入官方 LS 初始 Metadata 失败: {}", e))?;
            let _ = stdin.shutdown().await;
        }

        let started = timeout(
            OFFICIAL_LS_START_TIMEOUT,
            &mut extension_server.started_receiver,
        )
        .await
        .map_err(|_| "等待官方 LS LanguageServerStarted 超时".to_string())?
        .map_err(|_| "官方 LS LanguageServerStarted 通知通道已关闭".to_string())?;

        crate::modules::logger::log_info(&format!(
            "[WakeupGateway] 官方 LS 启动完成: https_port={}, http_port={}, lsp_port={}",
            started.https_port, started.http_port, started.lsp_port
        ));

        Ok(OfficialLsProcessHandle {
            child,
            stdout_task: Some(stdout_task),
            stderr_task,
            extension_server,
            started,
            ls_csrf_token: ls_csrf,
        })
    } else {
        let _ = child.start_kill();
        Err("官方 LS stdout 不可用".to_string())
    }
}

async fn ensure_wakeup_account_token(
    account_id: &str,
) -> Result<
    (
        crate::models::account::Account,
        crate::models::token::TokenData,
    ),
    String,
> {
    let mut account = crate::modules::account::load_account(account_id)?;
    let token = crate::modules::oauth::ensure_fresh_token(&account.token).await?;
    if token.access_token != account.token.access_token
        || token.refresh_token != account.token.refresh_token
        || token.expiry_timestamp != account.token.expiry_timestamp
        || token.project_id != account.token.project_id
        || token.is_gcp_tos != account.token.is_gcp_tos
    {
        account.token = token.clone();
        let _ = crate::modules::account::save_account(&account);
    }
    Ok((account, token))
}

async fn trigger_wakeup_via_official_language_server(
    account_id: &str,
    model: &str,
    prompt: &str,
    max_output_tokens: u32,
    items: Vec<SendMessageItem>,
    cascade_config: Option<Value>,
) -> Result<crate::modules::wakeup::WakeupResponse, String> {
    let (_account, token) = ensure_wakeup_account_token(account_id).await?;
    let mut ls = start_official_ls_process(account_id, &token).await?;
    let client = build_official_ls_local_client(20)?;
    let base_url = format!("https://127.0.0.1:{}", ls.started.https_port);
    let service_base = format!("{}/exa.language_server_pb.LanguageServerService", base_url);

    let outcome = async {
        let start_resp = post_json_to_official_ls(
            &client,
            &base_url,
            &ls.ls_csrf_token,
            START_CASCADE_PATH,
            &json!({}),
        )
        .await?;
        let cascade_id = start_resp
            .get("cascadeId")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| "官方 LS StartCascade 未返回 cascadeId".to_string())?
            .to_string();

        let items_value = if items.is_empty() {
            json!([{ "text": prompt }])
        } else {
            serde_json::to_value(items).map_err(|e| format!("序列化消息 items 失败: {}", e))?
        };

        let requested_model_fallback = if let Ok(num) = model.trim().parse::<i64>() {
            json!({ "model": num })
        } else {
            json!({ "alias": model })
        };

        let mut cascade_config_value = if let Some(cfg) = cascade_config {
            cfg
        } else {
            let max_tokens = if max_output_tokens > 0 {
                max_output_tokens
            } else {
                8192
            };
            json!({
                "plannerConfig": {
                    "planModel": 1008,
                    "requestedModel": requested_model_fallback,
                    "maxOutputTokens": max_tokens,
                },
                "checkpointConfig": {
                    "maxOutputTokens": max_tokens,
                }
            })
        };

        // 官方 LS 对 plannerConfig.planModel / requestedModel 比较严格，这里做保底修正，
        // 防止上游 JSON 在中间反序列化时丢字段导致 500。
        let max_tokens = if max_output_tokens > 0 {
            max_output_tokens
        } else {
            8192
        };
        if !cascade_config_value.is_object() {
            cascade_config_value = json!({});
        }
        let cfg_obj = cascade_config_value
            .as_object_mut()
            .ok_or_else(|| "内部错误：cascadeConfig 不是 object".to_string())?;
        if !cfg_obj
            .get("plannerConfig")
            .map(|v| v.is_object())
            .unwrap_or(false)
        {
            cfg_obj.insert("plannerConfig".to_string(), json!({}));
        }
        if !cfg_obj
            .get("checkpointConfig")
            .map(|v| v.is_object())
            .unwrap_or(false)
        {
            cfg_obj.insert("checkpointConfig".to_string(), json!({}));
        }
        if let Some(planner_obj) = cfg_obj
            .get_mut("plannerConfig")
            .and_then(|v| v.as_object_mut())
        {
            planner_obj
                .entry("planModel".to_string())
                .or_insert(json!(1008));
            planner_obj
                .entry("requestedModel".to_string())
                .or_insert(requested_model_fallback.clone());
            planner_obj
                .entry("maxOutputTokens".to_string())
                .or_insert(json!(max_tokens));
        }
        if let Some(checkpoint_obj) = cfg_obj
            .get_mut("checkpointConfig")
            .and_then(|v| v.as_object_mut())
        {
            checkpoint_obj
                .entry("maxOutputTokens".to_string())
                .or_insert(json!(max_tokens));
        }

        let send_body = json!({
            "cascadeId": cascade_id,
            "items": items_value,
            "cascadeConfig": cascade_config_value,
        });

        let _ = post_json_to_official_ls(
            &client,
            &base_url,
            &ls.ls_csrf_token,
            SEND_USER_CASCADE_MESSAGE_PATH,
            &send_body,
        )
        .await?;

        let started_at = std::time::Instant::now();
        let mut last_status = String::new();
        let mut last_error: Option<String> = None;
        let mut result: Option<crate::modules::wakeup::WakeupResponse> = None;

        while started_at.elapsed() < OFFICIAL_LS_POLL_TIMEOUT {
            let get_resp = post_json_to_official_ls(
                &client,
                &base_url,
                &ls.ls_csrf_token,
                GET_CASCADE_TRAJECTORY_PATH,
                &json!({ "cascadeId": cascade_id }),
            )
            .await?;

            if let Some(status) = get_resp.get("status").and_then(|v| v.as_str()) {
                last_status = status.to_string();
            }

            let duration_ms = started_at.elapsed().as_millis() as u64;
            if let Some(parsed) =
                extract_wakeup_response_from_official_ls_trajectory(&get_resp, duration_ms)
            {
                result = Some(parsed);
                break;
            }
            if let Some(err) = extract_official_ls_error_from_trajectory(&get_resp) {
                last_error = Some(err);
                break;
            }

            tokio::time::sleep(OFFICIAL_LS_POLL_INTERVAL).await;
        }

        let _ = client
            .post(format!("{}/DeleteCascadeTrajectory", service_base))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("x-codeium-csrf-token", &ls.ls_csrf_token)
            .json(&json!({ "cascadeId": cascade_id }))
            .send()
            .await;

        if let Some(err) = last_error {
            return Err(err);
        }

        result.ok_or_else(|| {
            if last_status.is_empty() {
                "官方 LS 未返回唤醒结果（轨迹中未出现 plannerResponse.modifiedResponse）"
                    .to_string()
            } else {
                format!(
                    "官方 LS 未在超时时间内返回唤醒结果，最后状态={}",
                    last_status
                )
            }
        })
    }
    .await;

    ls.shutdown().await;
    outcome
}

async fn handle_prepare_start_context(body: &[u8]) -> Result<Value, (u16, String)> {
    let req: PrepareStartContextRequest = serde_json::from_slice(body)
        .map_err(|e| (400, format!("prepareStartContext 请求体无效: {}", e)))?;

    let account_id = req.account_id.trim();
    if account_id.is_empty() {
        return Err((400, "缺少 accountId".to_string()));
    }

    let ctx = PreparedStartContext {
        account_id: account_id.to_string(),
        model: trim_non_empty(req.model),
        max_output_tokens: req.max_output_tokens,
        prepared_at: Utc::now(),
    };

    let mut guard = pending_start_contexts()
        .lock()
        .map_err(|_| (500, "准备上下文锁失败".to_string()))?;
    guard.push_back(ctx);

    Ok(json!({}))
}

async fn handle_health_check() -> Result<Value, (u16, String)> {
    Ok(json!({ "ok": true }))
}

async fn handle_start_cascade(body: &[u8]) -> Result<Value, (u16, String)> {
    let payload = parse_json_object_body(body, "StartCascade")?;
    let prepared = pop_prepared_start_context()
        .map_err(|e| (500, e))?
        .ok_or_else(|| {
            (
                400,
                "缺少账号上下文，请先调用内部 prepareStartContext".to_string(),
            )
        })?;

    let (start_resp, session) = start_official_ls_cascade_session(prepared, &payload)
        .await
        .map_err(|e| (500, e))?;

    let cascade_id = start_resp
        .get("cascadeId")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| (500, "官方 LS StartCascade 未返回 cascadeId".to_string()))?
        .to_string();

    let insert_ok = if let Ok(mut guard) = official_ls_sessions().lock() {
        guard.insert(cascade_id, session.clone());
        true
    } else {
        false
    };
    if !insert_ok {
        shutdown_official_ls_session(&session).await;
        return Err((500, "官方 LS 会话映射锁失败".to_string()));
    }

    Ok(start_resp)
}

async fn handle_send_user_cascade_message(body: &[u8]) -> Result<Value, (u16, String)> {
    let payload = parse_json_object_body(body, "SendUserCascadeMessage")?;
    let cascade_id = extract_required_cascade_id(&payload, "SendUserCascadeMessage")?;
    let session = get_official_ls_session(&cascade_id).map_err(|e| {
        if e.starts_with("会话不存在:") {
            (404, e)
        } else {
            (500, e)
        }
    })?;

    proxy_official_ls_session_json_request(&session, SEND_USER_CASCADE_MESSAGE_PATH, &payload)
        .await
        .map_err(|e| (500, e))
}

async fn handle_get_cascade_trajectory(body: &[u8]) -> Result<Value, (u16, String)> {
    let payload = parse_json_object_body(body, "GetCascadeTrajectory")?;
    let cascade_id = extract_required_cascade_id(&payload, "GetCascadeTrajectory")?;
    let session = get_official_ls_session(&cascade_id).map_err(|e| {
        if e.starts_with("会话不存在:") {
            (404, e)
        } else {
            (500, e)
        }
    })?;

    proxy_official_ls_session_json_request(&session, GET_CASCADE_TRAJECTORY_PATH, &payload)
        .await
        .map_err(|e| (500, e))
}

async fn handle_delete_cascade_trajectory(body: &[u8]) -> Result<Value, (u16, String)> {
    let payload = parse_json_object_body(body, "DeleteCascadeTrajectory")?;
    let cascade_id = extract_required_cascade_id(&payload, "DeleteCascadeTrajectory")?;
    let session = remove_official_ls_session(&cascade_id)
        .map_err(|e| (500, e))?
        .ok_or_else(|| (404, format!("会话不存在: {}", cascade_id)))?;

    let proxy_result =
        proxy_official_ls_session_json_request(&session, DELETE_CASCADE_TRAJECTORY_PATH, &payload)
            .await;

    crate::modules::logger::log_info(&format!(
        "[WakeupGateway] 清理官方 LS 会话: cascade_id={}, account_id={}",
        cascade_id, session.account_id
    ));
    shutdown_official_ls_session(&session).await;

    proxy_result.map_err(|e| (500, e))
}

async fn route_request(parsed: ParsedRequest) -> Vec<u8> {
    let path = normalize_path(&parsed.target);
    if parsed.method.eq_ignore_ascii_case("OPTIONS") {
        return options_response();
    }
    if !parsed.method.eq_ignore_ascii_case("POST") {
        return json_response(
            405,
            "Method Not Allowed",
            &json!({ "error": "Only POST is supported" }),
        );
    }

    let result = match path.as_str() {
        INTERNAL_HEALTH_CHECK_PATH => handle_health_check().await,
        INTERNAL_PREPARE_START_CONTEXT_PATH => handle_prepare_start_context(&parsed.body).await,
        START_CASCADE_PATH => handle_start_cascade(&parsed.body).await,
        SEND_USER_CASCADE_MESSAGE_PATH => handle_send_user_cascade_message(&parsed.body).await,
        GET_CASCADE_TRAJECTORY_PATH => handle_get_cascade_trajectory(&parsed.body).await,
        DELETE_CASCADE_TRAJECTORY_PATH => handle_delete_cascade_trajectory(&parsed.body).await,
        _ => Err((404, format!("Unknown path: {}", path))),
    };

    match result {
        Ok(body) => json_response(200, "OK", &body),
        Err((status, message)) => {
            let status_text = match status {
                400 => "Bad Request",
                404 => "Not Found",
                405 => "Method Not Allowed",
                _ => "Internal Server Error",
            };
            json_response(status, status_text, &json!({ "error": message }))
        }
    }
}

async fn handle_connection<S>(mut stream: S)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let response = match read_http_request(&mut stream).await {
        Ok(raw) => match parse_http_request(&raw) {
            Ok(parsed) => route_request(parsed).await,
            Err(err) => json_response(400, "Bad Request", &json!({ "error": err })),
        },
        Err(err) => json_response(400, "Bad Request", &json!({ "error": err })),
    };

    let _ = stream.write_all(&response).await;
    let _ = stream.flush().await;
    let _ = stream.shutdown().await;
}

fn build_tls_acceptor() -> Result<TlsAcceptor, String> {
    let certified =
        generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .map_err(|e| format!("生成本地 TLS 证书失败: {}", e))?;

    let cert_der: Vec<u8> = certified.cert.der().to_vec();
    let key_der: Vec<u8> = certified.key_pair.serialize_der();

    let certs = vec![CertificateDer::from(cert_der)];
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der));

    let mut server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("创建 TLS 配置失败: {}", e))?;
    server_config.alpn_protocols = vec![b"http/1.1".to_vec()];

    Ok(TlsAcceptor::from(Arc::new(server_config)))
}

async fn run_gateway_server(listener: TcpListener, tls_acceptor: TlsAcceptor) {
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let acceptor = tls_acceptor.clone();
                tokio::spawn(async move {
                    match acceptor.accept(stream).await {
                        Ok(tls_stream) => {
                            handle_connection(tls_stream).await;
                        }
                        Err(err) => {
                            crate::modules::logger::log_error(&format!(
                                "[WakeupGateway] TLS 握手失败: {}",
                                err
                            ));
                        }
                    }
                });
            }
            Err(err) => {
                crate::modules::logger::log_error(&format!(
                    "[WakeupGateway] accept 失败，网关停止: {}",
                    err
                ));
                break;
            }
        }
    }
}

pub async fn ensure_local_gateway_started() -> Result<String, String> {
    let store = LOCAL_GATEWAY_BASE_URL.get_or_init(|| TokioMutex::new(None));
    {
        let guard = store.lock().await;
        if let Some(base_url) = guard.as_ref() {
            return Ok(base_url.clone());
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("启动本地唤醒网关失败（绑定端口）: {}", e))?;
    let tls_acceptor = build_tls_acceptor()?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("读取本地唤醒网关端口失败: {}", e))?
        .port();
    let base_url = format!("https://127.0.0.1:{}", port);
    crate::modules::logger::log_info(&format!("[WakeupGateway] 本地网关已启动: {}", base_url));
    tokio::spawn(run_gateway_server(listener, tls_acceptor));

    let mut guard = store.lock().await;
    if let Some(existing) = guard.as_ref() {
        return Ok(existing.clone());
    }
    *guard = Some(base_url.clone());
    Ok(base_url)
}

pub async fn clear_local_gateway_base_url_cache() {
    let store = LOCAL_GATEWAY_BASE_URL.get_or_init(|| TokioMutex::new(None));
    let mut guard = store.lock().await;
    if let Some(current) = guard.take() {
        crate::modules::logger::log_warn(&format!(
            "[WakeupGateway] 清理本地网关地址缓存: {}",
            current
        ));
    }
}
