# Decisions (A3a audit fixes)

The project's decision log is `docs/decisions.md` (sections "A3a-fix Phase …"). Short list for this run:

- Phase 1: F1/F3/F4 implemented as specified; new error codes float 10 (`ERR_OUTPUT_SHAPE`), registry 17
  (`ERR_NO_RUNNER_KEY`). Rule 6 of the registry also triggers when the owner changes only the runner lock hash
  (not only `service_pubkey`), and is checked after rules 1–5 so existing error codes are unchanged.
- Obsolete float fee tests deleted; see `docs/decisions.md` for the test-helper changes.
- Phase 2: F2/F5 in `csr-job-lock` (errors 15, 16, 17), checked after the tip (11) so existing codes stay.
  Short config data also reports 16. The dep lock's code hash is not checked (only args length, as specified).
  Extra tests: malformed registry lock (10), two config deps (16). Docs: job/registry specs, decisions
  ("Downstream impact"), milestone "Security fixes", CHANGELOG (also an entry for Phase 1).
