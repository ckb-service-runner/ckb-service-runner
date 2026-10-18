// hello-js: result = "hello:" followed by the job payload.
// ckb-js-vm has no binding for raw syscalls, so the payload comes from witness 0 and the result is
// returned through the host's `CSR-RESULT:0x<hex>` debug-line fallback.
import * as ckb from "@ckb-js-std/bindings";

const payload = new Uint8Array(ckb.loadWitness(0, ckb.SOURCE_GROUP_INPUT));
const prefix = "hello:";
let hex = "";
for (let i = 0; i < prefix.length; i++) hex += prefix.charCodeAt(i).toString(16).padStart(2, "0");
for (let i = 0; i < payload.length; i++) hex += payload[i].toString(16).padStart(2, "0");
ckb.debug("CSR-RESULT:0x" + hex);
