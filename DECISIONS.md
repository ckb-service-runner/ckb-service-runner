
## RFC draft
- Draft written in `docs/rfc/`; number `0000`, date 2026-11-10; the spec wins on any difference.
- Listed 7050/7051 as reserved only; 7099 appears only as the range end. No syscalls invented.
- Reserved-range registration for a future CKB-VM is phrased as a documentation registry, not a VM change.
- Retry: the previous run failed because Node 22 treats `node --test acceptance/aiworker-csr-host-interface-rfc/` as a module path. Added the sibling shim `acceptance/aiworker-csr-host-interface-rfc.js` (same pattern as `aiworker-csr-app-sdk.js`), which imports the `*.test.mjs` files in that directory; the protected directory itself is untouched.

## csr-07-security phase 3 (Windows installer)
- Data root: `/inheritance:r` + grants on the root, then `icacls "<data>\*" /reset /T` so existing children inherit the new ACL (a plain `/inheritance:r /T` would leave children with empty ACLs), then `/remove:g *S-1-5-18 /T`. Install-Files also sets an Administrators-only ACL right after creating the folder so files written before the services exist are never world-readable.
- The dashboard password step runs before the services are registered (and before Restrict-DataRoot), so its files get the final ACL from the reset.
- The wide key search is asked at most once (cached) and only reached when the known places found no key; non-interactive sessions answer no.
- `binPath=` embeds quotes as `\"` because PowerShell 5.1 does not escape quotes in native arguments.
- A missing read ACE grant is only a warning (the install continues, identity stays unproven); a failed `sc.exe` call stops the install.
- package-windows.sh's README text mentions neither tasks nor SYSTEM, so it is unchanged.

## Phase 1 notes
- Phase 1 follows the spec's reference files unchanged (comments/formatting only). `csr-svc` lives in the separate `services/rust` workspace (the root workspace excludes `services`), so `cargo check -p csr-svc` passes when run from `services/rust` but not from the repo root; I did not change the root workspace (spec constraint).
- The RISC-V binary `crowdcell-finalize` was not compiled here (target not available); the owner builds it with `scripts/build-services.sh`.

- acceptance/a1/cargo.sh now switches to `services/rust` when `-p csr-svc|crowdcell-finalize[-logic]` is given, because the root workspace excludes `services` and `cargo check -p csr-svc` from the repo root otherwise fails ("did not match any packages").
