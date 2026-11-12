import { spawn } from 'node:child_process';

const tasks = [
  ['Configure app metadata', 'scripts/configure-build.mjs'],
  ['Discover and prepare app icon', 'scripts/prepare-branding.mjs'],
  ['Download Windows master binary', 'scripts/download-master-binary.mjs'],
];

const results = await Promise.all(tasks.map(([label, script]) => new Promise((resolve) => {
  const child = spawn(process.execPath, [script], {
    cwd: process.cwd(),
    env: process.env,
    stdio: 'inherit',
  });
  let settled = false;
  const finish = (code) => {
    if (settled) return;
    settled = true;
    resolve(code);
  };

  child.once('error', (error) => {
    console.error(`${label} could not start: ${error.message}`);
    finish(1);
  });
  child.once('exit', (code, signal) => {
    if (signal) console.error(`${label} stopped by ${signal}`);
    finish(code ?? 1);
  });
})));

if (results.some((code) => code !== 0)) {
  process.exitCode = 1;
}
