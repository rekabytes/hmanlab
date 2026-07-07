// Smoke test for bin/hmanlab.js. Runs without `npm install`: the shim
// has no runtime deps, so we can exercise the error path directly.
// With no @hmanlab/* optional dep resolvable, require.resolve fails
// and the shim must print "no prebuilt binary" and exit 1. This is
// the same path a user on an unsupported platform sees.

'use strict';

const { spawnSync } = require('node:child_process');
const path = require('node:path');

const shim = path.join(__dirname, '..', 'bin', 'hmanlab.js');
const result = spawnSync(process.execPath, [shim], { encoding: 'utf8' });

const failures = [];

if (result.status !== 1) {
  failures.push(`expected exit code 1, got ${result.status}`);
}
if (!/no prebuilt binary/.test(result.stderr)) {
  failures.push(`stderr missing "no prebuilt binary":\n${result.stderr}`);
}
if (!/Supported platforms/.test(result.stderr)) {
  failures.push(`stderr missing "Supported platforms":\n${result.stderr}`);
}

if (failures.length > 0) {
  console.error('smoke test FAILED:');
  for (const f of failures) console.error('  - ' + f);
  process.exit(1);
}

console.log('smoke test passed');
