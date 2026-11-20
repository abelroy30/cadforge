//! cadforge — a CAD-shaped web studio for training small models that
//! generate parametric CAD. This is the walking skeleton: upload real parts,
//! see them in a 3D viewport, and run the training pipeline end to end.

mod api;
mod dataset;
mod freecad;
mod ingest;
mod model;
mod ole;
mod ops;
mod pipeline;
mod propset;
mod reward;
mod score;
mod sldworks;
mod solid;
mod step;
mod stl;
mod swhist;
mod system;
mod vec3;
mod vision;

use model::{Asset, Db, Run};
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
    /// Fan-out channel for live artifact updates: every artifact the moment it
    /// has been processed, so the studio's list fills in as uploads land.
    pub asset_bus: broadcast::Sender<Asset>,
    /// Fan-out channel for ingest progress: folders being walked and files being
    /// read, so the interface shows the work rather than only its outcome.
    pub work_bus: broadcast::Sender<crate::ingest::WorkEvent>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use tracing_subscriber::EnvFilter;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let opts = Options::from_args();

    // `--emit-samples <dir>` writes a set of documents and stops; it needs no
    // server, and is how a new user gets something to load.
    if let Some(dir) = &opts.emit_samples {
        match emit_samples(dir) {
            Ok(n) => {
                println!("wrote {n} sample documents to {}", dir.display());
                println!("upload them all in the studio to see the pipeline work");
            }
            Err(e) => {
                eprintln!("could not write samples: {e}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    std::fs::create_dir_all(&opts.data).ok();

    let mut db = Db::load(&opts.data);
    // The reward a run is paid with is data, seeded visibly on first load so it
    // can be read, argued with and edited - not compiled in where nobody can see
    // what the model is actually being paid for.
    let had_rewards = !db.rewards.is_empty();
    crate::reward::ensure_defaults(&mut db);
    if !had_rewards {
        if let Err(e) = db.save(&opts.data) {
            tracing::warn!("could not save the default reward set: {e}");
        }
    }
    let (bus, _) = broadcast::channel(512);
    let (asset_bus, _) = broadcast::channel(512);
    let (work_bus, _) = broadcast::channel(1024);
    let app = std::sync::Arc::new(App {
        root: opts.data.clone(),
        db: Mutex::new(db),
        bus,
        asset_bus,
        work_bus,
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
    let url = format!("http://localhost:{}", opts.port);
    tracing::info!("cadforge listening on {url}");
    tracing::info!("data dir {}", opts.data.display());
    open_browser_soon(url);
    axum::serve(listener, router).await?;
    Ok(())
}

/// Open the studio in the default browser, so a double-click on the launcher is
/// all it takes. A few hundred milliseconds' grace lets the listener settle; if
/// the browser is already open it simply becomes a new tab. `CADFORGE_NO_OPEN=1`
/// turns it off, which is what a headless or scripted run wants.
fn open_browser_soon(url: String) {
    if std::env::var("CADFORGE_NO_OPEN").is_ok() {
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(450));
        let result = if cfg!(target_os = "windows") {
            // `start` is a cmd builtin, so it needs a shell; the empty title
            // keeps it from treating the URL as the window title.
            std::process::Command::new("cmd")
                .args(["/C", "start", "", &url])
                .spawn()
        } else if cfg!(target_os = "macos") {
            std::process::Command::new("open").arg(&url).spawn()
        } else {
            std::process::Command::new("xdg-open").arg(&url).spawn()
        };
        if let Err(e) = result {
            tracing::info!("could not open a browser ({e}); open {url} yourself");
        }
    });
}

struct Options {
    port: u16,
    data: PathBuf,
    static_dir: PathBuf,
    emit_samples: Option<PathBuf>,
}

impl Options {
    fn from_args() -> Self {
        let mut port = std::env::var("CADFORGE_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8787);
        let mut data = std::env::var("CADFORGE_DATA").unwrap_or_else(|_| "data".into());
        let mut static_dir = std::env::var("CADFORGE_STATIC").unwrap_or_else(|_| "static".into());
        let mut emit_samples: Option<PathBuf> = None;

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
                "--emit-samples" | "--samples" => {
                    if let Some(v) = args.get(i + 1) {
                        emit_samples = Some(v.into());
                    }
                    i += 2;
                }
                "--help" | "-h" => {
                    println!(
                        "cadforge [--port 8787] [--data data] [--static static]\n         \
                         cadforge --emit-samples samples   write sample documents and exit"
                    );
                    std::process::exit(0);
                }
                _ => i += 1,
            }
        }
        Self {
            port,
            data: data.into(),
            static_dir: static_dir.into(),
            emit_samples,
        }
    }
}

/// A tiny 24-bit BMP, so a sample document carries a preview the browser draws.
fn sample_preview(rgb: [u8; 3], size: u32) -> Vec<u8> {
    let data = size * 3 * size;
    let mut b: Vec<u8> = Vec::new();
    b.extend_from_slice(b"BM");
    b.extend_from_slice(&(54 + data).to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&54u32.to_le_bytes());
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&(size as i32).to_le_bytes());
    b.extend_from_slice(&(size as i32).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&24u16.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&data.to_le_bytes());
    b.extend_from_slice(&2835i32.to_le_bytes());
    b.extend_from_slice(&2835i32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    for y in 0..size {
        for x in 0..size {
            let shade = ((x + y) * 150 / (2 * size)) as u8;
            b.push(rgb[2].saturating_add(shade)); // BMP stores blue first
            b.push(rgb[1].saturating_add(shade / 2));
            b.push(rgb[0].saturating_add(shade));
        }
    }
    b
}

/// Write a set of sample documents - two parts, an assembly and a drawing -
/// each a real OLE2 compound file carrying parameters, a preview, and, for the
/// assembly, the parts it is made of. The studio reads them exactly as it reads
/// a genuine SolidWorks file, so the whole pipeline can be seen working without
/// a licence.
fn emit_samples(dir: &std::path::Path) -> std::io::Result<usize> {
    use ole::fixture as ole_fixture;
    use propset::fixture as pset;

    std::fs::create_dir_all(dir)?;
    let contents: Vec<u8> = (0..40_000u32).map(|i| (i % 251) as u8).collect();

    let bracket = ole_fixture::build(&[
        ("Preview", &sample_preview([120, 160, 210], 48)),
        ("ThirdPv", b"\x02\x00Extrude1\x00Sketch1\x00Fillet1\x00"),
        ("Contents", &contents),
        (
            "\u{5}SummaryInformation",
            &pset::build(&[&[(2, "bracket"), (4, "A. Machinist"), (18, "SolidWorks 2024")]]),
        ),
        (
            "\u{5}DocumentSummaryInformation",
            &pset::build(&[&[(15, "Acme Engineering"), (14, "J. Foreman")]]),
        ),
    ]);
    std::fs::write(dir.join("bracket.SLDPRT"), bracket)?;

    let plate = ole_fixture::build(&[
        ("Preview", &sample_preview([210, 150, 120], 48)),
        ("Contents", &contents),
        (
            "\u{5}SummaryInformation",
            &pset::build(&[&[(2, "plate"), (4, "A. Machinist")]]),
        ),
    ]);
    std::fs::write(dir.join("plate.SLDPRT"), plate)?;

    let rig = ole_fixture::build(&[
        ("Preview", &sample_preview([150, 200, 160], 48)),
        (
            "Contents",
            b"C:\\share\\plate.SLDPRT\x00C:\\share\\bracket.SLDPRT\x00",
        ),
        (
            "\u{5}SummaryInformation",
            &pset::build(&[&[(2, "rig"), (4, "A. Machinist")]]),
        ),
    ]);
    std::fs::write(dir.join("rig.SLDASM"), rig)?;

    let sheet = ole_fixture::build(&[
        ("Preview", &sample_preview([200, 200, 205], 48)),
        ("Contents", b"C:\\share\\bracket.SLDPRT\x00"),
        (
            "\u{5}SummaryInformation",
            &pset::build(&[&[(2, "bracket sheet"), (4, "A. Machinist")]]),
        ),
    ]);
    std::fs::write(dir.join("sheet.SLDDRW"), sheet)?;

    Ok(4)
}
