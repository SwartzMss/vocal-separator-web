use std::{
    collections::HashMap,
    env, fs as stdfs,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path as AxumPath, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::Mutex;
use tokio::time::{Duration, sleep};
use tokio::{fs, fs::File, fs::OpenOptions, io::AsyncWriteExt, net::TcpListener, process::Command};
use tokio_util::io::ReaderStream;
use tracing::{error, info};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

const ALLOWED_EXTENSIONS: &[&str] = &["mp3", "wav", "m4a", "flac", "ogg", "aac"];
const BROWSER_ID_COOKIE: &str = "vs_bid";
const BYPASS_KEY_HEADER: &str = "x-vs-bypass-key";
const JOB_DONE_MARKER: &str = ".done";

static LOG_GUARD: OnceLock<WorkerGuard> = OnceLock::new();

#[derive(Clone)]
struct AppState {
    jobs_dir: PathBuf,
    agent_script: PathBuf,
    python_bin: String,
    daily_limit_per_browser: u32,
    bypass_key: Option<String>,
    usage: Arc<Mutex<HashMap<String, DailyUsage>>>,
    jobs_ttl_seconds: u64,
    jobs_cleanup_interval_seconds: u64,
    request_records_file: PathBuf,
    request_records_lock: Arc<Mutex<()>>,
}

#[derive(Debug, Clone, Copy)]
struct DailyUsage {
    day: u64,
    used: u32,
    in_progress: bool,
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("server error: {err}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let env_filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());

    let log_dir = env::var("LOG_DIR").unwrap_or_else(|_| default_logs_dir());
    let log_dir = absolute_path(log_dir)?;
    stdfs::create_dir_all(&log_dir)?;
    let file_appender = tracing_appender::rolling::never(&log_dir, "backend.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
    let _ = LOG_GUARD.set(guard);

    let stdout_layer = fmt::layer().with_target(false);
    let file_layer = fmt::layer()
        .with_writer(file_writer)
        .with_ansi(false)
        .with_target(false);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(stdout_layer)
        .with(file_layer)
        .init();

    let jobs_dir = env::var("JOBS_DIR").unwrap_or_else(|_| default_jobs_dir());
    let jobs_dir = absolute_path(jobs_dir)?;
    fs::create_dir_all(&jobs_dir).await?;

    let agent_script = env::var("AGENT_SCRIPT").unwrap_or_else(|_| default_agent_script());
    let agent_script = absolute_path(agent_script)?;

    let python_bin = env::var("PYTHON_BIN").unwrap_or_else(|_| "python3".into());

    let daily_limit_per_browser: u32 = env::var("DAILY_LIMIT_PER_BROWSER")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);

    let bypass_key = env::var("BYPASS_KEY").ok().and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });

    let jobs_ttl_seconds: u64 = env::var("JOBS_TTL_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(3600);

    let jobs_cleanup_interval_seconds: u64 = env::var("JOBS_CLEANUP_INTERVAL_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(600);

    let request_records_file =
        env::var("REQUEST_RECORD_FILE").unwrap_or_else(|_| default_request_records_file());
    let request_records_file = absolute_path(request_records_file)?;
    if let Some(parent) = request_records_file.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).await?;
    }
    if let Err(err) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&request_records_file)
        .await
    {
        error!("failed to initialize request record file {request_records_file:?}: {err}");
    }

    let state = AppState {
        jobs_dir,
        agent_script,
        python_bin,
        daily_limit_per_browser,
        bypass_key,
        usage: Arc::new(Mutex::new(HashMap::new())),
        jobs_ttl_seconds,
        jobs_cleanup_interval_seconds,
        request_records_file,
        request_records_lock: Arc::new(Mutex::new(())),
    };

    if state.jobs_ttl_seconds > 0 {
        let cleanup_state = state.clone();
        tokio::spawn(async move {
            jobs_cleanup_loop(cleanup_state).await;
        });
    }

    let router = Router::new()
        .route("/api/jobs", post(create_job))
        .route("/api/jobs/:job_id/vocals", get(get_vocals))
        .route("/api/jobs/:job_id/instrumental", get(get_instrumental))
        .layer(DefaultBodyLimit::max(512 * 1024 * 1024))
        .with_state(state);

    let port: u16 = env::var("PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8000);
    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into());
    let addr: SocketAddr = format!("{host}:{port}").parse()?;

    info!("Rust backend listening on http://{addr}");

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;
    Ok(())
}

