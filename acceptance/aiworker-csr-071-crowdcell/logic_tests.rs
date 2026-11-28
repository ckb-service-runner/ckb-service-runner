// csr-071-crowdcell, phase 1: the pure logic of the finalize service. Included into the library crate
// `crowdcell-finalize-logic` (see its lib.rs); `use super::*` sees the public API.
use super::*;

fn rd32(b: &[u8], at: usize) -> usize {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) as usize
}

// Fields of a molecule table (or dynvec): header = total size, then one offset per field.
fn fields(b: &[u8]) -> Vec<&[u8]> {
    assert_eq!(rd32(b, 0), b.len(), "declared size must equal the real size");
    if b.len() == 4 {
        return vec![];
    }
    let n = rd32(b, 4) / 4 - 1;
    let mut offs: Vec<usize> = (0..n).map(|i| rd32(b, 4 + 4 * i)).collect();
    offs.push(b.len());
    (0..n).map(|i| &b[offs[i]..offs[i + 1]]).collect()
}

// A fixvec of fixed-size items: u32 count, then the items.
fn fix_items(b: &[u8], size: usize) -> Vec<&[u8]> {
    let n = rd32(b, 0);
    assert_eq!(b.len(), 4 + n * size);
    (0..n).map(|i| &b[4 + i * size..4 + (i + 1) * size]).collect()
}

fn campaign_data(goal: u64, deadline: u64, total: u64, status: u8, tail: &[u8]) -> Vec<u8> {
    let mut d = vec![0u8; 65];
    d[0..32].copy_from_slice(&[7u8; 32]);
    d[32..40].copy_from_slice(&goal.to_le_bytes());
    d[40..48].copy_from_slice(&deadline.to_le_bytes());
    d[48..56].copy_from_slice(&total.to_le_bytes());
    d[56] = status;
    d.extend_from_slice(tail);
    d
}

fn payload_bytes(script: &[u8], a: [u8; 36], b: [u8; 36]) -> Vec<u8> {
    let mut v = (script.len() as u32).to_le_bytes().to_vec();
    v.extend_from_slice(script);
    v.extend_from_slice(&a);
    v.extend_from_slice(&b);
    v
}

fn out_point(tag: u8) -> [u8; 36] {
    let mut o = [tag; 36];
    o[32..36].copy_from_slice(&(tag as u32).to_le_bytes());
    o
}

// A molecule CellOutput with three fields (capacity, lock, type): only its shape matters here.
fn cell_output(capacity: u64) -> Vec<u8> {
    let lock = vec![1u8; 20];
    let typ = vec![2u8; 30];
    let header = 16usize;
    let total = header + 8 + lock.len() + typ.len();
    let mut v = (total as u32).to_le_bytes().to_vec();
    v.extend_from_slice(&(header as u32).to_le_bytes());
    v.extend_from_slice(&((header + 8) as u32).to_le_bytes());
    v.extend_from_slice(&((header + 8 + lock.len()) as u32).to_le_bytes());
    v.extend_from_slice(&capacity.to_le_bytes());
    v.extend_from_slice(&lock);
    v.extend_from_slice(&typ);
    v
}

#[test]
fn payload_roundtrip() {
    let script = vec![9u8; 85];
    let p = parse_payload(&payload_bytes(&script, out_point(1), out_point(2))).unwrap();
    assert_eq!(p.type_script, script);
    assert_eq!(p.campaign_code, out_point(1));
    assert_eq!(p.lock_code, out_point(2));
}

#[test]
fn payload_rejects_short_trailing_and_oversized_input() {
    assert_eq!(parse_payload(&[]), Err(PayloadError::TooShort));
    assert_eq!(parse_payload(&[0u8; 75]), Err(PayloadError::TooShort));
    let mut ok = payload_bytes(&[9u8; 10], out_point(1), out_point(2));
    ok.push(0);
    assert_eq!(parse_payload(&ok), Err(PayloadError::LengthMismatch));
    let mut short = payload_bytes(&[9u8; 10], out_point(1), out_point(2));
    short.pop();
    assert_eq!(parse_payload(&short), Err(PayloadError::LengthMismatch));
    let big = payload_bytes(&vec![9u8; MAX_SCRIPT_LEN + 1], out_point(1), out_point(2));
    assert_eq!(parse_payload(&big), Err(PayloadError::ScriptTooLarge));
    let edge = payload_bytes(&vec![9u8; MAX_SCRIPT_LEN], out_point(1), out_point(2));
    assert!(parse_payload(&edge).is_ok());
}

#[test]
fn campaign_fields_are_read_from_the_agreed_offsets() {
    let c = parse_campaign(&campaign_data(1000, 500, 750, 0, b"title")).unwrap();
    assert_eq!(c, Campaign { goal: 1000, deadline: 500, total: 750, status: 0 });
    assert!(parse_campaign(&[0u8; 64]).is_none());
    assert!(parse_campaign(&[]).is_none());
}

#[test]
fn success_exactly_at_the_goal_and_failed_one_below() {
    let at = Campaign { goal: 1000, deadline: 500, total: 1000, status: 0 };
    assert_eq!(decide(&at, 500), Outcome::Finalize { new_status: 1 });
    let over = Campaign { total: 1001, ..at };
    assert_eq!(decide(&over, 900), Outcome::Finalize { new_status: 1 });
    let under = Campaign { total: 999, ..at };
    assert_eq!(decide(&under, 500), Outcome::Finalize { new_status: 2 });
    let zero = Campaign { total: 0, ..at };
    assert_eq!(decide(&zero, 500), Outcome::Finalize { new_status: 2 });
}

