//! `csr run`: startup checks, background loops and the dashboard server.

use crate::app::AppState;
use crate::config::Config;
use crate::db::{Db, Sample};
use crate::identity::Identity;
use crate::node::{self, rpc::RpcClient, Polled};
use crate::resources::Monitor;
use crate::{logs, ping, web};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicI64;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, watch};
use tokio::time::{interval_at, Instant as TokioInstant, MissedTickBehavior};

const R2_INTERVAL: Duration = Duration::from_secs(10);
const SAMPLE_INTERVAL: Duration = Duration::from_secs(60);
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(2);
const RESOURCE_INTERVAL: Duration = Duration::from_secs(5);
const SWEEP_INTERVAL: Duration = Duration::from_secs(60);
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

/// The automatic sweep of the runner wallet (`payout.sweep_*`).
#[derive(Default)]
struct Sweeper {
    /// When the last sweep check ran (the first one runs as soon as the node is reachable).
    checked: Option<Instant>,
    /// The sweep transaction not committed yet.
    pending: Option<String>,
    warned: bool,
}

impl Sweeper {
    async fn step(&mut self, state: &Arc<AppState>, cfg: &Config) {
        let due = self
            .checked
            .is_none_or(|t| t.elapsed() >= period(SWEEP_INTERVAL));
        if self.pending.is_none() && (!due || cfg.payout.sweep_address().is_none()) {
            return;
        }
        let chain = match crate::chain::Chain::connect(cfg, &state.service_pubkey).await {
            Ok(c) => c.with_service_key(state.service_key),
            Err(e) => {
                if !self.warned {
                    tracing::warn!("runner wallet sweep unavailable: {e}");
                    self.warned = true;
                }
                return;
            }
        };
        if let Some(tx) = &self.pending {
            match chain.tx_status(tx).await {
                Ok((s, _)) if s == "committed" => {
                    tracing::info!("runner wallet: sweep {tx} committed");
                    self.pending = None;
                }
                Ok((s, _)) if s == "pending" || s == "proposed" => return,
                Ok((s, _)) => {
                    tracing::warn!("runner wallet: sweep {tx} was not committed ({s})");
                    self.pending = None;
                }
                Err(_) => return,
            }
        }
        if !due {
            return;
        }
        // One runner transaction at a time: not while keeper transactions are pending.
        let busy = state
            .db
            .runs_in(&[crate::keeper::SUBMITTING])
            .map(|r| !r.is_empty())
            .unwrap_or(true);
        if busy {
            return;
        }
        self.checked = Some(Instant::now());
        let _wallet = state.wallet_tx.lock().await;
        match crate::wallet::sweep(&chain, cfg, &state.net_dir).await {
            Ok(v) if v["swept"] == true => {
                self.pending = v["tx_hash"].as_str().map(str::to_string);
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("runner wallet sweep failed: {e}"),
        }
    }
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
        service_key: identity.service_key,
        storage_key: identity.storage_key,
        ping_nonces: Mutex::new(Default::default()),
        db,
        set_log_level,
        peer_id: RwLock::new(first.snap.node_id.clone()),
        node: RwLock::new(first),
        resources: RwLock::new(Some(monitor.sample())),
        last_check_ok_at: AtomicI64::new(0),
        config_write: Mutex::new(()),
        wallet_tx: Default::default(),
        monitor_cache: Default::default(),
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
    if !state.config.read().unwrap().payout.lock_args.is_empty() {
        tracing::warn!(
            "payout.lock_args is set but ignored: since A8 the tips go into the runner wallet \
             (see `csr wallet status`); remove it from the config"
        );
    }

    // Storage format version 2: convert old entries before anything reads or deletes them.
    {
        let root = crate::service::storage_root(&state.config.read().unwrap(), &data_root);
        let key = state.storage_key;
        let n = crate::storage::migrate_v1(&root, &key)
            .map_err(|e| format!("storage migration failed: {e}"))?;
        tracing::info!("storage: migrated {n} version 1 entries");
    }

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

    // Keeper: on every new block discover jobs (runtime on), run a batch and track settlements.
    // The same task sweeps the runner wallet (every 60 s, independent of the runtime), so that the
    // runner's own transactions go out one at a time.
    if let Ok(n) = state.db.requeue_interrupted() {
        if n > 0 {
            tracing::info!("{n} interrupted job run(s) queued again");
        }
    }
    {
        let state = state.clone();
        let mut stop = stop_rx.clone();
        tokio::spawn(async move {
            let p = period(DISCOVERY_INTERVAL);
            let mut tick = interval_at(TokioInstant::now() + Duration::from_millis(500), p);
            tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
            let mut keeper: Option<crate::keeper::Keeper> = None;
            let mut last_tip = None;
            let mut warned = false;
            let mut sweeper = Sweeper::default();
            loop {
                tokio::select! {
                    _ = tick.tick() => {},
                    _ = stop.changed() => return,
                }
                let cfg = state.config.read().unwrap().clone();
                sweeper.step(&state, &cfg).await;
                // Never start keeper transactions while a sweep is pending: both spend the wallet.
                if sweeper.pending.is_some() {
                    continue;
                }
                let r2 = state.node.read().unwrap().report.state;
                let (rt, reason) = crate::app::runtime_state(cfg.runtime.enabled, &cfg.network, r2);
                let submitting = state
                    .db
                    .runs_in(&[crate::keeper::SUBMITTING])
                    .map(|r| !r.is_empty())
                    .unwrap_or(false);
                // Off / mainnet-gated: nothing runs; only transactions already submitted are followed.
                if (rt == "off" || rt == "inactive") && !submitting {
                    continue;
                }
                if keeper.is_none() {
                    match crate::chain::Chain::connect(&cfg, &state.service_pubkey).await {
                        Ok(c) => {
                            keeper = Some(crate::keeper::Keeper::new(
                                c.with_service_key(state.service_key),
                            ))
                        }
                        Err(e) => {
                            if !warned {
                                tracing::warn!("job discovery unavailable: {e}");
                                warned = true;
                            }
                            continue;
                        }
                    }
                }
                let k = keeper.as_mut().expect("connected");
                let chain_tip = match k.tip().await {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::warn!("job discovery failed: {e}");
                        continue;
                    }
                };
                if last_tip == Some(chain_tip) {
                    continue;
                }
                let hold = if rt == "on" {
                    let res = state.resources.read().unwrap().clone();
                    res.and_then(|r| crate::resources::hold_reason(&cfg.limits, &r, r2))
                } else {
                    reason
                };
                // Errors are retried at the next block.
                last_tip = Some(chain_tip);
                let _wallet = state.wallet_tx.lock().await;
                match k
                    .tick(&state, &cfg, rt != "off" && rt != "inactive", hold)
                    .await
                {
                    Ok(summary) => tracing::debug!("{summary}"),
                    Err(e) => tracing::warn!("job scheduler: {e}"),
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

    // Ping port: only when ranking is joined and the mode includes inbound.
    let ping_server = {
        let (join, mode, pbind) = {
            let c = state.config.read().unwrap();
            (
                c.ranking.join,
                c.ranking.ping_mode.clone(),
                c.ranking.ping_bind.clone(),
            )
        };
        if join && (mode == "inbound" || mode == "both") {
            let l = tokio::net::TcpListener::bind(&pbind)
                .await
                .map_err(|e| format!("cannot listen on {pbind}: {e}"))?;
            tracing::info!("ping responder listening on {pbind}");
            let app = ping::inbound::router(ping::inbound::Responder::new(state.clone()))
                .into_make_service_with_connect_info::<std::net::SocketAddr>();
            let mut st = stop_rx.clone();
            Some(tokio::spawn(async move {
                let _ = axum::serve(l, app)
                    .with_graceful_shutdown(async move {
                        let _ = st.changed().await;
                    })
                    .await;
            }))
        } else {
            None
        }
    };

    tokio::spawn(ping::outbound::run(state.clone(), stop_rx.clone()));

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
    if let Some(p) = ping_server {
        let _ = tokio::time::timeout(Duration::from_secs(3), p).await;
    }
    result
}
