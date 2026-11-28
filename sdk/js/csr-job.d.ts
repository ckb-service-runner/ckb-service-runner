// TypeScript declarations for csr-job.mjs. u64 values are `bigint`, byte strings are 0x-hex `string`.

export interface OutPoint { txHash: string; index: number }

export type Trigger =
  | { kind: 'OnDemand' }
  | { kind: 'AtBlock'; block: bigint }
  | { kind: 'AtEpoch'; epoch: bigint }
  | { kind: 'AtTimestamp'; timestampMs: bigint }
  | { kind: 'Every'; blocks: bigint };

export interface Job {
  trigger: Trigger;
  tipShannons: bigint;
  maxFeeShannons: bigint;
  maxCycles: bigint;
  /** 0x-hex, defaults to '0x' */
  payload?: string;
  expiryBlock: bigint;
  refundLockHash: string;
  remainingRuns: number;
}

export interface DeploymentScript { code_hash: string; hash_type: 'data2'; tx_hash: string; index: number }

export interface Deployment {
  registry_type_hash: string;
  scripts: { job_type: DeploymentScript; job_lock: DeploymentScript; [name: string]: DeploymentScript };
  [key: string]: unknown;
}

export interface Script { codeHash: string; hashType: string; args: string }

export interface JobCell {
  cellOutput: { capacity: string; lock: Script; type: Script };
  outputData: string;
}

export interface CellDep { outPoint: OutPoint; depType: 'code' }

export interface CrowdcellFinalizeParams {
  /** molecule-encoded campaign type script, 1..4096 bytes */
  typeScript: string;
  campaignCodeOutPoint: OutPoint;
  lockCodeOutPoint: OutPoint;
}

export interface ReclaimParams { deployment: Deployment; jobOutPoint: OutPoint; expiryBlock: bigint }

export interface ReclaimResult {
  cellDeps: CellDep[];
  jobInput: { previousOutput: OutPoint; since: bigint };
}

export function encodeJob(job: Job): string;
export function decodeJob(hex: string): Required<Job>;
export function loadDeployment(json: string | Deployment): Deployment;
export function minJobCapacity(args: { deployment: Deployment; serviceId: string; job: Job }): bigint;
export function jobCell(args: { deployment: Deployment; serviceId: string; job: Job; extraCapacityShannons?: bigint }): JobCell;
export function jobCellDeps(deployment: Deployment): CellDep[];
export function watchPayload(outPoint: OutPoint, innerHex?: string): string;
export function crowdcellFinalizePayload(args: CrowdcellFinalizeParams): string;
export function reclaimJob(args: ReclaimParams): ReclaimResult;
export function blake2b256(data: Uint8Array): Uint8Array;
export function serviceId(codeHash: string, argsHex?: string, dataHashes?: string[]): string;
