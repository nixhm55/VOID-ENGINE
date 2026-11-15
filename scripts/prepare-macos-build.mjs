import { spawn } from 'node:child_process';

const tasks = [
  ['Configure app metadata', 'scripts/configure-build.mjs', process.env],
  ['Discover and prepare app icon', 'scripts/prepare-branding.mjs', process.env],
  ['Download macOS master binary', 'scripts/download-master-binary.mjs', process.env],
];

if (process.env.MASTER_TEMPLATE_ASSET && process.env.MASTER_TEMPLATE_OUTPUT) {
  tasks.push([
    'Download pre-built macOS template DMG',
    'scripts/download-master-binary.mjs',
    {
      ...process.env,
      MASTER_BINARY_ASSET: process.env.MASTER_TEMPLATE_ASSET,
      MASTER_BINARY_OUTPUT: process.env.MASTER_TEMPLATE_OUTPUT,
    },
  ]);
}

const results = await Promise.all(tasks.map(([label, script, env]) => new Promise((resolve) => {
  const isOptionalTemplate = label.includes('template DMG');
  const child = spawn(process.execPath, [script], {
    cwd: process.cwd(),
    env,
    stdio: 'inherit',
  });
  let settled = false;
  const finish = (code) => {
    if (settled) return;
    settled = true;
    resolve(isOptionalTemplate ? 0 : code);
  };
  child.once('error', (error) => {
    if (!isOptionalTemplate) console.error(`${label} could not start: ${error.message}`);
    finish(1);
  });
  child.once('exit', (code, signal) => {
    if (signal && !isOptionalTemplate) console.error(`${label} stopped by ${signal}`);
    finish(code ?? 1);
  });
})));

if (results.some((code) => code !== 0)) {
  process.exitCode = 1;
}
