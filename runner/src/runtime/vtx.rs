//! The virtual transaction presented to a service in local runs.
//!
//! One input (the virtual job cell) whose type script is the service script; the cell deps are the
//! program and its data cells; `witnesses[0]` is the raw job payload.

use blake2b_ref::Blake2bBuilder;

/// CKB data hash: blake2b-256 with personalization `ckb-default-hash`.
pub fn ckb_hash(data: &[u8]) -> [u8; 32] {
    let mut h = Blake2bBuilder::new(32)
        .personal(b"ckb-default-hash")
        .build();
    h.update(data);
    let mut out = [0u8; 32];
    h.finalize(&mut out);
    out
}

/// `hash_type = data2`.
pub const HASH_TYPE_DATA2: u8 = 4;

#[derive(Clone)]
pub struct Script {
    pub code_hash: [u8; 32],
    pub hash_type: u8,
    pub args: Vec<u8>,
}

impl Script {
    /// Molecule encoding of `Script`.
    pub fn to_molecule(&self) -> Vec<u8> {
        let mut args = Vec::with_capacity(4 + self.args.len());
        args.extend_from_slice(&(self.args.len() as u32).to_le_bytes());
        args.extend_from_slice(&self.args);
        table(&[&self.code_hash, &[self.hash_type], &args])
    }

    pub fn hash(&self) -> [u8; 32] {
        ckb_hash(&self.to_molecule())
    }
}

/// Molecule `table`: total size, field offsets, fields.
fn table(fields: &[&[u8]]) -> Vec<u8> {
    let header = 4 + 4 * fields.len();
    let total = header + fields.iter().map(|f| f.len()).sum::<usize>();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&(total as u32).to_le_bytes());
    let mut off = header;
    for f in fields {
        out.extend_from_slice(&(off as u32).to_le_bytes());
        off += f.len();
    }
    for f in fields {
        out.extend_from_slice(f);
    }
    out
}

pub struct Cell {
    pub capacity: u64,
    pub lock: Script,
    pub type_: Option<Script>,
    pub data: Vec<u8>,
}

impl Cell {
    /// Molecule encoding of `CellOutput` (capacity, lock, type).
    pub fn output_molecule(&self) -> Vec<u8> {
        let ty = self
            .type_
            .as_ref()
            .map(|s| s.to_molecule())
            .unwrap_or_default();
        table(&[&self.capacity.to_le_bytes(), &self.lock.to_molecule(), &ty])
    }

    pub fn data_hash(&self) -> [u8; 32] {
        ckb_hash(&self.data)
    }

    /// Bytes the cell occupies on chain (shannons = bytes * 1e8).
    pub fn occupied_capacity(&self) -> u64 {
        let script = |s: &Script| 32 + 1 + s.args.len() as u64;
        let bytes = 8
            + self.data.len() as u64
            + script(&self.lock)
            + self.type_.as_ref().map(script).unwrap_or(0);
        bytes * 100_000_000
    }
}

pub struct VirtualTx {
    pub tx_hash: [u8; 32],
    /// Input 0, the job cell.
    pub input: Cell,
    pub input_out_point: ([u8; 32], u32),
    pub cell_deps: Vec<Cell>,
    pub witnesses: Vec<Vec<u8>>,
}

impl VirtualTx {
    /// `program` is the ELF run in the VM, `data_cells` the extra files offered as cell deps.
    pub fn new(
        program: Vec<u8>,
        data_cells: Vec<Vec<u8>>,
        args: Vec<u8>,
        payload: Vec<u8>,
    ) -> Self {
        let lock = Script {
            code_hash: [0; 32],
            hash_type: 0,
            args: Vec::new(),
        };
        let service_script = Script {
            code_hash: ckb_hash(&program),
            hash_type: HASH_TYPE_DATA2,
            args,
        };
        let dep = |data: Vec<u8>| Cell {
            capacity: 0,
            lock: lock.clone(),
            type_: None,
            data,
        };
        let mut cell_deps = vec![dep(program)];
        cell_deps.extend(data_cells.into_iter().map(dep));
        let mut seed = b"csr-virtual-tx-v1".to_vec();
        seed.extend_from_slice(&service_script.hash());
        seed.extend_from_slice(&payload);
        let tx_hash = ckb_hash(&seed);
        VirtualTx {
            tx_hash,
            input: Cell {
                capacity: 0,
                lock,
                type_: Some(service_script),
                data: Vec::new(),
            },
            input_out_point: (ckb_hash(b"csr-virtual-job-cell"), 0),
            cell_deps,
            witnesses: vec![payload],
        }
    }

    pub fn script(&self) -> &Script {
        self.input
            .type_
            .as_ref()
            .expect("job cell has a type script")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_hash_matches_ckb() {
        // blake2b-256("ckb-default-hash", "") as used for the empty-data hash in CKB.
        assert_eq!(
            hex::encode(ckb_hash(&[])),
            "44f4c69744d5f8c55d642062949dcae49bc4e7ef43d388c5a12f42b5633d163e"
        );
    }

    #[test]
    fn script_molecule_layout() {
        let s = Script {
            code_hash: [1; 32],
            hash_type: 4,
            args: vec![9, 9],
        };
        let m = s.to_molecule();
        // 4 size + 3*4 offsets + 32 + 1 + (4 + 2)
        assert_eq!(m.len(), 4 + 12 + 32 + 1 + 6);
        assert_eq!(
            u32::from_le_bytes(m[0..4].try_into().unwrap()) as usize,
            m.len()
        );
        assert_eq!(&m[4..8], &16u32.to_le_bytes());
    }
}
