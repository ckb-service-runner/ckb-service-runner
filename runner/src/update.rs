//! Update check, verified staging and the verification the root installer runs (D5 B).
//!
//! The runner asks GitHub for the latest release, downloads the binary for this machine together
//! with `SHA256SUMS` and `SHA256SUMS.sig`, checks the ed25519 signature against `RELEASE_PUBKEY` and
//! the binary's hash, and stages it in `<net dir>/update/`. A root systemd path unit
//! (`config/csr-update.path`) runs `scripts/install-update.sh`, which lets the installed binary
//! check the staged files again (`csr update verify`) before it installs them.

use ed25519_dalek::{Signature, VerifyingKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::{Duration, Instant};

/// Public ed25519 key of the release signer (created offline, Oct 2026; the private key never enters
/// the repository). All zero = not configured: every update is refused.
pub const RELEASE_PUBKEY: [u8; 32] = [
    0x1c, 0x46, 0x34, 0xc3, 0x9c, 0x12, 0xfc, 0x1c,
    0x0c, 0x3d, 0xd2, 0x27, 0x3e, 0x76, 0xd7, 0xc2,
    0xa6, 0x5c, 0x4f, 0x0a, 0x95, 0xe6, 0x1f, 0x23,
    0xe9, 0x68, 0x5e, 0x62, 0x35, 0x1a, 0xa2, 0x82,
];

pub const REPO: &str = "ckb-service-runner/ckb-service-runner";
const LATEST_URL: &str =
    "https://api.github.com/repos/ckb-service-runner/ckb-service-runner/releases/latest";
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_RELEASE_JSON: usize = 1 << 20;
const MAX_ASSET: usize = 64 << 20;
const MAX_REDIRECTS: usize = 5;
pub const CACHE_TTL: Duration = Duration::from_secs(3600);
/// Cache of a check asked for by hand: short, but repeated clicks do not hit GitHub's rate limit.
pub const MANUAL_TTL: Duration = Duration::from_secs(60);
pub const BACKGROUND_INTERVAL: Duration = Duration::from_secs(24 * 3600);
const NO_KEY: &str = "no release key configured: updates are refused by this build";

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub fn key_configured() -> bool {
    RELEASE_PUBKEY != [0u8; 32]
}

/// Parses `X.Y.Z` with an optional leading `v`; anything else (pre-releases, names) is `None`.
fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.strip_prefix('v').unwrap_or(s);
    let mut it = s.split('.');
    let mut part = || -> Option<u64> {
        let p = it.next()?;
        if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        p.parse().ok()
    };
    let v = (part()?, part()?, part()?);
    if it.next().is_some() {
        return None;
    }
    Some(v)
}

/// Whether `latest_tag` is newer than `current`; `None` if either is not `X.Y.Z` ("v" optional).
pub fn newer(current: &str, latest_tag: &str) -> Option<bool> {
    Some(parse_version(latest_tag)? > parse_version(current)?)
}

/// The SHA-256 listed for `name` in a `SHA256SUMS` file (`<64 hex>  <name>` lines).
pub fn sha256_for(sums: &str, name: &str) -> Option<[u8; 32]> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        // `sha256sum` writes two spaces, or " *" in binary mode.
        let file = file.trim_start_matches(' ').trim_start_matches('*');
        if file.trim_end() != name || hash.len() != 64 {
            return None;
        }
        hex::decode(hash).ok()?.try_into().ok()
    })
}

/// The release asset of the binary: `csr-<X.Y.Z>-<arch>-linux`.
pub fn asset_name(tag: &str, arch: &str) -> String {
    format!("csr-{}-{arch}-linux", tag.strip_prefix('v').unwrap_or(tag))
}

/// Checks the ed25519 signature of `SHA256SUMS` (strict verification; weak keys are refused).
pub fn verify_release(sums: &[u8], sig: &[u8; 64], pubkey: &[u8; 32]) -> bool {
    let Ok(key) = VerifyingKey::from_bytes(pubkey) else {
        return false;
    };
    key.verify_strict(sums, &Signature::from_bytes(sig)).is_ok()
}

/// Checks the signature of `sums`, then that `binary` has the hash listed there for `name`.
pub fn check_staged(
    binary: &[u8],
    name: &str,
    sums: &[u8],
    sig: &[u8; 64],
    pubkey: &[u8; 32],
) -> Result<(), String> {
    if *pubkey == [0u8; 32] {
        return Err(NO_KEY.into());
    }
    if !verify_release(sums, sig, pubkey) {
        return Err("SHA256SUMS: the signature does not match the release key".into());
    }
    let sums = std::str::from_utf8(sums).map_err(|_| "SHA256SUMS is not UTF-8".to_string())?;
    let want = sha256_for(sums, name).ok_or_else(|| format!("SHA256SUMS lists no {name}"))?;
    let got: [u8; 32] = Sha256::digest(binary).into();
    if got != want {
        return Err(format!("{name}: SHA-256 does not match SHA256SUMS"));
    }
    Ok(())
}

