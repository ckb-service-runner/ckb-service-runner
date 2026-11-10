//! Tests for `csr-service-manifest`, the type script of the service manifest cell (`spec/service-manifest.md`).
use ckb_testtool::bytes::Bytes;
use ckb_testtool::ckb_error::Error as CkbError;
use ckb_testtool::ckb_hash::blake2b_256;
use ckb_testtool::ckb_types::packed::OutPoint;
use ckb_testtool::ckb_types::prelude::*;
use csr_contract_tests::*;

const FEE: u64 = 100_000;
const WALLET: u64 = 5000 * CKB;

// error codes of the contract
const E_TYPE_ID: i8 = 6;
const E_VERSION_TAG: i8 = 9;
const E_CAP: i8 = 10;
const E_UTF8: i8 = 11;
const E_URL: i8 = 12;
const E_DATA_CELLS: i8 = 13;
const E_CODE_DEP: i8 = 15;
const E_PUBLISHER: i8 = 16;
const E_DATA_DEP: i8 = 17;
const E_FIXED: i8 = 18;
const E_LOCK: i8 = 19;

fn assert_script_error(res: Result<u64, CkbError>, script: &str, code: i8) {
    let err = res.expect_err("transaction must be rejected");
    let s = format!("{err:?}");
    assert!(s.contains(script), "not rejected by {script}: {s}");
    assert!(s.contains(&format!("error code {code} ")), "expected error code {code}: {s}");
}

/// A manifest, serialised by hand so that tests can produce invalid encodings too.
#[derive(Clone)]
struct M {
    version_tag: u8,
    code_hash: [u8; 32],
    code_cell: [u8; 36],
    data_hashes: Vec<[u8; 32]>,
    data_cells: Vec<[u8; 36]>,
    args: Vec<u8>,
    name: Vec<u8>,
    version: Vec<u8>,
    description: Vec<u8>,
    url: Vec<u8>,
    source: Vec<u8>,
    language: Vec<u8>,
    min_csr: Vec<u8>,
    deprecated: u8,
}

fn mol_vec(items: &[&[u8]]) -> Vec<u8> {
    let mut v = (items.len() as u32).to_le_bytes().to_vec();
    for i in items {
        v.extend_from_slice(i);
    }
    v
}

impl M {
    fn new(code_hash: [u8; 32], code_cell: &OutPoint) -> M {
        M {
            version_tag: 1,
            code_hash,
            code_cell: code_cell.as_slice().try_into().unwrap(),
            data_hashes: vec![],
            data_cells: vec![],
            args: vec![],
            name: b"Price keeper".to_vec(),
            version: b"1.0.0".to_vec(),
            description: b"Posts the price once per hour.".to_vec(),
            url: b"https://example.org/keeper".to_vec(),
            source: b"https://github.com/example/keeper".to_vec(),
            language: b"rust".to_vec(),
            min_csr: b"0.6.0".to_vec(),
            deprecated: 0,
        }
    }

    fn data(&self) -> Bytes {
        let dh: Vec<&[u8]> = self.data_hashes.iter().map(|h| &h[..]).collect();
        let dc: Vec<&[u8]> = self.data_cells.iter().map(|h| &h[..]).collect();
        let fields: Vec<Vec<u8>> = vec![
            vec![self.version_tag],
            self.code_hash.to_vec(),
            self.code_cell.to_vec(),
            mol_vec(&dh),
            mol_vec(&dc),
            mol_bytes(&self.args).as_slice().to_vec(),
            mol_bytes(&self.name).as_slice().to_vec(),
            mol_bytes(&self.version).as_slice().to_vec(),
            mol_bytes(&self.description).as_slice().to_vec(),
            mol_bytes(&self.url).as_slice().to_vec(),
            mol_bytes(&self.source).as_slice().to_vec(),
            mol_bytes(&self.language).as_slice().to_vec(),
            mol_bytes(&self.min_csr).as_slice().to_vec(),
            vec![self.deprecated],
        ];
        let header = 4 * (fields.len() + 1);
        let total = header + fields.iter().map(Vec::len).sum::<usize>();
        let mut out = (total as u32).to_le_bytes().to_vec();
        let mut off = header;
        for f in &fields {
            out.extend_from_slice(&(off as u32).to_le_bytes());
            off += f.len();
        }
        for f in &fields {
            out.extend_from_slice(f);
        }
        out.into()
    }
}

