mod config;
mod report;
mod serve_sync;
mod sync_meta;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::{Parser, Subcommand};
use kvitto_core::{Categorizer, Overrides, ProfileId, Rules, SourceId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tower_http::services::ServeDir;

const OVERRIDES_PATH: &str = "data/category_overrides.json";
const CATEGORIES_PATH: &str = "data/categories.toml";
const CONFIG_PATH: &str = "config.toml";

#[derive(Parser)]
#[command(name = "kvittokartan")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Re-parse and re-categorize everything already fetched, rebuild the report — no login needed.
    Reparse {
        #[arg(long)]
        force: bool,
    },
    /// Generate the static dashboard (out/report.html) from whatever's already synced.
    Report,
    /// Serve the dashboard on the home network (0.0.0.0). Per-source Uppdatera buttons.
    Serve {
        #[arg(long, default_value_t = 7878)]
        port: u16,
    },
}

fn load_categorizer() -> anyhow::Result<Categorizer> {
    let raw = std::fs::read_to_string(CATEGORIES_PATH).unwrap_or_default();
    let rules: Rules = toml::from_str(&raw).unwrap_or_default();
    let overrides_raw = std::fs::read_to_string(OVERRIDES_PATH).unwrap_or_else(|_| "{}".into());
    let overrides: Overrides = serde_json::from_str(&overrides_raw).unwrap_or_default();
    Ok(Categorizer { rules, overrides })
}

fn build_report(profiles: &[ProfileId]) -> anyhow::Result<()> {
    let store = kvitto_core::FsStore::new("data");
    let html = report::render_html(&store, profiles, "data")?;
    std::fs::create_dir_all("out")?;
    std::fs::write("out/report.html", &html)?;
    eprintln!("Rapport genererad: out/report.html");
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let profiles = config::load_profiles(CONFIG_PATH)?;

    match cli.command {
        Command::Reparse { force } => {
            let state = Arc::new(serve_sync::AppState::new("data".to_string(), profiles));
            serve_sync::reparse(&state, force)?;
            eprintln!("Klart.");
        }
        Command::Report => {
            build_report(&profiles)?;
        }
        Command::Serve { port } => {
            run_server(port, profiles).await?;
        }
    }

    Ok(())
}

