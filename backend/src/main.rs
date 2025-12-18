use std::{
    env, fs as stdfs,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::Stdio,
    sync::OnceLock,
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
use tokio::{fs, fs::File, io::AsyncWriteExt, net::TcpListener, process::Command};
use tokio_util::io::ReaderStream;
use tracing::{error, info};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

const ALLOWED_EXTENSIONS: &[&str] = &["mp3", "wav", "m4a", "flac", "ogg", "aac"];

static LOG_GUARD: OnceLock<WorkerGuard> = OnceLock::new();

#[derive(Clone)]
struct AppState {
    jobs_dir: PathBuf,
    agent_script: PathBuf,
    python_bin: String,
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

    let log_dir = env::var("LOG_DIR").unwrap_or_else(|_| "logs".into());
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

    let jobs_dir = env::var("JOBS_DIR").unwrap_or_else(|_| "jobs".into());
    let jobs_dir = absolute_path(jobs_dir)?;
    fs::create_dir_all(&jobs_dir).await?;

    let agent_script = env::var("AGENT_SCRIPT").unwrap_or_else(|_| "agent.py".into());
    let agent_script = absolute_path(agent_script)?;

    let python_bin = env::var("PYTHON_BIN").unwrap_or_else(|_| "python3".into());

    let state = AppState {
        jobs_dir,
        agent_script,
        python_bin,
    };

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

#[derive(Debug, Serialize)]
struct JobResponse {
    job_id: String,
    instrumental_url: String,
    vocals_url: String,
}

#[derive(Debug, Deserialize)]
struct AgentResponse {
    #[allow(dead_code)]
    vocals: String,
    #[allow(dead_code)]
    instrumental: String,
}

async fn create_job(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<JobResponse>, AppError> {
    while let Some(field) = multipart.next_field().await? {
        if field.name() == Some("file") {
            let response = handle_file_upload(&state, field).await?;
            return Ok(Json(response));
        }
    }
    Err(AppError::BadRequest("file field missing".into()))
}

async fn handle_file_upload(
    state: &AppState,
    field: axum::extract::multipart::Field<'_>,
) -> Result<JobResponse, AppError> {
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

    info!("Job {} completed", job_id);

    Ok(JobResponse {
        job_id: job_id.clone(),
        instrumental_url: format!("/api/jobs/{job_id}/instrumental"),
        vocals_url: format!("/api/jobs/{job_id}/vocals"),
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

#[derive(Debug, Error)]
enum AppError {
    #[error("{0}")]
    BadRequest(String),
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

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match &self {
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()).into_response(),
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
