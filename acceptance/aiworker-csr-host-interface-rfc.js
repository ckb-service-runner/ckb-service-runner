// Node 22 runs `node --test <dir>` as a module path; `<dir>` resolves to this file first.
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const dir = path.join(__dirname, 'aiworker-csr-host-interface-rfc');
for (const f of fs.readdirSync(dir).filter((f) => /\.test\.m?js$/.test(f)).sort()) {
  import(pathToFileURL(path.join(dir, f)).href);
}