struct S {
    env: Env,
    publisher: Key,
    code: Vec<u8>,
}

fn setup() -> S {
    let mut env = Env::new();
    let publisher = env.new_key();
    S { env, publisher, code: b"\x7fELF fake service binary".to_vec() }
}

impl S {
    /// A code cell locked by `lock_of` (default: the publisher) holding `data`.
    fn dep_cell(&mut self, data: &[u8], owner_other: bool) -> OutPoint {
        let lock = if owner_other { self.env.new_key().lock } else { self.publisher.lock.clone() };
        self.env.min_cell(0, lock, None, Bytes::copy_from_slice(data))
    }

    fn base(&mut self) -> M {
        let code = self.code.clone();
        let op = self.dep_cell(&code, false);
        M::new(blake2b_256(&code), &op)
    }

    /// Creation with the given manifest data; `deps` are the cell deps (the code cell by default).
    fn create_with(&mut self, data: Bytes, deps: &[OutPoint], id: Option<[u8; 32]>) -> Result<u64, CkbError> {
        let wallet = self.env.cell(WALLET, self.publisher.lock.clone(), None, Bytes::new());
        let input = self.env.input(&wallet, 0);
        let id = id.unwrap_or_else(|| type_id(&input, 0));
        let t = self.env.manifest_type_script(&id);
        let cap = min_capacity(&self.publisher.lock, Some(&t), data.len());
        let outputs = vec![
            (output(cap, self.publisher.lock.clone(), Some(t)), data),
            (output(WALLET - cap - FEE, self.publisher.lock.clone(), None), Bytes::new()),
        ];
        let mut tx = tx_with(vec![input], outputs).as_advanced_builder();
        for d in deps {
            tx = tx.cell_dep(cell_dep(d));
        }
        let publisher = &self.publisher;
        self.env.verify_signed(tx.build(), &[publisher])
    }

    fn create(&mut self, m: &M) -> Result<u64, CkbError> {
        let op = OutPoint::from_slice(&m.code_cell).unwrap();
        self.create_with(m.data(), &[op], None)
    }

    /// A live manifest cell holding `m`.
    fn live(&mut self, m: &M) -> (OutPoint, ckb_testtool::ckb_types::packed::Script, u64) {
        let id = self.env.fresh_id();
        let t = self.env.manifest_type_script(&id);
        let data = m.data();
        let cap = min_capacity(&self.publisher.lock, Some(&t), 600) + CKB;
        let op = self.env.cell(cap, self.publisher.lock.clone(), Some(t.clone()), data);
        (op, t, cap)
    }

    /// Update: the live manifest `old` in, `outs` copies of `new` out (lock `lock` of the outputs).
    fn update(&mut self, old: &M, new: &M, outs: usize, other_lock: bool) -> Result<u64, CkbError> {
        let (op, t, cap) = self.live(old);
        let lock = if other_lock { self.env.new_key().lock } else { self.publisher.lock.clone() };
        let outputs: Vec<_> = (0..outs).map(|_| (output(cap - FEE, lock.clone(), Some(t.clone())), new.data())).collect();
        let tx = tx_with(vec![self.env.input(&op, 0)], outputs);
        let publisher = &self.publisher;
        self.env.verify_signed(tx, &[publisher])
    }
}

#[test]
fn manifest_create_ok() {
    let mut s = setup();
    let m = s.base();
    let cycles = s.create(&m).expect("create");
    println!("manifest create: {cycles} cycles (whole transaction incl. system lock)");
    // empty optional text fields, maximal caps
    let mut m = s.base();
    m.url.clear();
    m.source.clear();
    m.language.clear();
    m.min_csr.clear();
    m.description = vec![b'a'; 512];
    m.name = vec![b'n'; 48];
    m.version = vec![b'1'; 16];
    m.args = vec![7u8; 64];
    s.create(&m).expect("create with caps");
}

