//! Service manifests (`services/manifests/<name>.toml`).

use super::vtx::ckb_hash;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    name: String,
    language: String,
    elf: String,
    #[serde(default)]
    identity: String,
    #[serde(default)]
    data_cells: Vec<String>,
    #[serde(default)]
    args: String,
}

// `name` and `language` are used by the services registry (later phases).
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Manifest {
    pub name: String,
    pub language: String,
    pub elf: PathBuf,
    pub identity: PathBuf,
    pub data_cells: Vec<PathBuf>,
    pub args: Vec<u8>,
}

/// A manifest with its files read.
pub struct LoadedService {
    pub manifest: Manifest,
    pub elf: Vec<u8>,
    pub data_cells: Vec<Vec<u8>>,
    /// CKB data hash of the identity file.
    pub code_hash: [u8; 32],
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Manifest, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read manifest {}: {e}", path.display()))?;
        let raw: Raw = toml::from_str(&text)
            .map_err(|e| format!("invalid manifest {}: {e}", path.display()))?;
        if !matches!(raw.language.as_str(), "rust" | "c" | "js") {
            return Err(format!(
                "manifest language must be rust, c or js, got {:?}",
                raw.language
            ));
        }
        if raw.name.is_empty() {
            return Err("manifest name must not be empty".into());
        }
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        let resolve = |p: &str| {
            let p = Path::new(p);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                dir.join(p)
            }
        };
        let elf = resolve(&raw.elf);
        let identity = if raw.identity.is_empty() {
            elf.clone()
        } else {
            resolve(&raw.identity)
        };
        let args_text = raw.args.trim();
        let args_hex = args_text.strip_prefix("0x").unwrap_or(args_text);
        let args = hex::decode(args_hex)
            .map_err(|_| format!("manifest args must be 0x-prefixed hex, got {:?}", raw.args))?;
        Ok(Manifest {
            name: raw.name,
            language: raw.language,
            elf,
            identity,
            data_cells: raw.data_cells.iter().map(|p| resolve(p)).collect(),
            args,
        })
    }

    pub fn read_files(self) -> Result<LoadedService, String> {
        let read =
            |p: &Path| std::fs::read(p).map_err(|e| format!("cannot read {}: {e}", p.display()));
        let elf = read(&self.elf)?;
        let identity = if self.identity == self.elf {
            elf.clone()
        } else {
            read(&self.identity)?
        };
        let data_cells = self
            .data_cells
            .iter()
            .map(|p| read(p))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(LoadedService {
            code_hash: ckb_hash(&identity),
            manifest: self,
            elf,
            data_cells,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_paths_and_hashes_identity() {
        let tmp = crate::test_tmp();
        std::fs::write(tmp.path().join("prog.bin"), b"abc").unwrap();
        let m = tmp.path().join("m.toml");
        std::fs::write(
            &m,
            "name = \"x\"\nlanguage = \"rust\"\nelf = \"prog.bin\"\nargs = \"0x0102\"\n",
        )
        .unwrap();
        let loaded = Manifest::load(&m).unwrap().read_files().unwrap();
        assert_eq!(loaded.manifest.args, vec![1, 2]);
        assert_eq!(loaded.code_hash, ckb_hash(b"abc"));
    }

    #[test]
    fn rejects_bad_language_and_unknown_fields() {
        let tmp = crate::test_tmp();
        let m = tmp.path().join("m.toml");
        std::fs::write(&m, "name = \"x\"\nlanguage = \"go\"\nelf = \"a\"\n").unwrap();
        assert!(Manifest::load(&m).is_err());
        std::fs::write(&m, "name = \"x\"\nlanguage = \"c\"\nelf = \"a\"\nfoo = 1\n").unwrap();
        assert!(Manifest::load(&m).is_err());
    }
}