async fn run_server(port: u16, profiles: Vec<ProfileId>) -> anyhow::Result<()> {
    let mut state = serve_sync::AppState::new("data".to_string(), profiles);
    state.base_url = Some(format!("http://localhost:{port}"));
    let state = Arc::new(state);

    // Build once at startup so `out/report.html` exists even before the
    // first Uppdatera click.
    if let Err(e) = report::render_html(&state.store, &state.profiles, &state.data_root)
        .and_then(|html| {
            std::fs::create_dir_all("out")?;
            std::fs::write("out/report.html", html)?;
            Ok(())
        })
    {
        tracing::warn!("initial report build failed: {e}");
    }

    let app = Router::new()
        .route("/api/overrides", get(get_overrides).delete(delete_overrides))
        .route("/api/overrides/set", post(post_override))
        .route("/api/profiles", get(get_profiles))
        .route("/api/sync/start", post(post_sync_start))
        .route("/api/sync/status/:profile/:source", get(get_sync_status))
        .route("/api/reparse", post(post_reparse))
        .fallback_service(ServeDir::new("out"))
        .with_state(state);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("Serverar out/ på http://{addr}/report.html (LAN — ingen auth)");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn read_overrides_map() -> HashMap<String, String> {
    let raw = tokio::fs::read_to_string(OVERRIDES_PATH).await.unwrap_or_else(|_| "{}".to_string());
    serde_json::from_str(&raw).unwrap_or_default()
}

async fn get_overrides() -> Json<HashMap<String, String>> {
    Json(read_overrides_map().await)
}

async fn write_overrides_map(map: &HashMap<String, String>) -> StatusCode {
    if let Some(parent) = std::path::Path::new(OVERRIDES_PATH).parent() {
        if tokio::fs::create_dir_all(parent).await.is_err() {
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    }
    let pretty = match serde_json::to_string_pretty(map) {
        Ok(s) => s,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR,
    };
    match tokio::fs::write(OVERRIDES_PATH, pretty).await {
        Ok(()) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[derive(Deserialize)]
struct SetOverrideRequest {
    /// The chain-prefixed override key (`kvitto_core::categorize::override_key`),
    /// not a bare product name — two chains' identically-named products
    /// must stay independently recategorizable.
    key: String,
    category: String,
}

/// One change at a time — read-modify-write, so two people recategorizing
/// different items at once never clobber each other (unlike a full PUT of
/// the whole overrides map).
async fn post_override(Json(body): Json<SetOverrideRequest>) -> StatusCode {
    let mut map = read_overrides_map().await;
    map.insert(body.key, body.category);
    write_overrides_map(&map).await
}

async fn delete_overrides() -> StatusCode {
    write_overrides_map(&HashMap::new()).await
}

async fn get_profiles(State(state): State<Arc<serve_sync::AppState>>) -> Json<Vec<String>> {
    Json(state.profiles.iter().map(|p| p.0.clone()).collect())
}

#[derive(Deserialize)]
struct StartRequest {
    profile: String,
    source: String,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
enum SyncStatusView {
    Idle,
    Authenticating { qr_code: Option<String>, autostart_link: Option<String>, hint: String },
    Working,
    Done { newly_fetched: usize },
    Error { message: String },
}

fn source_id(name: &str) -> Option<SourceId> {
    [kvitto_core::WILLYS, kvitto_core::ICA].into_iter().find(|s| s.0 == name)
}

fn view_of(state: Option<kvitto_core::JobState>) -> SyncStatusView {
    let Some(job) = state else { return SyncStatusView::Idle };
    match job.phase {
        kvitto_core::Phase::Idle => SyncStatusView::Idle,
        kvitto_core::Phase::Authenticating { prompt, status, .. } => {
            let (qr_code, autostart_link) = match prompt {
                Some(kvitto_core::AuthPrompt::BankId { autostart_url, qr_payload }) => {
                    (qr_payload, autostart_url)
                }
                _ => (None, None),
            };
            SyncStatusView::Authenticating { qr_code, autostart_link, hint: status }
        }
        kvitto_core::Phase::Listing { .. }
        | kvitto_core::Phase::Fetching { .. }
        | kvitto_core::Phase::Categorizing
        | kvitto_core::Phase::Rebuilding => SyncStatusView::Working,
        kvitto_core::Phase::Done => {
            let newly_fetched =
                job.reports.iter().map(|(_, r)| r.downloaded).sum();
            SyncStatusView::Done { newly_fetched }
        }
        kvitto_core::Phase::Failed { error } => SyncStatusView::Error { message: error },
    }
}

async fn post_sync_start(
    State(state): State<Arc<serve_sync::AppState>>,
    Json(body): Json<StartRequest>,
) -> Result<Json<SyncStatusView>, (StatusCode, String)> {
    if !state.profiles.iter().any(|p| p.0 == body.profile) {
        return Err((StatusCode::BAD_REQUEST, "okänd profil".to_string()));
    }
    let source = source_id(&body.source).ok_or((StatusCode::BAD_REQUEST, "okänd källa".to_string()))?;

    let id = serve_sync::start_sync(state.clone(), ProfileId(body.profile), source)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let job = serve_sync::poll(&state, id);
    Ok(Json(view_of(job)))
}

async fn get_sync_status(
    State(state): State<Arc<serve_sync::AppState>>,
    Path((profile, source)): Path<(String, String)>,
) -> Json<SyncStatusView> {
    let job = source_id(&source).and_then(|s| state.jobs.status_for(&ProfileId(profile), s));
    Json(view_of(job))
}

async fn post_reparse(State(state): State<Arc<serve_sync::AppState>>) -> StatusCode {
    match serve_sync::reparse(&state, false) {
        Ok(()) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
