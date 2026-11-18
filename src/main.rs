//! cadforge — a CAD-shaped web studio for training small models that
//! generate parametric CAD. This is the walking skeleton: upload real parts,
//! see them in a 3D viewport, and run the training pipeline end to end.

mod api;
mod freecad;
mod model;
mod ops;
mod pipeline;
mod score;
mod solid;
mod step;
mod stl;
mod vec3;

use model::{Db, Run};
use std::path::PathBuf;
use std::sync::Mutex;
use tokio::sync::broadcast;

/// Shared application state.
pub struct App {
    /// Where uploads and the db live.
    pub root: PathBuf,
    pub db: Mutex<Db>,
    /// Fan-out channel for live run updates (the SSE endpoint).
    pub bus: broadcast::Sender<Run>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use tracing_subscriber::EnvFilter;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let opts = Options::from_args();
    std::fs::create_dir_all(&opts.data).ok();

    let db = Db::load(&opts.data);
    let (bus, _) = broadcast::channel(512);
    let app = std::sync::Arc::new(App {
        root: opts.data.clone(),
        db: Mutex::new(db),
        bus,
    });

    let static_dir = opts.static_dir;
    if !static_dir.is_dir() {
        tracing::warn!("static dir {} does not exist", static_dir.display());
    }
    let index = static_dir.join("index.html");
    let files = tower_http::services::ServeDir::new(&static_dir)
        .not_found_service(tower_http::services::ServeFile::new(&index));

    let router = api::router(app)
        .fallback_service(files)
        .layer(tower_http::trace::TraceLayer::new_for_http());

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], opts.port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("cadforge listening on http://{addr}");
    tracing::info!("data dir {}", opts.data.display());
    axum::serve(listener, router).await?;
    Ok(())
}

struct Options {
    port: u16,
    data: PathBuf,
    static_dir: PathBuf,
}

impl Options {
    fn from_args() -> Self {
        let mut port = std::env::var("CADFORGE_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8787);
        let mut data = std::env::var("CADFORGE_DATA").unwrap_or_else(|_| "data".into());
        let mut static_dir = std::env::var("CADFORGE_STATIC").unwrap_or_else(|_| "static".into());

        let args: Vec<String> = std::env::args().skip(1).collect();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--port" | "-p" => {
                    if let Some(v) = args.get(i + 1).and_then(|v| v.parse().ok()) {
                        port = v;
                    }
                    i += 2;
                }
                "--data" | "-d" => {
                    if let Some(v) = args.get(i + 1) {
                        data = v.clone();
                    }
                    i += 2;
                }
                "--static" | "-s" => {
                    if let Some(v) = args.get(i + 1) {
                        static_dir = v.clone();
                    }
                    i += 2;
                }
                "--help" | "-h" => {
                    println!("cadforge [--port 8787] [--data data] [--static static]");
                    std::process::exit(0);
                }
                _ => i += 1,
            }
        }
        Self {
            port,
            data: data.into(),
            static_dir: static_dir.into(),
        }
    }
}
