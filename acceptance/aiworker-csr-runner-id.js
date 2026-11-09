// Node 22 runs `node --test <dir>` as a module path, not a directory scan; Node resolves
// `<dir>` to this file first, so it loads the test files in that directory.
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const dir = path.join(__dirname, 'aiworker-csr-runner-id');
for (const f of fs.readdirSync(dir).filter((f) => /\.test\.m?js$/.test(f)).sort()) {
  import(pathToFileURL(path.join(dir, f)).href);
}
