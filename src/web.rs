//! The web user interface: a JSON API over the core plus one embedded page, served on loopback.

use anyhow::Result;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tagrss::{Core, SyncEvent, SyncSummary};

const INDEX: &str = include_str!("web/index.html");

/// Sync progress as the page sees it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncState {
    pub running: bool,
    pub done: usize,
    pub total: usize,
    pub new_entries: usize,
    pub failures: Vec<Failure>,
    pub finished: Option<SyncSummary>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Failure {
    pub source: String,
    pub error: String,
}

pub struct AppState {
    /// Every core operation is milliseconds; a sync runs on its own thread and connection,
    /// so it never holds this lock.
    core: Mutex<Core>,
    sync: Arc<Mutex<SyncState>>,
}

type Shared = Arc<AppState>;

struct ApiError(anyhow::Error);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": format!("{:#}", self.0) }))).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        ApiError(e.into())
    }
}

type ApiResult<T> = std::result::Result<Json<T>, ApiError>;

pub fn router(core: Core) -> Router {
    let state = Arc::new(AppState { core: Mutex::new(core), sync: Arc::new(Mutex::new(SyncState::default())) });
    Router::new()
        .route("/", get(|| async { Html(INDEX) }))
        .route("/api/folders", get(folders))
        .route("/api/entries", get(entries))
        .route("/api/entries/{id}/read", post(set_read))
        .route("/api/entries/{id}/open", post(open_entry))
        .route("/api/folders/read", post(mark_folder_read))
        .route("/api/sync", get(sync_state).post(start_sync))
        .route("/api/reload", post(reload))
        .route("/api/sources", get(sources))
        .with_state(state)
}

fn core(state: &Shared) -> std::sync::MutexGuard<'_, Core> {
    state.core.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

async fn folders(State(state): State<Shared>) -> ApiResult<Vec<tagrss::FolderSummary>> {
    Ok(Json(core(&state).folders()?))
}

#[derive(Deserialize)]
struct FolderQuery {
    folder: String,
}

async fn entries(State(state): State<Shared>, Query(q): Query<FolderQuery>) -> ApiResult<Vec<tagrss::Entry>> {
    Ok(Json(core(&state).entries(&q.folder)?))
}

#[derive(Deserialize)]
struct ReadBody {
    read: bool,
}