#[test]
fn manifest_create_with_data_cells_ok() {
    let mut s = setup();
    let mut m = s.base();
    let mut deps = vec![OutPoint::from_slice(&m.code_cell).unwrap()];
    for i in 0..4u8 {
        let d = vec![i; 40 + i as usize];
        let op = s.dep_cell(&d, i % 2 == 1); // data cells may belong to anyone
        m.data_hashes.push(blake2b_256(&d));
        m.data_cells.push(op.as_slice().try_into().unwrap());
        deps.push(op);
    }
    s.create_with(m.data(), &deps, None).expect("create with 4 data cells");
}

#[test]
fn manifest_create_without_code_dep_rejected() {
    let mut s = setup();
    let m = s.base();
    assert_script_error(s.create_with(m.data(), &[], None), "Outputs[0].Type", E_CODE_DEP);
}

#[test]
fn manifest_create_wrong_code_hash_rejected() {
    let mut s = setup();
    let mut m = s.base();
    m.code_hash = [1u8; 32];
    assert_script_error(s.create(&m), "Outputs[0].Type", E_CODE_DEP);
}

#[test]
fn manifest_create_other_publisher_rejected() {
    let mut s = setup();
    let code = s.code.clone();
    let op = s.dep_cell(&code, true); // the code cell belongs to somebody else
    let m = M::new(blake2b_256(&code), &op);
    assert_script_error(s.create(&m), "Outputs[0].Type", E_PUBLISHER);
}

#[test]
fn manifest_create_wrong_data_hash_rejected() {
    let mut s = setup();
    let mut m = s.base();
    let op = s.dep_cell(b"data file", false);
    m.data_hashes.push([2u8; 32]);
    m.data_cells.push(op.as_slice().try_into().unwrap());
    let deps = [OutPoint::from_slice(&m.code_cell).unwrap(), op.clone()];
    assert_script_error(s.create_with(m.data(), &deps, None), "Outputs[0].Type", E_DATA_DEP);
    // the data cell is not a cell dep at all
    m.data_hashes[0] = blake2b_256(b"data file");
    let deps = [OutPoint::from_slice(&m.code_cell).unwrap()];
    assert_script_error(s.create_with(m.data(), &deps, None), "Outputs[0].Type", E_DATA_DEP);
}

#[test]
fn manifest_create_bad_version_tag_rejected() {
    let mut s = setup();
    let mut m = s.base();
    m.version_tag = 2;
    assert_script_error(s.create(&m), "Outputs[0].Type", E_VERSION_TAG);
}

#[test]
fn manifest_create_name_too_long_rejected() {
    let mut s = setup();
    let mut m = s.base();
    m.name = vec![b'n'; 49];
    assert_script_error(s.create(&m), "Outputs[0].Type", E_CAP);
    let mut m = s.base();
    m.name.clear();
    assert_script_error(s.create(&m), "Outputs[0].Type", E_CAP);
}

#[test]
fn manifest_create_description_too_long_rejected() {
    let mut s = setup();
    let mut m = s.base();
    m.description = vec![b'd'; 513];
    assert_script_error(s.create(&m), "Outputs[0].Type", E_CAP);
}

#[test]
fn manifest_create_http_url_rejected() {
    let mut s = setup();
    let mut m = s.base();
    m.url = b"http://example.org".to_vec();
    assert_script_error(s.create(&m), "Outputs[0].Type", E_URL);
    let mut m = s.base();
    m.source = b"javascript:alert(1)".to_vec();
    assert_script_error(s.create(&m), "Outputs[0].Type", E_URL);
}

#[test]
fn manifest_create_invalid_utf8_rejected() {
    let mut s = setup();
    let mut m = s.base();
    m.description = vec![b'a', 0xff, 0xfe];
    assert_script_error(s.create(&m), "Outputs[0].Type", E_UTF8);
}

#[test]
fn manifest_create_too_many_data_cells_rejected() {
    let mut s = setup();
    let mut m = s.base();
    let mut deps = vec![OutPoint::from_slice(&m.code_cell).unwrap()];
    for i in 0..5u8 {
        let d = vec![i; 40];
        let op = s.dep_cell(&d, false);
        m.data_hashes.push(blake2b_256(&d));
        m.data_cells.push(op.as_slice().try_into().unwrap());
        deps.push(op);
    }
    assert_script_error(s.create_with(m.data(), &deps, None), "Outputs[0].Type", E_DATA_CELLS);
    // lengths differ
    let mut m = s.base();
    m.data_hashes.push([1u8; 32]);
    assert_script_error(s.create(&m), "Outputs[0].Type", E_DATA_CELLS);
}

