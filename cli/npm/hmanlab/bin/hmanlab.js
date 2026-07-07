#!/usr/bin/env node
// hmanlab launcher. Resolves the matching prebuilt binary from the
// per-arch optional dependency and execs it with the user's args. The
// binary itself is a self-contained Rust executable — this shim just
// figures out which one to run and forwards stdio + signals.

'use strict';

const { spawn } = require('node:child_process');

const { platform, arch } = process;
// npm normalises win32 platforms to 'win32', x64/arm64 are already the
// arch names we use in the subpackage names — match them 1:1.
const pkg = `@hmanlab/${platform}-${arch}`;
const exe = platform === 'win32' ? 'hmanlab.exe' : 'hmanlab';

let binPath;
try {
  binPath = require.resolve(`${pkg}/bin/${exe}`);
} catch (e) {
  console.error(
    `hmanlab: no prebuilt binary for ${platform}-${arch}.\n` +
    'Supported platforms: linux-x64, linux-arm64, darwin-x64, darwin-arm64, win32-x64.\n' +
    'Install Rust and build from source: https://github.com/rekabytes/hmanlab'
  );
  process.exit(1);
}

const child = spawn(binPath, process.argv.slice(2), { stdio: 'inherit' });

// Forward signals to the child so Ctrl+C in the parent shell shuts down
// the TUI cleanly instead of orphaning it with a half-painted terminal.
for (const sig of ['SIGINT', 'SIGTERM', 'SIGHUP', 'SIGQUIT']) {
  process.on(sig, () => {
    try { child.kill(sig); } catch (_) { /* child already gone */ }
  });
}

child.on('error', (err) => {
  console.error(`hmanlab: failed to launch binary (${binPath}): ${err.message}`);
  process.exit(1);
});

child.on('exit', (code, signal) => {
  // If the child died by signal, re-raise it on ourselves so the parent
  // shell's exit-code reporting (e.g. 130 for SIGINT) stays consistent
  // with running the native binary directly.
  if (signal) {
    process.kill(process.pid, signal);
  } else {
    process.exit(code ?? 0);
  }
});
