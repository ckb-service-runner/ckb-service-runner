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
const RENEW_INTERVAL: Duration = Duration::from_secs(30 * 60);
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

pub fn period(normal: Duration) -> Duration {
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
    /// The block at which the pending sweep was sent.
    sent_block: u64,
    warned: bool,
    /// The "no sweep address" warning was checked for this start.
    sweep_warned: bool,
}

const SWEEP_KEY: &str = "sweep_pending";

impl Sweeper {
    /// Loads a sweep that was pending when the runner stopped (`"<hash>:<block>"`).
    fn load(db: &Db) -> Sweeper {
        let mut s = Sweeper::default();
        if let Ok(Some(v)) = db.get_setting(SWEEP_KEY) {
            if let Some((h, b)) = v.rsplit_once(':') {
                if let Ok(b) = b.parse::<u64>() {
                    s.pending = Some(h.to_string());
                    s.sent_block = b;
                }
            }
        }
        s
    }

    /// Ends the pending sweep (committed, failed or given up).
    fn clear(&mut self, db: &Db) {
        self.pending = None;
        if let Err(e) = db.put_setting(SWEEP_KEY, "") {
            tracing::warn!("cannot clear the pending sweep: {e}");
        }
    }

    async fn step(&mut self, state: &Arc<AppState>, cfg: &Config) {
        let due = self
            .checked
            .is_none_or(|t| t.elapsed() >= period(SWEEP_INTERVAL));
        if !self.sweep_warned && cfg.payout.sweep_address().is_none() {
            if let Ok(c) = crate::chain::Chain::connect(cfg, &state.service_pubkey).await {
                if let Ok(cap) = c.wallet_capacity().await {
                    self.sweep_warned = true;
                    if let Some(w) = crate::wallet::sweep_warning(
                        cfg.payout.sweep_address(),
                        cap,
                        cfg.payout.sweep_threshold_ckb,
                    ) {
                        tracing::warn!("{w}");
                    }
                }
            }
        }
        if self.pending.is_none() && (!due || cfg.payout.sweep_address().is_none()) {
            return;
        }
        let chain = match crate::chain::Chain::connect_wallet(cfg, &state.service_pubkey).await {
            Ok(c) => c.with_service_key(state.service_key),
            Err(e) => {
                if !self.warned {
                    tracing::warn!("runner wallet sweep unavailable: {e}");
                    self.warned = true;
                }
                return;
            }
        };
        if let Some(tx) = self.pending.clone() {
            let tip = chain.tip_info().await.ok().map(|t| t.number);
            match chain.tx_status(&tx).await {
                Ok((s, _)) if s == "committed" => {
                    tracing::info!("runner wallet: sweep {tx} committed");
                    self.clear(&state.db);
                }
                Ok((s, _)) if s == "pending" || s == "proposed" => {
                    if tip.is_some_and(|t| crate::keeper::sweep_given_up(self.sent_block, t)) {
                        tracing::warn!("runner wallet: sweep {tx} is still {s}, giving it up");
                        self.clear(&state.db);
                    } else {
                        return;
                    }
                }
                Ok((s, _)) => {
                    tracing::warn!("runner wallet: sweep {tx} was not committed ({s})");
                    self.clear(&state.db);
                }
                Err(_) => {
                    if tip.is_some_and(|t| crate::keeper::sweep_given_up(self.sent_block, t)) {
                        tracing::warn!("runner wallet: sweep {tx} cannot be followed, giving it up");
                        self.clear(&state.db);
                    } else {
                        return;
                    }
                }
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
                if let Some(h) = &self.pending {
                    self.sent_block = chain.tip_info().await.map(|t| t.number).unwrap_or(0);
                    let value = format!("{h}:{}", self.sent_block);
                    if let Err(e) = state.db.put_setting(SWEEP_KEY, &value) {
                        tracing::warn!("cannot store the pending sweep: {e}");
                    }
                }
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("runner wallet sweep failed: {e}"),
        }
    }
}

/// One check of the `delete_on_consume` rules: out points reported `dead` by the node are consumed;
/// `live`, `unknown`, RPC errors and missing methods keep the rules.
async fn watch_consumed(client: &RpcClient, root: &Path, key: &[u8; 32]) -> Result<(), String> {
    let ops = crate::storage::rule_outpoints(root, key)?;
    if ops.is_empty() {
        return Ok(());
    }
    let mut answers = std::collections::HashMap::new();
    for op in ops {
        let params = serde_json::json!([
            {"tx_hash": format!("0x{}", hex::encode(&op[..32])),
             "index": format!("0x{:x}", u32::from_le_bytes(op[32..].try_into().unwrap()))},
            false
        ]);
        let answer = match client.call("get_live_cell", params).await {
            Ok(r) => Ok(r["status"] != "dead"),
            Err(_) => Err("get_live_cell failed".to_string()),
        };
        answers.insert(op, answer);
    }
    let mut log = Vec::new();
    let res = crate::storage::apply_consumed(
        root,
        key,
        &mut |o: &[u8; 36]| answers.get(o).cloned().unwrap_or(Ok(true)),
        &mut log,
    );
    if !log.is_empty() {
        for line in String::from_utf8_lossy(&log).lines() {
            tracing::info!("{line}");
        }
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join("deletions.log"))
        {
            let _ = f.write_all(&log);
        }
    }
    res.map(|_| ())
}