fn absolute_path<P: AsRef<Path>>(path: P) -> Result<PathBuf, std::io::Error> {
    let path = path.as_ref();
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn is_backend_workdir() -> bool {
    std::env::current_dir()
        .ok()
        .map(|dir| dir.join("Cargo.toml").exists() && dir.join("src/main.rs").exists())
        .unwrap_or(false)
}

fn default_jobs_dir() -> String {
    if is_backend_workdir() {
        "../jobs".into()
    } else {
        "jobs".into()
    }
}

fn default_logs_dir() -> String {
    if is_backend_workdir() {
        "../logs".into()
    } else {
        "logs".into()
    }
}

fn default_agent_script() -> String {
    let candidates: &[&str] = if is_backend_workdir() {
        &["../python/agent.py", "../agent.py", "agent.py"]
    } else {
        &["python/agent.py", "agent.py"]
    };

    candidates
        .iter()
        .copied()
        .find(|candidate| Path::new(candidate).exists())
        .unwrap_or(candidates[0])
        .to_string()
}

fn default_request_records_file() -> String {
    if is_backend_workdir() {
        "../request_records.txt".into()
    } else {
        "request_records.txt".into()
    }
}

#[derive(Debug, Serialize)]
struct JobResponse {
    job_id: String,
    instrumental_url: String,
    vocals_url: String,
}

#[derive(Debug, Serialize)]
struct RequestRecord {
    ts_rfc3339: String,
    bypass: bool,
    outcome: String,
    filename: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AgentResponse {
    #[allow(dead_code)]
    vocals: String,
    #[allow(dead_code)]
    instrumental: String,
}

struct JobWithMeta {
    response: JobResponse,
    file_name: Option<String>,
}

async fn create_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Response {
    let (browser_id, set_cookie) = get_or_create_browser_id(&headers);
    let bypass = has_valid_bypass_key(&headers, state.bypass_key.as_deref());

    if state.daily_limit_per_browser > 0
        && !bypass
        && let Err(err) = reserve_daily_slot(&state, &browser_id).await
    {
        let ts_rfc3339 = now_timestamp_rfc3339();
        append_request_record(
            &state,
            RequestRecord {
                ts_rfc3339,
                bypass,
                outcome: err.outcome().to_string(),
                filename: None,
                error: Some(err.to_string()),
            },
        )
        .await;

        let mut response = err.into_response();
        if let Some(cookie) = set_cookie {
            response.headers_mut().insert(header::SET_COOKIE, cookie);
        }
        return response;
    }

    let result = create_job_inner(&state, multipart).await;
    if state.daily_limit_per_browser > 0 && !bypass {
        match &result {
            Ok(_) => mark_daily_success(&state, &browser_id).await,
            Err(_) => release_daily_slot(&state, &browser_id).await,
        }
    }

    match &result {
        Ok(job) => {
            let ts_rfc3339 = now_timestamp_rfc3339();
            append_request_record(
                &state,
                RequestRecord {
                    ts_rfc3339,
                    bypass,
                    outcome: "success".into(),
                    filename: job.file_name.clone(),
                    error: None,
                },
            )
            .await;
        }
        Err(err) => {
            let ts_rfc3339 = now_timestamp_rfc3339();
            append_request_record(
                &state,
                RequestRecord {
                    ts_rfc3339,
                    bypass,
                    outcome: err.outcome().to_string(),
                    filename: None,
                    error: Some(err.to_string()),
                },
            )
            .await;
        }
    }

    let mut response = result.map(|job| Json(job.response)).into_response();
    if let Some(cookie) = set_cookie {
        response.headers_mut().insert(header::SET_COOKIE, cookie);
    }
    response
}

fn now_timestamp_rfc3339() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let unix_ms = duration.as_millis() as u64;
    format_unix_ms_rfc3339_local(unix_ms)
}

fn format_unix_ms_rfc3339_local(unix_ms: u64) -> String {
    let secs = (unix_ms / 1000) as i64;
    let millis = (unix_ms % 1000) as u32;
    let offset_seconds = local_offset_seconds(secs)
        .map(i64::from)
        .filter(|offset| offset.rem_euclid(60) == 0)
        .unwrap_or(0);
    let local_secs = secs.saturating_add(offset_seconds);

    let days = local_secs.div_euclid(86_400);
    let secs_of_day = local_secs.rem_euclid(86_400) as u32;

    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;

    let (year, month, day) = civil_from_days(days);

    let (offset_sign, offset_abs) = if offset_seconds >= 0 {
        ('+', offset_seconds as u32)
    } else {
        ('-', (-offset_seconds) as u32)
    };
    let offset_hour = offset_abs / 3600;
    let offset_minute = (offset_abs % 3600) / 60;

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}{}{:02}:{:02}",
        year, month, day, hour, minute, second, millis, offset_sign, offset_hour, offset_minute
    )
}

