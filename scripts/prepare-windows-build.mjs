import { spawn } from 'node:child_process';

const tasks = [
  ['Configure app metadata', 'scripts/configure-build.mjs', process.env, false],
  ['Discover and prepare app icon', 'scripts/prepare-branding.mjs', process.env, false],
  ['Download Windows master binary', 'scripts/download-master-binary.mjs', process.env, false],
];

if (process.env.MASTER_TEMPLATE_MSI_ASSET && process.env.MASTER_TEMPLATE_MSI_OUTPUT) {
  tasks.push([
    'Download pre-compiled Windows template MSI',
    'scripts/download-master-binary.mjs',
    {
      ...process.env,
      MASTER_BINARY_ASSET: process.env.MASTER_TEMPLATE_MSI_ASSET,
      MASTER_BINARY_OUTPUT: process.env.MASTER_TEMPLATE_MSI_OUTPUT,
    },
    true,
  ]);
}

const results = await Promise.all(tasks.map(([label, script, env, optional]) => new Promise((resolve) => {
  const child = spawn(process.execPath, [script], {
    cwd: process.cwd(),
    env,
    stdio: 'inherit',
  });
  let settled = false;
  const finish = (code) => {
    if (settled) return;
    settled = true;
    resolve(optional ? 0 : code);
  };

  child.once('error', (error) => {
    if (!optional) console.error(`${label} could not start: ${error.message}`);
    finish(1);
  });
  child.once('exit', (code, signal) => {
    if (signal && !optional) console.error(`${label} stopped by ${signal}`);
    finish(code ?? 1);
  });
})));

if (results.some((code) => code !== 0)) {
  process.exitCode = 1;
}
