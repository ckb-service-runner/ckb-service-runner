//! `csr run`: startup checks, background loops and the dashboard server.

use crate::app::AppState;
use crate::config::Config;
use crate::db::{Db, Sample};
use crate::identity::Identity;
use crate::node::{self, rpc::RpcClient, Polled};
use crate::resources::Monitor;
use crate::{logs, web};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicI64;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, watch};
use tokio::time::{interval_at, Instant as TokioInstant, MissedTickBehavior};

const R2_INTERVAL: Duration = Duration::from_secs(10);
const SAMPLE_INTERVAL: Duration = Duration::from_secs(60);
const RESOURCE_INTERVAL: Duration = Duration::from_secs(5);
const MAINNET_MARKER: &str = ".mainnet-confirmed";

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Debug builds only: `CSR_TEST_FAST_TICK=1` speeds the loops up for tests.
#[cfg(debug_assertions)]
pub fn fast_tick() -> bool {
    std::env::var("CSR_TEST_FAST_TICK").as_deref() == Ok("1")
}
#[cfg(not(debug_assertions))]
pub fn fast_tick() -> bool {
    false
}

fn period(normal: Duration) -> Duration {
    if fast_tick() {
        Duration::from_secs(1)
    } else {
        normal
    }
}

async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
    let mut int = signal(SignalKind::interrupt()).expect("SIGINT handler");
    tokio::select! {
        _ = term.recv() => {},
        _ = int.recv() => {},
    }
}

fn write_marker(path: &Path) -> Result<(), String> {
    std::fs::write(path, format!("confirmed {}\n", now()))
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

pub async fn run(
    cfg: Config,
    config_path: PathBuf,
    data_root: PathBuf,
    confirm_mainnet: bool,
) -> Result<(), String> {
    cfg.validate()?;
    let net_dir = data_root.join(&cfg.network);
    let marker = net_dir.join(MAINNET_MARKER);
    if cfg.network == "mainnet" && !confirm_mainnet && !marker.exists() {
        return Err(
            "mainnet requires explicit confirmation: start once with `csr run --confirm-mainnet`"
                .into(),
        );
    }

    // First contact with the node happens before anything is written to disk.
    let client = RpcClient::new(&cfg.node.rpc_url);
    let first = node::poll(&client, &cfg.network).await;
    let mut contacted = node::first_contact(&first.snap, &cfg.network)?;

    std::fs::create_dir_all(&net_dir)
        .map_err(|e| format!("cannot create {}: {e}", net_dir.display()))?;
    if cfg.network == "mainnet" && !marker.exists() {
        write_marker(&marker)?;
    }
    let set_log_level = logs::init(&net_dir.join("logs").join("csr.log"), &cfg.logging.level)?;
    let identity = Identity::load_or_create(&net_dir)?;
    let db = Db::open(&net_dir.join("csr.db"))?;
    let monitor = Arc::new(Monitor::new(data_root.clone()));
    let bind = cfg.dashboard.bind.clone();
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .map_err(|e| format!("cannot listen on {bind}: {e}"))?;

    let state = Arc::new(AppState {
        config_path,
        net_dir,
        started: Instant::now(),
        service_pubkey: identity.service_pubkey.clone(),
        db,
        set_log_level,
        peer_id: RwLock::new(first.snap.node_id.clone()),
        node: RwLock::new(first),
        resources: RwLock::new(Some(monitor.sample())),
        last_check_ok_at: AtomicI64::new(0),
        config_write: Mutex::new(()),
        sessions: Mutex::new(Default::default()),
        config: RwLock::new(cfg),
    });
    tracing::info!(
        "csr {} started on {} ({}), service key {}",
        env!("CARGO_PKG_VERSION"),
        bind,
        state.config.read().unwrap().network,
        state.service_pubkey
    );

    let (stop_tx, stop_rx) = watch::channel(false);
    let (fatal_tx, mut fatal_rx) = mpsc::channel::<String>(1);

    // R2 guard.
    {
        let state = state.clone();
        let mut stop = stop_rx.clone();
        let network = state.config.read().unwrap().network.clone();
        tokio::spawn(async move {
            let p = period(R2_INTERVAL);
            let mut tick = interval_at(TokioInstant::now() + p, p);
            tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = tick.tick() => {},
                    _ = stop.changed() => return,
                }
                let polled: Polled = node::poll(&client, &network).await;
                if !contacted {
                    match node::first_contact(&polled.snap, &network) {
                        Ok(c) => contacted = c,
                        Err(e) => {
                            let _ = fatal_tx.send(e).await;
                            return;
                        }
                    }
                }
                if let Some(id) = &polled.snap.node_id {
                    *state.peer_id.write().unwrap() = Some(id.clone());
                }
                let old = state.node.read().unwrap().report.state;
                if old != polled.report.state {
                    tracing::info!(
                        "R2 state {} -> {} {:?}",
                        old.as_str(),
                        polled.report.state.as_str(),
                        polled.report.reasons
                    );
                }
                *state.node.write().unwrap() = polled;
            }
        });
    }

    // Resource refresh (every 5 s) and metric samples into SQLite.
    {
        let state = state.clone();
        let mut stop = stop_rx.clone();
        tokio::spawn(async move {
            let refresh = if fast_tick() {
                Duration::from_secs(1)
            } else {
                RESOURCE_INTERVAL
            };
            let every = period(SAMPLE_INTERVAL);
            let mut tick = interval_at(TokioInstant::now() + Duration::from_millis(500), refresh);
            tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
            let mut last_saved: Option<Instant> = None;
            loop {
                tokio::select! {
                    _ = tick.tick() => {},
                    _ = stop.changed() => return,
                }
                let m = monitor.clone();
                let Ok(r) = tokio::task::spawn_blocking(move || m.sample()).await else {
                    continue;
                };
                *state.resources.write().unwrap() = Some(r.clone());
                if last_saved.map(|t| t.elapsed() >= every).unwrap_or(true) {
                    last_saved = Some(Instant::now());
                    let tip_lag = state.node.read().unwrap().report.tip_lag;
                    let sample = Sample {
                        t: now(),
                        cpu_percent: r.cpu.total_percent,
                        ram_free_mb: r.ram.free_mb,
                        tip_lag,
                    };
                    if let Err(e) = state.db.add_sample(&sample) {
                        tracing::warn!("storing metric sample failed: {e}");
                    }
                }
            }
        });
    }

    let app = web::router(state.clone());
    let mut srv_stop = stop_rx.clone();
    let mut server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = srv_stop.changed().await;
            })
            .await
    });

    let result = tokio::select! {
        _ = shutdown_signal() => {
            tracing::info!("shutting down");
            Ok(())
        }
        Some(e) = fatal_rx.recv() => Err(e),
        r = &mut server => Err(format!("dashboard server stopped: {r:?}")),
    };
    let _ = stop_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(3), server).await;
    result
}
