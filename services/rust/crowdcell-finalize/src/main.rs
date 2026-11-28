#![no_std]
#![no_main]

use alloc::vec::Vec;
use crowdcell_finalize_logic::{build_skeleton, decide, finalized_data, parse_campaign, parse_payload, Outcome};

ckb_std::entry!(program_entry);
ckb_std::default_alloc!();

const EXIT_BAD_PAYLOAD: i8 = 3;
const EXIT_HOST: i8 = 4;
const EXIT_BAD_CAMPAIGN: i8 = 5;

// Result: [code, new status]. 0 finalized, 1 already final, 2 deadline not reached, 3 no live campaign cell.
const R_FINALIZED: u8 = 0;
const R_ALREADY_FINAL: u8 = 1;
const R_NOT_YET: u8 = 2;
const R_NO_CELL: u8 = 3;

// An empty skeleton (never call tx_skeleton_put) makes the run settle with only its tip.
fn nothing_to_do(code: u8) -> i8 {
    csr_svc::result_put(&[code, 0]);
    0
}

/// Finalizes one CrowdCell campaign after its deadline; idempotent. See spec/crowdcell-finalize.md.
fn program_entry() -> i8 {
    let mut pbuf = alloc::vec![0u8; 4 + 4096 + 72];
    let (r, len) = csr_svc::job_payload(&mut pbuf);
    if r != 0 || len as usize > pbuf.len() {
        return EXIT_BAD_PAYLOAD;
    }
    let payload = match parse_payload(&pbuf[..len as usize]) {
        Ok(p) => p,
        Err(_) => return EXIT_BAD_PAYLOAD,
    };

    // The live campaign cell: the campaign type script (full args) is the prefix to match.
    let mut ops = [0u8; 4 + 64 * 36];
    let (r, n) = csr_svc::cells_query(&payload.type_script, 1, &mut ops);
    if r != 0 || n as usize > ops.len() {
        return EXIT_HOST;
    }
    let count = u32::from_le_bytes([ops[0], ops[1], ops[2], ops[3]]) as usize;
    if count == 0 {
        return nothing_to_do(R_NO_CELL);
    }
    if count > 1 {
        return EXIT_HOST; // a TypeID script has one live cell; more means the payload script is too short
    }
    let mut out_point = [0u8; 36];
    out_point.copy_from_slice(&ops[4..40]);

    let mut output = alloc::vec![0u8; 2048];
    let (r, n) = csr_svc::chain_read(0, &out_point, &mut output);
    if r == 1 {
        return nothing_to_do(R_NO_CELL); // spent between the query and the read
    }
    if r != 0 || n as usize > output.len() {
        return EXIT_HOST;
    }
    output.truncate(n as usize);
    let mut data = alloc::vec![0u8; 4096];
    let (r, n) = csr_svc::chain_read(1, &out_point, &mut data);
    if r == 1 {
        return nothing_to_do(R_NO_CELL);
    }
    if r != 0 || n as usize > data.len() {
        return EXIT_BAD_CAMPAIGN;
    }
    data.truncate(n as usize);

    let campaign = match parse_campaign(&data) {
        Some(c) => c,
        None => return EXIT_BAD_CAMPAIGN,
    };
    let mut info = [0u8; 24];
    if csr_svc::block_info(&mut info) != 0 {
        return EXIT_HOST;
    }
    let tip = u64::from_le_bytes([info[0], info[1], info[2], info[3], info[4], info[5], info[6], info[7]]);

    match decide(&campaign, tip) {
        Outcome::AlreadyFinal => nothing_to_do(R_ALREADY_FINAL),
        Outcome::NotYet => nothing_to_do(R_NOT_YET),
        Outcome::Malformed => EXIT_BAD_CAMPAIGN,
        Outcome::Finalize { new_status } => {
            let new_data = finalized_data(&data, new_status);
            let skeleton: Vec<u8> = match build_skeleton(&payload, &out_point, &output, &new_data, campaign.deadline) {
                Ok(s) => s,
                Err(_) => return EXIT_BAD_CAMPAIGN,
            };
            if csr_svc::tx_skeleton_put(&skeleton) != 0 {
                return EXIT_HOST;
            }
            csr_svc::result_put(&[R_FINALIZED, new_status]);
            0
        }
    }
}