async fn set_read(State(state): State<Shared>, Path(id): Path<i64>, Json(body): Json<ReadBody>) -> ApiResult<serde_json::Value> {
    core(&state).mark(&[id], body.read)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn open_entry(State(state): State<Shared>, Path(id): Path<i64>) -> ApiResult<serde_json::Value> {
    let url = core(&state).open_entry(id)?;
    Ok(Json(serde_json::json!({ "url": url })))
}

#[derive(Deserialize)]
struct FolderBody {
    folder: String,
}

async fn mark_folder_read(State(state): State<Shared>, Json(body): Json<FolderBody>) -> ApiResult<serde_json::Value> {
    let marked = core(&state).mark_folder_read(&body.folder)?;
    Ok(Json(serde_json::json!({ "marked": marked })))
}

async fn reload(State(state): State<Shared>) -> ApiResult<serde_json::Value> {
    let warnings = core(&state).reload_config()?;
    Ok(Json(serde_json::json!({ "warnings": warnings })))
}

async fn sources(State(state): State<Shared>) -> ApiResult<Vec<tagrss::SourceStatus>> {
    Ok(Json(core(&state).sources()?))
}

async fn sync_state(State(state): State<Shared>) -> Json<SyncState> {
    Json(state.sync.lock().unwrap().clone())
}

async fn start_sync(State(state): State<Shared>) -> Response {
    let (syncer, titles) = {
        let core = core(&state);
        let titles: HashMap<String, String> = match core.sources() {
            Ok(sources) => sources.into_iter().map(|s| (s.url, s.title)).collect(),
            Err(e) => return ApiError(e).into_response(),
        };
        (core.syncer(), titles)
    };
    {
        let mut sync = state.sync.lock().unwrap();
        if sync.running {
            return (StatusCode::CONFLICT, Json(sync.clone())).into_response();
        }
        *sync = SyncState { running: true, total: titles.len(), ..SyncState::default() };
    }
    let progress = state.sync.clone();
    std::thread::spawn(move || {
        let result = syncer.run(|event| {
            let mut p = progress.lock().unwrap();
            match event {
                SyncEvent::Started { sources } => p.total = *sources,
                SyncEvent::Fetched { new_observations, .. } => {
                    p.done += 1;
                    p.new_entries += new_observations;
                }
                SyncEvent::Failed { url, error } => {
                    p.done += 1;
                    let source = titles.get(url).cloned().unwrap_or_else(|| url.clone());
                    p.failures.push(Failure { source, error: error.clone() });
                }
            }
        });
        let mut p = progress.lock().unwrap();
        p.running = false;
        match result {
            Ok(summary) => p.finished = Some(summary),
            Err(e) => p.error = Some(format!("{e:#}")),
        }
    });
    Json(state.sync.lock().unwrap().clone()).into_response()
}

/// Serves on 127.0.0.1 only; port 0 picks a free one. Opens the browser unless told not to.
pub fn run(core: Core, port: u16, open_browser: bool) -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
        let url = format!("http://{}/", listener.local_addr()?);
        println!("TagRss is running at {url}  (Ctrl-C to stop)");
        if open_browser {
            let _ = crate::open_in_browser(&url);
        }
        axum::serve(listener, router(core)).await?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    const CONFIG: &str = r#"
        [[source]]
        url = "http://127.0.0.1:9/feed"
        title = "Local"
        tags = ["topic/test"]

        [[folder]]
        name = "Test folder"
        rule = "topic/test"
    "#;

    fn app() -> (tempfile::TempDir, Router) {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("tagrss.toml");
        std::fs::write(&config, CONFIG).unwrap();
        let (core, _) = Core::open(&config, &dir.path().join("tagrss.db")).unwrap();
        (dir, router(core))
    }

    async fn call(app: &Router, method: &str, uri: &str, body: &str) -> (StatusCode, serde_json::Value) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
    }

    #[tokio::test]
    async fn serves_the_page_and_folder_summaries() {
        let (_dir, app) = app();
        let page = app.clone().oneshot(Request::get("/").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(page.status(), StatusCode::OK);
        let (status, folders) = call(&app, "GET", "/api/folders", "").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(folders, serde_json::json!([{ "name": "Test folder", "unread": 0, "total": 0 }]));
        let (status, entries) = call(&app, "GET", "/api/entries?folder=Test%20folder", "").await;
        assert_eq!((status, entries), (StatusCode::OK, serde_json::json!([])));
    }

    #[tokio::test]
    async fn errors_are_json_with_a_message() {
        let (_dir, app) = app();
        let (status, body) = call(&app, "GET", "/api/entries?folder=Nope", "").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("Test folder"));
        let (status, body) = call(&app, "POST", "/api/entries/42/read", r#"{"read":true}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "no entry 42");
    }

    #[tokio::test]
    async fn sync_runs_once_at_a_time_and_records_failures() {
        let (_dir, app) = app();
        let (status, state) = call(&app, "POST", "/api/sync", "").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(state["running"], true);
        // Port 9 refuses connections, so the sync ends quickly with one failure.
        let state = loop {
            let (_, state) = call(&app, "GET", "/api/sync", "").await;
            if state["running"] == false {
                break state;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };
        assert_eq!(state["finished"]["failed"], 1);
        assert_eq!(state["failures"][0]["source"], "Local");
        let (_, sources) = call(&app, "GET", "/api/sources", "").await;
        assert!(sources[0]["error"].is_string());
    }
}
