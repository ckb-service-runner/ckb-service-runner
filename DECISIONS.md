
## RFC draft
- Draft written in `docs/rfc/`; number `0000`, date 2026-11-10; the spec wins on any difference.
- Listed 7050/7051 as reserved only; 7099 appears only as the range end. No syscalls invented.
- Reserved-range registration for a future CKB-VM is phrased as a documentation registry, not a VM change.
- Retry: the previous run failed because Node 22 treats `node --test acceptance/aiworker-csr-host-interface-rfc/` as a module path. Added the sibling shim `acceptance/aiworker-csr-host-interface-rfc.js` (same pattern as `aiworker-csr-app-sdk.js`), which imports the `*.test.mjs` files in that directory; the protected directory itself is untouched.
