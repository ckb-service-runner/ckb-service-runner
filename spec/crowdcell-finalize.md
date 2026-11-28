# Service `crowdcell-finalize`

Finalizes one CrowdCell campaign (all-or-nothing crowdfunding on CKB) after its deadline: the campaign cell goes from
`Active` to `Success` (total pledged >= funding goal) or `Failed`. Creator side only: no backer settle jobs, no cleanup,
no contract change. The creator's create transaction also makes one CSR job cell `AtBlock(deadline)` with this service.

## Payload

`u32 LE script_len ‖ campaign type script ‖ 36-byte out point of the campaign code cell ‖ 36-byte out point of the campaign-lock code cell`

- The type script is a molecule `Script` with the full args (64 bytes: TypeID ‖ pledge-lock code hash), at most 4096 bytes.
- An out point is tx hash ‖ index u32 LE. Nothing else, no trailing bytes.

## Behaviour

1. Parse the payload. 2. `cells_query` (type script, prefix match) finds the live campaign cell. 3. `chain_read`
kind 0 gives its `CellOutput`, kind 1 its data; the tip comes from `block_info`.

| Situation | Skeleton | Result `[code, new_status]` |
|---|---|---|
| Finalized | skeleton | `[0, new_status]` (1 Success, 2 Failed) |
| Already finalized (status not Active) | empty skeleton | `[1, 0]` |
| Tip below the deadline | empty skeleton | `[2, 0]` |
| No live campaign cell | empty skeleton | `[3, 0]` |

An empty skeleton means the service never calls `tx_skeleton_put`; the run settles with only its tip (D54).

Exit codes (a failed run is counted; after 3 failures the job is skipped):
- `3` malformed payload;
- `4` a host call failed, or more than one live cell matched (payload script too short);
- `5` malformed or unknown campaign data (too short, unknown status, deadline 0, unusable output).

## The skeleton the contracts accept

Campaign data: `creator_lock_hash` [0..32], `funding_goal` u64 LE [32..40], `deadline_block` u64 LE [40..48],
`total_pledged` u64 LE [48..56], **status byte 56** (0 Active, 1 Success, 2 Failed), reserved [57..65], optional metadata tail.

- One input: the campaign cell with `since` = `deadline_block` (absolute block number, plain encoding, not 0).
- One output built from the cell's own `CellOutput` bytes (same capacity, lock, type): the campaign keeps every
  shannon; the fee comes from the job cell.
- Output data identical to the input data except status byte 56: 1 if `total_pledged >= funding_goal`, else 2.
- Cell deps: the `campaign` type code cell and the `campaign-lock` code cell. One empty witness; no signature checked.

A finalization before the deadline is impossible (`since` check in both scripts), so the service answers `[2, 0]`.

## Idempotency

A second run on a finalized campaign sees status byte 56 != 0, returns an empty skeleton and `[1, 0]`; nothing changes.
This is not an error and the exit code is 0.

## Cost and job settings

Two node calls (`cells_query`, `chain_read` twice, `block_info`) cost 200,000 cycles for the node-backed calls plus the usual
VM and host-call cycles, see `spec/host-interface.md`. Measured total: to be measured on devnet.

- `expiry_block`: the deadline plus a margin (the keeper's hold-back plus time for retries), so a late run still counts.
- `max_cycles`: the measured cycles plus headroom; until measured, use a generous value (for example 2,000,000).
