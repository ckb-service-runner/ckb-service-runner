# Decisions (A3a audit fixes)

The project's decision log is `docs/decisions.md` (sections "A3a-fix Phase …"). Short list for this run:

- Phase 1: F1/F3/F4 implemented as specified; new error codes float 10 (`ERR_OUTPUT_SHAPE`), registry 17
  (`ERR_NO_RUNNER_KEY`). Rule 6 of the registry also triggers when the owner changes only the runner lock hash
  (not only `service_pubkey`), and is checked after rules 1–5 so existing error codes are unchanged.
- Obsolete float fee tests deleted; see `docs/decisions.md` for the test-helper changes.