/// Whether the update may talk to `url`: https on github.com, api.github.com or *.githubusercontent.com.
pub fn download_host_allowed(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else {
        return false;
    };
    if u.scheme() != "https" || u.port().is_some() || !u.username().is_empty() {
        return false;
    }
    match u.host() {
        Some(url::Host::Domain(h)) => {
            let h = h.to_ascii_lowercase();
            h == "github.com"
                || h == "api.github.com"
                || (h.ends_with(".githubusercontent.com")
                    && h.len() > ".githubusercontent.com".len())
        }
        _ => false,
    }
}

/// HTTP client of the update: no proxy, at most 5 redirects and only to GitHub hosts over https.
/// (`network::http_client` follows no redirects; GitHub release downloads need them.)
pub fn http_client(timeout: Duration) -> Result<reqwest::Client, String> {
    let policy = reqwest::redirect::Policy::custom(|a| {
        if a.previous().len() >= MAX_REDIRECTS {
            a.error("too many redirects")
        } else if download_host_allowed(a.url().as_str()) {
            a.follow()
        } else {
            let msg = format!("redirect to a non-GitHub URL refused: {}", a.url());
            a.error(msg)
        }
    });
    reqwest::Client::builder()
        .no_proxy()
        .timeout(timeout)
        .redirect(policy)
        .user_agent(format!("csr/{}", current_version()))
        .build()
        .map_err(|e| format!("http client: {e}"))
}

