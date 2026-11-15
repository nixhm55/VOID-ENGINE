import { spawnSync } from 'node:child_process';

const res = spawnSync('bash', ['scripts/package-macos-dmg.sh'], { stdio: 'inherit', env: process.env });
if (res.status !== 0) process.exit(res.status ?? 1);
