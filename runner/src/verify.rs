//! "Verify binary" for the Services page: re-reads the cells of a service from the own node and recomputes its
//! service ID. Pure logic; the node reads happen in `verify_service`.
use crate::catalog;
use crate::db::Db;
use crate::node::rpc::RpcClient;
use ckb_hash::blake2b_256;
use serde_json::{json, Value};

/// What reading one cell from the node gave.
#[derive(Debug, Clone, PartialEq)]
pub enum CellRead {
    Live(Vec<u8>),
    Dead,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Match,
    Mismatch,
    Unavailable,
    NotRecorded,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Match => "match",
            Status::Mismatch => "mismatch",
            Status::Unavailable => "unavailable",
            Status::NotRecorded => "not_recorded",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub what: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub status: Status,
    pub service_id: String,
    pub computed_id: Option<String>,
    pub checks: Vec<Check>,
}

impl Report {
    pub fn to_json(&self) -> Value {
        json!({
            "status": self.status.as_str(),
            "ok": self.status == Status::Match,
            "service_id": self.service_id,
            "computed_id": self.computed_id,
            "checks": self.checks.iter().map(|c| json!({"what": c.what, "ok": c.ok, "detail": c.detail})).collect::<Vec<_>>(),
        })
    }
}

fn chk(what: &str, ok: bool, detail: impl Into<String>) -> Check {
    Check { what: what.into(), ok, detail: detail.into() }
}

fn early(status: Status, service_id: &str, checks: Vec<Check>) -> Report {
    Report { status, service_id: service_id.into(), computed_id: None, checks }
}

fn read_check(what: &str, cell: &str, r: &CellRead, checks: &mut Vec<Check>) -> Option<Vec<u8>> {
    match r {
        CellRead::Live(d) => {
            checks.push(chk(what, true, format!("{cell} is live, {} bytes", d.len())));
            Some(d.clone())
        }
        CellRead::Dead => {
            checks.push(chk(what, false, format!("{cell} is not live (spent or unknown)")));
            None
        }
        CellRead::Failed(e) => {
            checks.push(chk(what, false, format!("cannot read {cell}: {e}")));
            None
        }
    }
}

pub fn verify(
    service_id: &str,
    code_cell: Option<&str>,
    code: Option<&CellRead>,
    args_hex: &str,
    data_cells: &[String],
    data: &[CellRead],
) -> Report {
    let want: Option<[u8; 32]> = service_id
        .strip_prefix("0x")
        .and_then(|h| hex::decode(h).ok())
        .and_then(|b| b.try_into().ok());
    let Some(want) = want else {
        return early(Status::Unavailable, service_id, vec![chk("service ID", false, "the stored service ID is not 0x + 64 hex digits")]);
    };
    let (Some(code_cell), Some(code)) = (code_cell, code) else {
        return early(Status::NotRecorded, service_id, vec![]);
    };
    let Ok(args) = hex::decode(args_hex) else {
        return early(Status::Unavailable, service_id, vec![chk("args", false, "the stored args are not valid hex")]);
    };
    if data_cells.len() != data.len() {
        return early(Status::Unavailable, service_id, vec![chk("data cells", false, "data cells and reads differ in number")]);
    }
    let mut checks = Vec::new();
    let code_bytes = read_check("code cell", code_cell, code, &mut checks);
    let mut datas = Vec::new();
    for (c, r) in data_cells.iter().zip(data) {
        datas.push(read_check("data cell", c, r, &mut checks));
    }
    let Some(code_bytes) = code_bytes else {
        return early(Status::Unavailable, service_id, checks);
    };
    if datas.iter().any(|d| d.is_none()) {
        return early(Status::Unavailable, service_id, checks);
    }
    let code_hash = blake2b_256(&code_bytes);
    checks.push(chk("code hash", true, format!("0x{}", hex::encode(code_hash))));
    let hashes: Vec<[u8; 32]> = datas.iter().flatten().map(|d| blake2b_256(d)).collect();
    let got = catalog::service_id(&code_hash, &args, &hashes);
    let ok = got == want;
    let got_hex = format!("0x{}", hex::encode(got));
    checks.push(chk(
        "service ID",
        ok,
        if ok { format!("recomputed ID equals {got_hex}") } else { format!("recomputed {got_hex}, expected {service_id}") },
    ));
    Report {
        status: if ok { Status::Match } else { Status::Mismatch },
        service_id: service_id.into(),
        computed_id: Some(got_hex),
        checks,
    }
}

async fn read(rpc: &RpcClient, cell: &str) -> CellRead {
    match crate::registry::live_data(rpc, cell).await {
        Ok(Some(b)) => CellRead::Live(b),
        Ok(None) => CellRead::Dead,
        Err(e) => CellRead::Failed(e),
    }
}

/// Reads the cells of a service from the own node and verifies its ID. `Ok(None)`: unknown service.
pub async fn verify_service(db: &Db, rpc: &RpcClient, id: &str) -> Result<Option<Value>, String> {
    let id = id.to_ascii_lowercase();
    let Some(row) = db.service(&id)? else {
        return Ok(None);
    };
    let mut code = None;
    let mut datas = Vec::new();
    if let Some(cell) = row.outpoint.as_deref() {
        code = Some(read(rpc, cell).await);
        for c in &row.data_cells {
            datas.push(read(rpc, c).await);
        }
    }
    Ok(Some(
        verify(row.id(), row.outpoint.as_deref(), code.as_ref(), &row.args, &row.data_cells, &datas).to_json(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unknown_id_gives_none_and_no_code_cell_is_not_recorded() {
        let db = Db::open_memory();
        let rpc = RpcClient::new("http://127.0.0.1:1");
        assert!(verify_service(&db, &rpc, "0xdead").await.unwrap().is_none());
        let id = format!("0x{}", "ab".repeat(32));
        db.add_service(&id, "x", "onchain").unwrap();
        let v = verify_service(&db, &rpc, &id.to_uppercase().replace("0X", "0x")).await.unwrap().unwrap();
        assert_eq!(v["status"], "not_recorded");
    }
}

#[cfg(test)]
#[path = "../../acceptance/aiworker-csr-services-detail/verify_tests.rs"]
mod aiworker_sd_verify_tests;
