import fs from 'node:fs/promises';
import path from 'node:path';

const repository = (process.env.MASTER_BINARY_REPOSITORY || 'nixhm55/VOID-ENGINE').trim();
const asset = (process.env.MASTER_BINARY_ASSET || '').trim();
const output = (process.env.MASTER_BINARY_OUTPUT || '').trim();

if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository) || repository.includes('..')) {
  throw new Error('MASTER_BINARY_REPOSITORY must be owner/repository');
}
if (!/^[A-Za-z0-9_.-]+$/.test(asset) || !output) {
  throw new Error('MASTER_BINARY_ASSET and MASTER_BINARY_OUTPUT are required');
}

const url = `https://github.com/${repository}/releases/latest/download/${encodeURIComponent(asset)}`;
const response = await fetch(url, {
  redirect: 'follow',
  signal: AbortSignal.timeout(90_000),
  headers: { 'User-Agent': 'VOID-master-binary-downloader/1.0', Accept: 'application/octet-stream' },
});
if (!response.ok) {
  const detail = (await response.text()).slice(0, 300);
  throw new Error(`Could not download ${asset} from ${repository}: HTTP ${response.status} ${detail}`);
}
const contentType = response.headers.get('content-type') || '';
if (contentType.includes('text/html')) {
  throw new Error(`GitHub did not return the ${asset} release asset; publish the master-binaries release first`);
}

const bytes = Buffer.from(await response.arrayBuffer());
if (bytes.length < 1024) throw new Error(`Downloaded master asset ${asset} is unexpectedly small (${bytes.length} bytes)`);
await fs.mkdir(path.dirname(output), { recursive: true });
await fs.writeFile(output, bytes);
console.log(`Downloaded ${asset} (${(bytes.length / 1_048_576).toFixed(1)} MiB) to ${output}`);