#[test]
fn nothing_happens_before_the_deadline_or_after_finalization() {
    let c = Campaign { goal: 1000, deadline: 500, total: 1000, status: 0 };
    assert_eq!(decide(&c, 499), Outcome::NotYet);
    assert_eq!(decide(&c, 0), Outcome::NotYet);
    assert_eq!(decide(&Campaign { status: 1, ..c }, 9999), Outcome::AlreadyFinal);
    assert_eq!(decide(&Campaign { status: 2, ..c }, 9999), Outcome::AlreadyFinal);
    // before the deadline a finalized campaign is still just finalized
    assert_eq!(decide(&Campaign { status: 1, ..c }, 0), Outcome::AlreadyFinal);
}

#[test]
fn unknown_status_or_zero_deadline_is_malformed() {
    let c = Campaign { goal: 1000, deadline: 500, total: 1000, status: 0 };
    assert_eq!(decide(&Campaign { status: 3, ..c }, 600), Outcome::Malformed);
    assert_eq!(decide(&Campaign { status: 255, ..c }, 600), Outcome::Malformed);
    assert_eq!(decide(&Campaign { deadline: 0, ..c }, 600), Outcome::Malformed);
}

#[test]
fn finalized_data_changes_only_the_status_byte() {
    let old = campaign_data(1000, 500, 1000, 0, b"campaign title and metadata");
    for status in [1u8, 2u8] {
        let new = finalized_data(&old, status);
        assert_eq!(new.len(), old.len());
        assert_eq!(new[56], status);
        assert_eq!(&new[..56], &old[..56]);
        assert_eq!(&new[57..], &old[57..], "reserved bytes and the metadata tail stay identical");
    }
}

#[test]
fn skeleton_has_exactly_the_agreed_shape() {
    let script = vec![9u8; 85];
    let p = parse_payload(&payload_bytes(&script, out_point(1), out_point(2))).unwrap();
    let cell = out_point(5);
    let output = cell_output(123_456_789_000);
    let old = campaign_data(1000, 4242, 1000, 0, b"meta");
    let new = finalized_data(&old, 1);
    let tx = build_skeleton(&p, &cell, &output, &new, 4242).unwrap();

    let top = fields(&tx);
    assert_eq!(top.len(), 2, "Transaction = raw + witnesses");
    let raw = fields(top[0]);
    assert_eq!(raw.len(), 6, "RawTransaction has six fields");
    assert_eq!(raw[0], &0u32.to_le_bytes(), "version 0");

    // cell deps: the campaign code cell, then the campaign-lock code cell, both dep type code (0)
    let deps = fix_items(raw[1], 37);
    assert_eq!(deps.len(), 2);
    assert_eq!(&deps[0][..36], &out_point(1));
    assert_eq!(deps[0][36], 0);
    assert_eq!(&deps[1][..36], &out_point(2));
    assert_eq!(deps[1][36], 0);

    // no header deps
    assert_eq!(fix_items(raw[2], 32).len(), 0);

    // one input: since = the deadline as an absolute block number, then the campaign cell's out point
    let inputs = fix_items(raw[3], 44);
    assert_eq!(inputs.len(), 1);
    let since = u64::from_le_bytes(inputs[0][0..8].try_into().unwrap());
    assert_eq!(since, 4242);
    assert_eq!(since >> 56, 0, "no since flag bits: absolute block number");
    assert_eq!(&inputs[0][8..], &cell);

    // one output: byte for byte the cell's own output (capacity, lock and type unchanged)
    let outputs = fields(raw[4]);
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0], &output[..]);

    // one data entry: the finalized campaign data
    let data = fields(raw[5]);
    assert_eq!(data.len(), 1);
    assert_eq!(rd32(data[0], 0), new.len());
    assert_eq!(&data[0][4..], &new[..]);

    // one empty witness
    let wit = fields(top[1]);
    assert_eq!(wit.len(), 1);
    assert_eq!(wit[0], &0u32.to_le_bytes(), "an empty Bytes is just its zero length");
}

#[test]
fn skeleton_refuses_a_malformed_output_or_a_deadline_with_flag_bits() {
    let p = parse_payload(&payload_bytes(&[9u8; 10], out_point(1), out_point(2))).unwrap();
    let cell = out_point(5);
    let data = campaign_data(1, 2, 3, 1, b"");
    assert_eq!(build_skeleton(&p, &cell, &[], &data, 2), Err(BuildError::BadOutput));
    let mut wrong_size = cell_output(1);
    wrong_size.push(0);
    assert_eq!(build_skeleton(&p, &cell, &wrong_size, &data, 2), Err(BuildError::BadOutput));
    assert_eq!(build_skeleton(&p, &cell, &cell_output(1), &data, 1u64 << 56), Err(BuildError::BadDeadline));
    assert!(build_skeleton(&p, &cell, &cell_output(1), &data, (1u64 << 56) - 1).is_ok());
}

#[test]
fn decision_and_skeleton_fit_together() {
    // the whole happy path on one campaign: failed (999 < 1000) at the deadline block
    let old = campaign_data(1000, 700, 999, 0, b"x");
    let c = parse_campaign(&old).unwrap();
    let Outcome::Finalize { new_status } = decide(&c, 700) else { panic!("must finalize") };
    assert_eq!(new_status, 2);
    let p = parse_payload(&payload_bytes(&[9u8; 10], out_point(1), out_point(2))).unwrap();
    let tx = build_skeleton(&p, &out_point(5), &cell_output(10), &finalized_data(&old, new_status), c.deadline).unwrap();
    assert!(tx.len() > 100);
}