#[cfg(target_os = "linux")]
fn local_offset_seconds(unix_seconds: i64) -> Option<i32> {
    let t: libc::time_t = unix_seconds;
    let mut local_tm: libc::tm = unsafe { std::mem::zeroed() };
    let result = unsafe { libc::localtime_r(&t, &mut local_tm) };
    if result.is_null() {
        return None;
    }
    Some(local_tm.tm_gmtoff as i32)
}

#[cfg(not(target_os = "linux"))]
fn local_offset_seconds(_unix_seconds: i64) -> Option<i32> {
    None
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096).div_euclid(365);
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2).div_euclid(153);
    let d = doy - (153 * mp + 2).div_euclid(5) + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = (y + if m <= 2 { 1 } else { 0 }) as i32;
    let month = m as u32;
    let day = d as u32;
    (year, month, day)
}

async fn append_request_record(state: &AppState, record: RequestRecord) {
    let line = match serde_json::to_string(&record) {
        Ok(line) => line,
        Err(err) => {
            error!("failed to serialize request record: {err}");
            return;
        }
    };

    let _guard = state.request_records_lock.lock().await;
    let mut file = match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&state.request_records_file)
        .await
    {
        Ok(file) => file,
        Err(err) => {
            error!(
                "failed to open request record file {:?}: {err}",
                state.request_records_file
            );
            return;
        }
    };

    if let Err(err) = file.write_all(line.as_bytes()).await {
        error!(
            "failed to write request record to {:?}: {err}",
            state.request_records_file
        );
        return;
    }
    if let Err(err) = file.write_all(b"\n").await {
        error!(
            "failed to write request record newline to {:?}: {err}",
            state.request_records_file
        );
    }
}

async fn create_job_inner(
    state: &AppState,
    mut multipart: Multipart,
) -> Result<JobWithMeta, AppError> {
    while let Some(field) = multipart.next_field().await? {
        if field.name() == Some("file") {
            let response = handle_file_upload(state, field).await?;
            return Ok(response);
        }
    }
    Err(AppError::BadRequest("file field missing".into()))
}

