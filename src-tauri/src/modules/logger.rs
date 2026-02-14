use crate::modules::account::get_data_dir;
use chrono::{DateTime, Duration, Local};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

const LOG_FILE_PREFIX: &str = "app.log";
const LOG_RETENTION_DAYS: i64 = 3;

struct LocalTimer;

impl tracing_subscriber::fmt::time::FormatTime for LocalTimer {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        let now = chrono::Local::now();
        write!(w, "{}", now.to_rfc3339())
    }
}

pub fn get_log_dir() -> Result<PathBuf, String> {
    let data_dir = get_data_dir()?;
    let log_dir = data_dir.join("logs");

    if !log_dir.exists() {
        fs::create_dir_all(&log_dir).map_err(|e| format!("创建日志目录失败: {}", e))?;
    }

    Ok(log_dir)
}

fn is_app_log_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with(LOG_FILE_PREFIX))
        .unwrap_or(false)
}

fn cleanup_expired_logs(log_dir: &Path) {
    let cutoff = Local::now() - Duration::days(LOG_RETENTION_DAYS);
    let entries = match fs::read_dir(log_dir) {
        Ok(entries) => entries,
        Err(err) => {
            warn!("读取日志目录失败，跳过清理: {}", err);
            return;
        }
    };

    let mut removed_count = 0usize;

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                warn!("读取日志文件失败，已忽略: {}", err);
                continue;
            }
        };

        let path = entry.path();
        if !path.is_file() || !is_app_log_file(&path) {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(err) => {
                warn!("读取日志元数据失败，已忽略: {:?}, {}", path, err);
                continue;
            }
        };

        let modified_at = match metadata.modified() {
            Ok(time) => {
                let dt: DateTime<Local> = time.into();
                dt
            }
            Err(err) => {
                warn!("读取日志修改时间失败，已忽略: {:?}, {}", path, err);
                continue;
            }
        };

        if modified_at >= cutoff {
            continue;
        }

        match fs::remove_file(&path) {
            Ok(_) => removed_count += 1,
            Err(err) => warn!("删除过期日志失败，已忽略: {:?}, {}", path, err),
        }
    }

    if removed_count > 0 {
        info!(
            "日志清理完成：删除 {} 个超过 {} 天的日志文件",
            removed_count, LOG_RETENTION_DAYS
        );
    }
}

/// 初始化日志系统
pub fn init_logger() {
    let _ = tracing_log::LogTracer::init();

    let log_dir = match get_log_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("无法初始化日志目录: {}", e);
            return;
        }
    };

    let file_appender = tracing_appender::rolling::daily(log_dir.clone(), "app.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    let console_layer = fmt::Layer::new()
        .with_target(false)
        .with_thread_ids(false)
        .with_level(true)
        .with_timer(LocalTimer);

    let file_layer = fmt::Layer::new()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(true)
        .with_level(true)
        .with_timer(LocalTimer);

    let filter_layer = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let _ = tracing_subscriber::registry()
        .with(filter_layer)
        .with(console_layer)
        .with(file_layer)
        .try_init();

    std::mem::forget(_guard);

    cleanup_expired_logs(&log_dir);

    info!("日志系统已完成初始化");
}

pub fn log_info(message: &str) {
    info!("{}", message);
}

pub fn log_warn(message: &str) {
    warn!("{}", message);
}

pub fn log_error(message: &str) {
    error!("{}", message);
}