/// The background renewal: every 30 minutes, two days before the expiry.
#[derive(Default)]
struct Renewer {
    checked: Option<Instant>,
}

impl Renewer {
    async fn step(&mut self, state: &AppState, cfg: &Config) {
        crate::renew::follow_pending(state, cfg).await;
        let every = period(RENEW_INTERVAL);
        if self.checked.is_some_and(|t| t.elapsed() < every) {
            return;
        }
        let r2 = state.node.read().unwrap().report.state;
        let (rt, _) = crate::app::runtime_state(cfg.runtime.enabled, &cfg.network, r2);
        if rt == "off" || crate::wallet::busy_reason(&state.db).is_some() {
            return;
        }
        let chain = match crate::chain::Chain::connect(cfg, &state.service_pubkey).await {
            Ok(c) => c.with_service_key(state.service_key),
            Err(_) => return,
        };
        let due = match chain.renewal_state().await {
            Ok(Some((true, expiry, clock, _))) => crate::chain::renewal_due(expiry, clock),
            Ok(_) => false,
            Err(e) => {
                tracing::warn!("attestation renewal check failed: {e}");
                self.checked = Some(Instant::now());
                return;
            }
        };
        self.checked = Some(Instant::now());
        if !due {
            return;
        }
        let _wallet = state.wallet_tx.lock().await;
        match crate::renew::renew(&chain, cfg, &state.service_key, &state.service_pubkey, &state.db).await {
            Ok(v) => {
                let _ = state.db.put_setting(crate::renew::RENEWAL_ERROR, "");
                tracing::info!(
                    "attestation renewed in {}, new expiry {} ms",
                    v["tx_hash"],
                    v["expiry_ms"]
                );
            }
            Err(e) => {
                tracing::warn!("attestation renewal failed, trying again in 30 minutes: {e}");
                let _ = state.db.put_setting(crate::renew::RENEWAL_ERROR, &e);
            }
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
        sessions: Default::default(),
        login_limiter: Default::default(),
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
    // The background tasks; if one ends while no shutdown was requested, the process ends too.
    let mut tasks: tokio::task::JoinSet<()> = tokio::task::JoinSet::new();
    let mut names: std::collections::HashMap<tokio::task::Id, &'static str> = Default::default();

    // R2 guard.
    {
        let state = state.clone();
        let mut stop = stop_rx.clone();
        let network = state.config.read().unwrap().network.clone();
        let h = tasks.spawn(async move {
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
        names.insert(h.id(), "R2 guard");
    }

    // Resource refresh (every 5 s) and metric samples into SQLite.
    {
        let state = state.clone();
        let mut stop = stop_rx.clone();
        let h = tasks.spawn(async move {
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
                        disk_free_gb: Some(r.disk.free_gb),
                    };
                    if let Err(e) = state.db.add_sample(&sample) {
                        tracing::warn!("storing metric sample failed: {e}");
                    }
                }
            }
        });
        names.insert(h.id(), "resource sampler");
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
        let h = tasks.spawn(async move {
            let p = period(DISCOVERY_INTERVAL);
            let mut tick = interval_at(TokioInstant::now() + Duration::from_millis(500), p);
            tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
            let mut keeper: Option<crate::keeper::Keeper> = None;
            let mut last_tip = None;
            let mut warned = false;
            let mut sweeper = Sweeper::load(&state.db);
            let mut last_prune: Option<Instant> = None;
            let mut renewer = Renewer::default();
            loop {
                tokio::select! {
                    _ = tick.tick() => {},
                    _ = stop.changed() => return,
                }
                if last_prune.is_none_or(|t| t.elapsed() >= Duration::from_secs(3600)) {
                    last_prune = Some(Instant::now());
                    match state.db.prune_runs(now(), 30 * 86400) {
                        Ok(n) => tracing::debug!("pruned {n} old job run(s)"),
                        Err(e) => tracing::warn!("cannot prune old job runs: {e}"),
                    }
                }
                let cfg = state.config.read().unwrap().clone();
                sweeper.step(&state, &cfg).await;
                renewer.step(&state, &cfg).await;
                // Never start keeper transactions while a sweep is pending: both spend the wallet.
                // Submitted transactions are still followed.
                // The same holds for a pending attestation refresh.
                if sweeper.pending.is_some() || crate::wallet::refresh_pending(&state.db) {
                    if let Some(k) = keeper.as_mut() {
                        if let Err(e) = k.follow(&state).await {
                            tracing::warn!("job tracking: {e}");
                        }
                    }
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
        names.insert(h.id(), "keeper loop");
    }

    // Consumed-cell watch: on every new block check the out points of all delete_on_consume rules
    // (all services, also disabled ones, independent of the runtime) and apply the consumed ones.
    {
        let state = state.clone();
        let mut stop = stop_rx.clone();
        tokio::spawn(async move {
            let p = period(DISCOVERY_INTERVAL);
            let mut tick = interval_at(TokioInstant::now() + Duration::from_millis(750), p);
            tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
            let mut last_tip: Option<u64> = None;
            loop {
                tokio::select! {
                    _ = tick.tick() => {},
                    _ = stop.changed() => return,
                }
                let cfg = state.config.read().unwrap().clone();
                let client = RpcClient::new(&cfg.node.rpc_url);
                let tip = match client.call("get_tip_block_number", serde_json::json!([])).await {
                    Ok(v) => match node::rpc::hex_u64(&v) {
                        Some(t) => t,
                        None => continue,
                    },
                    Err(_) => continue,
                };
                if last_tip == Some(tip) {
                    continue;
                }
                last_tip = Some(tip);
                let root = state.net_dir.join(&cfg.storage.path);
                if let Err(e) = watch_consumed(&client, &root, &state.storage_key).await {
                    tracing::warn!("delete_on_consume check failed: {e}");
                }
            }
        });
    }

    let app = web::router(state.clone());
    let mut srv_stop = stop_rx.clone();
    let h = tasks.spawn(async move {
        if let Err(e) = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
            .with_graceful_shutdown(async move {
                let _ = srv_stop.changed().await;
            })
            .await
        {
            tracing::error!("dashboard server failed: {e}");
        }
    });
    names.insert(h.id(), "dashboard server");

    // Ping port: only when ranking is joined and the mode includes inbound.
    {
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
            let h = tasks.spawn(async move {
                let _ = axum::serve(l, app)
                    .with_graceful_shutdown(async move {
                        let _ = st.changed().await;
                    })
                    .await;
            });
            names.insert(h.id(), "ping responder");
        }
    }

    let h = tasks.spawn(ping::outbound::run(state.clone(), stop_rx.clone()));
    names.insert(h.id(), "outbound check-in");

    let result = tokio::select! {
        biased;
        _ = shutdown_signal() => {
            tracing::info!("shutting down");
            Ok(())
        }
        Some(e) = fatal_rx.recv() => Err(e),
        Some(ended) = tasks.join_next_with_id() => {
            let id = match &ended {
                Ok((id, _)) => *id,
                Err(e) => e.id(),
            };
            let name = names.get(&id).copied().unwrap_or("a background task");
            let how = match ended {
                Ok(_) => "ended".to_string(),
                Err(e) => format!("failed: {e}"),
            };
            tracing::error!("{name} {how} without a shutdown request, exiting");
            Err(format!("{name} {how}"))
        }
    };
    let _ = stop_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(3), async {
        while tasks.join_next().await.is_some() {}
    })
    .await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_sweep_survives_a_restart() {
        let db = Db::open_memory();
        assert!(Sweeper::load(&db).pending.is_none());
        db.put_setting(SWEEP_KEY, "0xabc:1234").unwrap();
        let mut s = Sweeper::load(&db);
        assert_eq!(s.pending.as_deref(), Some("0xabc"));
        assert_eq!(s.sent_block, 1234);
        s.clear(&db);
        assert!(s.pending.is_none());
        assert!(Sweeper::load(&db).pending.is_none());
    }

    #[test]
    fn garbage_sweep_setting_is_ignored() {
        let db = Db::open_memory();
        db.put_setting(SWEEP_KEY, "nonsense").unwrap();
        assert!(Sweeper::load(&db).pending.is_none());
    }
}
