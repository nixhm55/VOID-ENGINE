import fs from 'node:fs/promises';
import path from 'node:path';

const repository = (process.env.MASTER_BINARY_REPOSITORY || 'nixhm55/VOID-ENGINE').trim();
const releasePrefix = (process.env.MASTER_BINARY_RELEASE_PREFIX || '').trim();
const sourceSha = (process.env.MASTER_BINARY_SOURCE_SHA || '').trim().toLowerCase();
const asset = (process.env.MASTER_BINARY_ASSET || '').trim();
const output = (process.env.MASTER_BINARY_OUTPUT || '').trim();

if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository) || repository.includes('..')) {
  throw new Error('MASTER_BINARY_REPOSITORY must be owner/repository');
}
if (!/^[A-Za-z0-9_.-]+$/.test(asset) || !output) {
  throw new Error('MASTER_BINARY_ASSET and MASTER_BINARY_OUTPUT are required');
}
if (sourceSha && !/^[0-9a-f]{7,40}$/.test(sourceSha)) {
  throw new Error('MASTER_BINARY_SOURCE_SHA must be a Git commit SHA');
}

const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));
const waitSeconds = Number(process.env.MASTER_BINARY_WAIT_SECONDS || 1200);
const pollMilliseconds = Number(process.env.MASTER_BINARY_POLL_MILLISECONDS || 15000);
const deadline = Date.now() + (sourceSha ? waitSeconds * 1000 : 0);

let url;
if (releasePrefix) {
  if (!/^engine-master-(lite|max)-$/.test(releasePrefix)) {
    throw new Error('MASTER_BINARY_RELEASE_PREFIX must be engine-master-lite- or engine-master-max-');
  }
  const requiredTagPrefix = sourceSha
    ? `${releasePrefix}${sourceSha.slice(0, 12)}`
    : releasePrefix;
  let lastReason = 'no matching profile release is published yet';
  while (true) {
    const releasesResponse = await fetch(`https://api.github.com/repos/${repository}/releases?per_page=100`, {
      signal: AbortSignal.timeout(20_000),
      headers: { 'User-Agent': 'VOID-master-binary-downloader/1.0', Accept: 'application/vnd.github+json' },
    });
    if (!releasesResponse.ok) {
      throw new Error(`Could not list profile releases from ${repository}: HTTP ${releasesResponse.status}`);
    }
    const releases = await releasesResponse.json();
    const release = releases
      .filter((candidate) => !candidate.draft && candidate.tag_name.startsWith(requiredTagPrefix))
      .sort((a, b) => Date.parse(b.created_at) - Date.parse(a.created_at))[0];
    const foundAsset = release?.assets?.find((candidate) => candidate.name === asset);
    if (release && foundAsset) {
      url = foundAsset.browser_download_url;
      break;
    }
    if (release) lastReason = `release ${release.tag_name} does not contain ${asset} yet`;
    if (Date.now() >= deadline) {
      const commitHint = sourceSha ? ` for commit ${sourceSha.slice(0, 12)}` : '';
      throw new Error(`No ${releasePrefix.slice(13, -1)} master asset ${asset} is published${commitHint} for ${repository} (${lastReason})`);
    }
    console.log(`Waiting for ${asset} from ${requiredTagPrefix} (${lastReason})`);
    await sleep(Math.min(pollMilliseconds, Math.max(1, deadline - Date.now())));
  }
} else {
  url = `https://github.com/${repository}/releases/latest/download/${encodeURIComponent(asset)}`;
}
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