async fn handle_file_upload(
    state: &AppState,
    field: axum::extract::multipart::Field<'_>,
) -> Result<JobWithMeta, AppError> {
    let file_name = field
        .file_name()
        .map(str::to_string)
        .ok_or_else(|| AppError::BadRequest("filename missing".into()))?;
    let extension = Path::new(&file_name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
        .ok_or_else(|| AppError::BadRequest("unable to detect extension".into()))?;

    if !is_allowed_extension(&extension) {
        return Err(AppError::BadRequest(format!(
            "unsupported file type: .{}",
            extension
        )));
    }

    let job_id = Uuid::new_v4().to_string();
    let job_dir = state.jobs_dir.join(&job_id);
    fs::create_dir_all(&job_dir).await?;

    let input_path = job_dir.join(format!("input.{}", extension));
    if let Err(err) = save_upload(field, &input_path).await {
        let _ = fs::remove_dir_all(&job_dir).await;
        return Err(err);
    }

    if let Err(err) = run_agent(state, &input_path, &job_dir).await {
        let _ = fs::remove_dir_all(&job_dir).await;
        return Err(err);
    }

    if let Err(err) = fs::write(job_dir.join(JOB_DONE_MARKER), "ok").await {
        error!("failed to write job marker for {job_id}: {err}");
    }

    info!("Job {} completed", job_id);

    Ok(JobWithMeta {
        response: JobResponse {
            job_id: job_id.clone(),
            instrumental_url: format!("/api/jobs/{job_id}/instrumental"),
            vocals_url: format!("/api/jobs/{job_id}/vocals"),
        },
        file_name: Some(file_name),
    })
}

async fn save_upload(
    mut field: axum::extract::multipart::Field<'_>,
    path: &Path,
) -> Result<(), AppError> {
    let mut file = File::create(path).await?;
    while let Some(chunk) = field.chunk().await? {
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    Ok(())
}

async fn run_agent(
    state: &AppState,
    input_path: &Path,
    job_dir: &Path,
) -> Result<AgentResponse, AppError> {
    let mut cmd = Command::new(&state.python_bin);
    cmd.arg(&state.agent_script)
        .arg("--input")
        .arg(input_path)
        .arg("--output-dir")
        .arg(job_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = cmd
        .output()
        .await
        .map_err(|err| AppError::AgentFailure(format!("failed to spawn agent: {err}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(AppError::AgentFailure(format!(
            "agent exited with {}: {}",
            output.status, stderr
        )));
    }

    let response: AgentResponse = serde_json::from_slice(&output.stdout)?;
    Ok(response)
}

async fn get_vocals(
    State(state): State<AppState>,
    AxumPath(job_id): AxumPath<String>,
) -> Result<Response, AppError> {
    serve_audio(&state, &job_id, "vocals.wav").await
}

async fn get_instrumental(
    State(state): State<AppState>,
    AxumPath(job_id): AxumPath<String>,
) -> Result<Response, AppError> {
    serve_audio(&state, &job_id, "instrumental.wav").await
}

async fn serve_audio(state: &AppState, job_id: &str, filename: &str) -> Result<Response, AppError> {
    let path = state.jobs_dir.join(job_id).join(filename);
    let file = match File::open(&path).await {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(AppError::NotFound);
        }
        Err(err) => return Err(AppError::Io(err)),
    };

    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("audio/wav"));
    let disposition = format!("attachment; filename=\"{filename}\"");
    if let Ok(value) = HeaderValue::from_str(&disposition) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }

    Ok((headers, body).into_response())
}

fn is_allowed_extension(ext: &str) -> bool {
    ALLOWED_EXTENSIONS.contains(&ext)
}

fn utc_day_number() -> u64 {
    let now = SystemTime::now();
    let duration = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    duration.as_secs() / 86_400
}

async fn jobs_cleanup_loop(state: AppState) {
    let interval = Duration::from_secs(state.jobs_cleanup_interval_seconds.max(60));
    loop {
        sleep(interval).await;
        if let Err(err) = cleanup_expired_jobs(&state).await {
            error!("jobs cleanup error: {err}");
        }
    }
}

async fn cleanup_expired_jobs(state: &AppState) -> Result<(), std::io::Error> {
    if state.jobs_ttl_seconds == 0 {
        return Ok(());
    }
    let ttl = Duration::from_secs(state.jobs_ttl_seconds);
    let now = SystemTime::now();

    let mut entries = fs::read_dir(&state.jobs_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let file_type = entry.file_type().await?;
        if !file_type.is_dir() {
            continue;
        }

        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if Uuid::parse_str(name_str).is_err() {
            continue;
        }

        let job_dir = entry.path();
        let Some(completed_at) = job_completed_at(&job_dir).await else {
            continue;
        };

        let age = now.duration_since(completed_at).unwrap_or_default();
        if age < ttl {
            continue;
        }

        match fs::remove_dir_all(&job_dir).await {
            Ok(()) => info!("Job {name_str} expired and removed"),
            Err(err) => error!("failed to remove expired job dir {job_dir:?}: {err}"),
        }
    }

    Ok(())
}

async fn job_completed_at(job_dir: &Path) -> Option<SystemTime> {
    let marker = job_dir.join(JOB_DONE_MARKER);
    if let Ok(metadata) = fs::metadata(&marker).await
        && let Ok(modified) = metadata.modified()
    {
        return Some(modified);
    }

    let vocals = job_dir.join("vocals.wav");
    let instrumental = job_dir.join("instrumental.wav");
    let vocals_modified = fs::metadata(&vocals).await.ok()?.modified().ok()?;
    let instrumental_modified = fs::metadata(&instrumental).await.ok()?.modified().ok()?;
    Some(vocals_modified.max(instrumental_modified))
}