/// GETs `url` (which must pass `download_host_allowed`) and reads at most `max` bytes.
async fn fetch(http: &reqwest::Client, url: &str, max: usize) -> Result<Vec<u8>, String> {
    if !download_host_allowed(url) {
        return Err(format!("refusing to download from {url}"));
    }
    let mut r = http
        .get(url)
        .header(
            "Accept",
            "application/vnd.github+json, application/octet-stream",
        )
        .send()
        .await
        .map_err(|e| format!("{url}: {e}"))?;
    if !r.status().is_success() {
        return Err(format!("{url}: HTTP {}", r.status()));
    }
    if r.content_length().is_some_and(|n| n > max as u64) {
        return Err(format!("{url}: larger than {max} bytes"));
    }
    let mut body = Vec::new();
    while let Some(chunk) = r.chunk().await.map_err(|e| format!("{url}: {e}"))? {
        if body.len() + chunk.len() > max {
            return Err(format!("{url}: larger than {max} bytes"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// The latest release as GitHub reports it.
#[derive(Clone, Debug)]
pub struct Release {
    pub tag: String,
    pub notes_url: String,
    /// (name, browser_download_url)
    pub assets: Vec<(String, String)>,
}

async fn latest_release() -> Result<Release, String> {
    let http = http_client(CHECK_TIMEOUT)?;
    let body = fetch(&http, LATEST_URL, MAX_RELEASE_JSON)
        .await
        .map_err(|e| match e.ends_with("HTTP 404 Not Found") {
            true => format!("no release of {REPO} is published yet"),
            false => e,
        })?;
    let j: Value =
        serde_json::from_slice(&body).map_err(|e| format!("GitHub answer is not JSON: {e}"))?;
    let tag = j["tag_name"]
        .as_str()
        .ok_or("GitHub answer has no tag_name")?
        .to_string();
    // Shown as a link on the dashboard: only GitHub https URLs.
    let notes_url = j["html_url"]
        .as_str()
        .filter(|u| download_host_allowed(u))
        .map(String::from)
        .unwrap_or_else(|| format!("https://github.com/{REPO}/releases/tag/{tag}"));
    let assets = j["assets"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    Some((
                        x["name"].as_str()?.to_string(),
                        x["browser_download_url"].as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Release {
        tag,
        notes_url,
        assets,
    })
}

/// The answer of `GET /api/update` and `csr update check`.
pub fn check_json(rel: &Release, checked_at: i64) -> Value {
    json!({
        "current": current_version(),
        "latest": rel.tag.strip_prefix('v').unwrap_or(&rel.tag),
        "newer": newer(current_version(), &rel.tag),
        "notes_url": rel.notes_url,
        "checked_at": checked_at,
        "key_configured": key_configured(),
    })
}

/// The last check result (shared by the dashboard, the daily background check and staging).
#[derive(Default)]
pub struct Cache {
    last: std::sync::Mutex<Option<(Instant, i64, Release)>>,
    /// Serialises checks and staging.
    busy: tokio::sync::Mutex<()>,
}

impl Cache {
    /// The cached check result without contacting GitHub (for the Overview notice).
    pub fn cached_json(&self) -> Option<Value> {
        let last = self.last.lock().unwrap();
        last.as_ref().map(|(_, at, rel)| check_json(rel, *at))
    }

    async fn release(&self, max_age: Duration) -> Result<(Release, i64), String> {
        let _busy = self.busy.lock().await;
        self.release_locked(max_age).await
    }

    async fn release_locked(&self, max_age: Duration) -> Result<(Release, i64), String> {
        if let Some((t, at, rel)) = &*self.last.lock().unwrap() {
            if t.elapsed() < max_age {
                return Ok((rel.clone(), *at));
            }
        }
        let rel = latest_release().await?;
        let at = crate::run::now();
        *self.last.lock().unwrap() = Some((Instant::now(), at, rel.clone()));
        Ok((rel, at))
    }

    /// Checks for a newer release (cached for an hour).
    pub async fn check(&self) -> Result<Value, String> {
        let (rel, at) = self.release(CACHE_TTL).await?;
        Ok(check_json(&rel, at))
    }

    /// Like `check`, but with a cache of at most `MANUAL_TTL` (the dashboard's "Check for update").
    pub async fn check_fresh(&self) -> Result<Value, String> {
        let (rel, at) = self.release(MANUAL_TTL).await?;
        Ok(check_json(&rel, at))
    }

    /// Downloads, verifies and stages the latest release in `dir`.
    pub async fn stage(&self, dir: &Path) -> Result<Value, String> {
        if !key_configured() {
            return Err(NO_KEY.into());
        }
        let _busy = self
            .busy
            .try_lock()
            .map_err(|_| "an update check or download is already running".to_string())?;
        // Fresh answer: staging should not act on an hour-old release.
        let (rel, _) = self.release_locked(Duration::ZERO).await?;
        stage_release(&rel, dir).await
    }
}

/// Parses `SHA256SUMS.sig`: 64 raw bytes or 128 hex characters.
pub fn parse_sig(raw: &[u8]) -> Result<[u8; 64], String> {
    if raw.len() == 64 {
        return Ok(raw.try_into().unwrap());
    }
    let text = std::str::from_utf8(raw)
        .map_err(|_| "SHA256SUMS.sig: neither 64 bytes nor hex".to_string())?
        .trim();
    if text.len() != 128 {
        return Err("SHA256SUMS.sig: neither 64 bytes nor 128 hex characters".into());
    }
    let b = hex::decode(text).map_err(|_| "SHA256SUMS.sig: invalid hex".to_string())?;
    Ok(b.try_into().unwrap())
}

async fn stage_release(rel: &Release, dir: &Path) -> Result<Value, String> {
    match newer(current_version(), &rel.tag) {
        Some(true) => {}
        Some(false) => {
            return Err(format!(
                "no newer release: latest is {}, this is {}",
                rel.tag,
                current_version()
            ))
        }
        None => return Err(format!("latest release tag {:?} is not X.Y.Z", rel.tag)),
    }
    let version = rel.tag.strip_prefix('v').unwrap_or(&rel.tag).to_string();
    let asset = asset_name(&rel.tag, std::env::consts::ARCH);
    let url_of = |name: &str| {
        rel.assets
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, u)| u.clone())
            .ok_or_else(|| format!("release {} has no asset {name}", rel.tag))
    };
    let http = http_client(DOWNLOAD_TIMEOUT)?;
    let binary = fetch(&http, &url_of(&asset)?, MAX_ASSET).await?;
    let sums = fetch(&http, &url_of("SHA256SUMS")?, MAX_ASSET).await?;
    let sig_raw = fetch(&http, &url_of("SHA256SUMS.sig")?, MAX_ASSET).await?;
    let sig = parse_sig(&sig_raw)?;
    check_staged(&binary, &asset, &sums, &sig, &RELEASE_PUBKEY)?;
    write_staged(dir, &version, &asset, &binary, &sums, &sig_raw)?;
    tracing::info!("update {version} staged in {}", dir.display());
    Ok(json!({
        "staged": true,
        "version": version,
        "asset": asset,
        "dir": dir.display().to_string(),
        "message": format!(
            "Update {version} is verified and staged. The installer checks it again, installs it \
             and restarts the runner in a few seconds (needs csr-update.path, see docs/install.md)."
        ),
    }))
}

fn write_file(path: &Path, data: &[u8], mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let tmp = path.with_extension("part");
    let err = |e: std::io::Error| format!("cannot write {}: {e}", path.display());
    std::fs::write(&tmp, data).map_err(err)?;
    // Explicit mode, independent of the umask.
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode)).map_err(err)?;
    std::fs::rename(&tmp, path).map_err(err)
}

/// Writes the staged files; `ready.json` last, as it triggers the installer.
fn write_staged(
    dir: &Path,
    version: &str,
    asset: &str,
    binary: &[u8],
    sums: &[u8],
    sig: &[u8],
) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let _ = std::fs::remove_file(dir.join("ready.json"));
    write_file(&dir.join("csr.new"), binary, 0o755)?;
    write_file(&dir.join("SHA256SUMS"), sums, 0o644)?;
    write_file(&dir.join("SHA256SUMS.sig"), sig, 0o644)?;
    let ready = json!({"version": version, "asset": asset}).to_string();
    write_file(&dir.join("ready.json"), ready.as_bytes(), 0o644)
}

/// `csr update verify`: checks the staged files with this binary's key and version (no network).
pub fn verify_dir(dir: &Path) -> Result<Value, String> {
    let read =
        |name: &str| std::fs::read(dir.join(name)).map_err(|e| format!("cannot read {name}: {e}"));
    let ready: Value =
        serde_json::from_slice(&read("ready.json")?).map_err(|e| format!("ready.json: {e}"))?;
    let version = ready["version"].as_str().ok_or("ready.json: no version")?;
    let asset = ready["asset"].as_str().ok_or("ready.json: no asset")?;
    // The asset name carries the version, so the signed SHA256SUMS line binds the version too.
    let want = asset_name(version, std::env::consts::ARCH);
    if asset != want {
        return Err(format!("ready.json: asset {asset:?}, expected {want:?}"));
    }
    match newer(current_version(), version) {
        Some(true) => {}
        _ => {
            return Err(format!(
                "staged version {version:?} is not newer than {}",
                current_version()
            ))
        }
    }
    let sig = parse_sig(&read("SHA256SUMS.sig")?)?;
    check_staged(
        &read("csr.new")?,
        asset,
        &read("SHA256SUMS")?,
        &sig,
        &RELEASE_PUBKEY,
    )?;
    Ok(json!({"verified": true, "version": version, "asset": asset}))
}

/// Daily background check feeding the Overview notice.
pub async fn background(
    state: std::sync::Arc<crate::app::AppState>,
    mut stop: tokio::sync::watch::Receiver<bool>,
) {
    let mut wait = Duration::from_secs(60);
    loop {
        tokio::select! {
            _ = tokio::time::sleep(wait) => {},
            _ = stop.changed() => return,
        }
        wait = BACKGROUND_INTERVAL;
        match state.update.check().await {
            Ok(j) if j["newer"] == json!(true) => {
                tracing::info!("csr {} is available: {}", j["latest"], j["notes_url"])
            }
            Ok(_) => {}
            Err(e) => tracing::debug!("update check failed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sig_file_formats() {
        let raw = [7u8; 64];
        assert_eq!(parse_sig(&raw).unwrap(), raw);
        let hexed = format!("{}\n", hex::encode(raw));
        assert_eq!(parse_sig(hexed.as_bytes()).unwrap(), raw);
        assert!(parse_sig(b"short").is_err());
    }

    #[test]
    fn zero_key_refuses_everything() {
        assert!(check_staged(b"x", "x", b"", &[0; 64], &[0; 32])
            .unwrap_err()
            .contains("no release key"));
    }

    #[test]
    fn verify_dir_checks_version_and_asset() {
        let d = crate::test_tmp();
        let asset = asset_name("0.0.1", std::env::consts::ARCH);
        write_staged(d.path(), "0.0.1", &asset, b"bin", b"sums", &[0; 64]).unwrap();
        assert!(verify_dir(d.path()).unwrap_err().contains("not newer"));
        write_staged(
            d.path(),
            "99.0.0",
            "csr-1.0.0-x86_64-linux",
            b"bin",
            b"sums",
            &[0; 64],
        )
        .unwrap();
        assert!(verify_dir(d.path()).unwrap_err().contains("expected"));
        let asset = asset_name("99.0.0", std::env::consts::ARCH);
        write_staged(d.path(), "99.0.0", &asset, b"bin", b"sums", &[0; 64]).unwrap();
        let e = verify_dir(d.path()).unwrap_err();
        assert!(
            e.contains("no release key") || e.contains("signature does not match"),
            "{e}"
        );
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(d.path().join("csr.new"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
    }
}

#[cfg(test)]
#[path = "../../acceptance/aiworker-csr-update-check/update_tests.rs"]
mod aiworker_update_tests;
