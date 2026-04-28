//! APXM GUI binary entry-point.

use std::path::PathBuf;

use clap::Parser;
use tracing::info;

use apxm_gui::cli::CliArgs;
use apxm_gui::env;
use apxm_gui::routes::build_router;
use apxm_gui::state::AppState;

const DEFAULT_PORT: u16 = 18801;
const TRACING_DEFAULT_FILTER: &str = "apxm_gui=info,tower_http=info";

fn default_static_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("frontend-dist")
}

fn default_examples_dir() -> Option<PathBuf> {
    if let Ok(home) = std::env::var(env::keys::APXM_HOME) {
        let p = PathBuf::from(home).join("examples");
        if p.is_dir() {
            return Some(p);
        }
    }
    let local = PathBuf::from("examples");
    if local.is_dir() { Some(local) } else { None }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| TRACING_DEFAULT_FILTER.parse().unwrap()),
        )
        .init();

    let args = CliArgs::parse();

    let port = args.port.unwrap_or_else(|| env::gui_port(DEFAULT_PORT));
    let static_dir = args.static_dir.clone().unwrap_or_else(default_static_dir);
    let initial_file = args.initial_file();
    let examples_dir = args.examples_dir.clone().or_else(default_examples_dir);

    if let Some(ref f) = initial_file {
        info!("Initial file: {f}");
    }
    if let Some(ref d) = examples_dir {
        info!("Examples dir: {}", d.display());
    }

    let state = AppState::new(initial_file, examples_dir);
    let app = build_router(state, &static_dir);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    info!("APXM GUI listening on http://localhost:{port}");
    info!("Static files: {}", static_dir.display());

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind address");

    let shutdown = async {
        let ctrl_c = tokio::signal::ctrl_c();
        #[cfg(unix)]
        {
            let mut sigterm =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("failed to install SIGTERM handler");
            tokio::select! {
                _ = ctrl_c => {},
                _ = sigterm.recv() => {},
            }
        }
        #[cfg(not(unix))]
        ctrl_c.await.ok();

        info!("shutdown signal received");
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .expect("server error");
}