fn has_valid_bypass_key(headers: &HeaderMap, expected: Option<&str>) -> bool {
    let Some(expected) = expected else {
        return false;
    };
    let Some(actual) = headers
        .get(BYPASS_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    actual.trim() == expected
}

fn get_or_create_browser_id(headers: &HeaderMap) -> (String, Option<HeaderValue>) {
    if let Some(existing) = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookie| get_cookie_value(cookie, BROWSER_ID_COOKIE))
        .filter(|value| is_reasonable_cookie_value(value))
    {
        return (existing, None);
    }

    let browser_id = Uuid::new_v4().to_string();
    let cookie = format!(
        "{name}={value}; Path=/; Max-Age=31536000; SameSite=Lax; HttpOnly",
        name = BROWSER_ID_COOKIE,
        value = browser_id
    );
    let header = HeaderValue::from_str(&cookie).ok();
    (browser_id, header)
}

fn get_cookie_value(cookie: &str, name: &str) -> Option<String> {
    for part in cookie.split(';') {
        let trimmed = part.trim();
        let (key, value) = trimmed.split_once('=')?;
        if key == name {
            return Some(value.to_string());
        }
    }
    None
}

fn is_reasonable_cookie_value(value: &str) -> bool {
    let len = value.len();
    (16..=128).contains(&len)
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
}

async fn reserve_daily_slot(state: &AppState, browser_id: &str) -> Result<(), AppError> {
    let today = utc_day_number();
    let mut usage = state.usage.lock().await;
    let entry = usage.entry(browser_id.to_string()).or_insert(DailyUsage {
        day: today,
        used: 0,
        in_progress: false,
    });

    if entry.day != today {
        entry.day = today;
        entry.used = 0;
        entry.in_progress = false;
    }

    if entry.in_progress || entry.used >= state.daily_limit_per_browser {
        return Err(AppError::TooManyRequests(
            "每日仅可使用一次；如需无限制请填写 Key。".into(),
        ));
    }

    entry.in_progress = true;
    Ok(())
}

async fn release_daily_slot(state: &AppState, browser_id: &str) {
    let today = utc_day_number();
    let mut usage = state.usage.lock().await;
    let Some(entry) = usage.get_mut(browser_id) else {
        return;
    };
    if entry.day == today {
        entry.in_progress = false;
    }
}

async fn mark_daily_success(state: &AppState, browser_id: &str) {
    let today = utc_day_number();
    let mut usage = state.usage.lock().await;
    let entry = usage.entry(browser_id.to_string()).or_insert(DailyUsage {
        day: today,
        used: 0,
        in_progress: false,
    });

    if entry.day != today {
        entry.day = today;
        entry.used = 0;
        entry.in_progress = false;
    }

    entry.in_progress = false;
    entry.used = entry.used.saturating_add(1);
}

#[derive(Debug, Error)]
enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    TooManyRequests(String),
    #[error("job not found")]
    NotFound,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("multipart error: {0}")]
    Multipart(#[from] axum::extract::multipart::MultipartError),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    AgentFailure(String),
}

impl AppError {
    fn outcome(&self) -> &'static str {
        match self {
            AppError::BadRequest(_) => "bad_request",
            AppError::TooManyRequests(_) => "too_many_requests",
            AppError::NotFound => "not_found",
            AppError::AgentFailure(_)
            | AppError::Io(_)
            | AppError::Json(_)
            | AppError::Multipart(_) => "error",
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match &self {
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()).into_response(),
            AppError::TooManyRequests(_) => {
                (StatusCode::TOO_MANY_REQUESTS, self.to_string()).into_response()
            }
            AppError::NotFound => (StatusCode::NOT_FOUND, self.to_string()).into_response(),
            AppError::AgentFailure(_)
            | AppError::Io(_)
            | AppError::Json(_)
            | AppError::Multipart(_) => {
                error!("{self}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error").into_response()
            }
        }
    }
}
