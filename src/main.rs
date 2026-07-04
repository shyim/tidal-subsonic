use tidal_subsonic::{build_router, build_state, db};

#[tokio::main]
async fn main() {
    // Default to INFO for this crate (so request logs always show) plus WARN
    // elsewhere; RUST_LOG overrides everything when set.
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,tidal_subsonic=info,tower_http=warn"));
    fmt().with_env_filter(filter).init();

    // Open or create SQLite database
    let db_path = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("tidal-subsonic")
        .join("tidal-subsonic.db");

    let db = db::open_db(&db_path).expect("Failed to open database");
    let cfg = db::load_config(&db).await;

    // Encryption key for TIDAL tokens at rest, then fold any legacy single-user
    // setup into the multi-user schema.
    let cipher = db::master_cipher(&db).await;
    if let Err(e) = db::migrate_single_user(&db, &cipher).await {
        tracing::warn!("single-user migration: {}", e);
    }

    let host = cfg.server_host.clone();
    let port = cfg.server_port;

    let state = build_state(db.clone(), cipher, &cfg).await;
    let app = build_router(state);

    let addr = format!("{}:{}", host, port);
    tracing::info!("Starting tidal-subsonic server on {}", addr);

    let users = db::user_count(&db).await;
    tracing::info!(
        "{} registered user(s). Open http://{}:{}/ to manage TIDAL links.",
        users,
        host,
        port
    );

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