#[test]
fn manifest_create_bad_type_id_rejected() {
    let mut s = setup();
    let m = s.base();
    let op = OutPoint::from_slice(&m.code_cell).unwrap();
    assert_script_error(s.create_with(m.data(), &[op], Some([9u8; 32])), "Outputs[0].Type", E_TYPE_ID);
}

#[test]
fn manifest_update_text_ok() {
    let mut s = setup();
    let old = s.base();
    let mut new = old.clone();
    new.name = b"Price keeper pro".to_vec();
    new.version = b"1.1.0".to_vec();
    new.description = b"Better.".to_vec();
    new.url = b"https://example.org/v2".to_vec();
    new.source = vec![];
    new.language = b"c".to_vec();
    new.min_csr = b"0.7.0".to_vec();
    s.update(&old, &new, 1, false).expect("update text");
}

#[test]
fn manifest_update_deprecate_ok() {
    let mut s = setup();
    let old = s.base();
    let mut new = old.clone();
    new.deprecated = 1;
    s.update(&old, &new, 1, false).expect("deprecate");
}

#[test]
fn manifest_update_code_hash_rejected() {
    let mut s = setup();
    let old = s.base();
    let mut new = old.clone();
    new.code_hash = [5u8; 32];
    assert_script_error(s.update(&old, &new, 1, false), "Inputs[0].Type", E_FIXED);
    let mut new = old.clone();
    new.code_cell[0] ^= 1;
    assert_script_error(s.update(&old, &new, 1, false), "Inputs[0].Type", E_FIXED);
    let mut new = old.clone();
    new.data_hashes.push([1u8; 32]);
    new.data_cells.push([1u8; 36]);
    assert_script_error(s.update(&old, &new, 1, false), "Inputs[0].Type", E_FIXED);
}

#[test]
fn manifest_update_args_rejected() {
    let mut s = setup();
    let old = s.base();
    let mut new = old.clone();
    new.args = vec![1, 2, 3];
    assert_script_error(s.update(&old, &new, 1, false), "Inputs[0].Type", E_FIXED);
}

#[test]
fn manifest_update_lock_rejected() {
    let mut s = setup();
    let old = s.base();
    assert_script_error(s.update(&old, &old.clone(), 1, true), "Inputs[0].Type", E_LOCK);
}

#[test]
fn manifest_two_outputs_rejected() {
    let mut s = setup();
    let old = s.base();
    let mut new = old.clone();
    new.version = b"1.0.1".to_vec();
    let err = s.update(&old, &new, 2, false).expect_err("two outputs");
    assert!(format!("{err:?}").contains("Type"), "{err:?}");
    // two manifests created in one transaction
    let m = s.base();
    let op = OutPoint::from_slice(&m.code_cell).unwrap();
    let wallet = s.env.cell(WALLET, s.publisher.lock.clone(), None, Bytes::new());
    let input = s.env.input(&wallet, 0);
    let t = s.env.manifest_type_script(&type_id(&input, 0));
    let cap = min_capacity(&s.publisher.lock, Some(&t), m.data().len());
    let outs = vec![
        (output(cap, s.publisher.lock.clone(), Some(t.clone())), m.data()),
        (output(cap, s.publisher.lock.clone(), Some(t)), m.data()),
    ];
    let tx = tx_with(vec![input], outs).as_advanced_builder().cell_dep(cell_dep(&op)).build();
    let publisher = &s.publisher;
    let err = s.env.verify_signed(tx, &[publisher]).expect_err("two outputs in one group");
    assert!(format!("{err:?}").contains("Type"), "{err:?}");
}

#[test]
fn manifest_destroy_ok() {
    let mut s = setup();
    let m = s.base();
    let (op, _t, cap) = s.live(&m);
    let tx = tx_with(vec![s.env.input(&op, 0)], vec![(output(cap - FEE, s.publisher.lock.clone(), None), Bytes::new())]);
    let publisher = &s.publisher;
    s.env.verify_signed(tx, &[publisher]).expect("destroy");
}
