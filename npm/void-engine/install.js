#!/usr/bin/env node
'use strict';

const { randomBytes } = require('node:crypto');
const fs = require('node:fs/promises');
const https = require('node:https');
const path = require('node:path');
const { createWriteStream } = require('node:fs');
const { pipeline } = require('node:stream/promises');
const packageVersion = require('./package.json').version;

const REPOSITORY = 'nixhm55/VOID-ENGINE';
const RELEASE_URL = `https://github.com/${REPOSITORY}/releases/latest/download/`;
const MAX_REDIRECTS = 10;
const REQUEST_TIMEOUT_MS = 30_000;

const TARGETS = {
  'darwin-arm64': { asset: 'void-macos', executable: 'void' },
  'linux-x64': { asset: 'void-linux', executable: 'void' },
  'win32-x64': { asset: 'void-windows.exe', executable: 'void.exe' },
};

function getResponse(url) {
  return new Promise((resolve, reject) => {
    const request = https.get(
      url,
      {
        headers: {
          Accept: 'application/octet-stream',
          'User-Agent': `void-engine-npm/${packageVersion}`,
        },
      },
      resolve,
    );

    request.setTimeout(REQUEST_TIMEOUT_MS, () => {
      request.destroy(new Error(`Request timed out after ${REQUEST_TIMEOUT_MS / 1000} seconds.`));
    });
    request.on('error', reject);
  });
}

async function download(url, destination, redirects = 0) {
  if (redirects > MAX_REDIRECTS) {
    throw new Error(`GitHub Release download exceeded ${MAX_REDIRECTS} redirects.`);
  }

  const response = await getResponse(url);
  const status = response.statusCode || 0;

  if ([301, 302, 303, 307, 308].includes(status)) {
    const location = response.headers.location;
    response.resume();
    if (!location) {
      throw new Error(`GitHub Release returned HTTP ${status} without a redirect location.`);
    }

    const nextUrl = new URL(location, url);
    if (nextUrl.protocol !== 'https:') {
      throw new Error('GitHub Release redirected the binary download to a non-HTTPS URL.');
    }
    return download(nextUrl, destination, redirects + 1);
  }

  if (status !== 200) {
    response.resume();
    throw new Error(
      `GitHub Release download failed with HTTP ${status}. Confirm a published release contains the expected asset.`,
    );
  }

  const temporaryPath = `${destination}.${process.pid}.${randomBytes(6).toString('hex')}.tmp`;
  try {
    await pipeline(response, createWriteStream(temporaryPath, { flags: 'wx', mode: 0o755 }));
    const file = await fs.stat(temporaryPath);
    if (file.size === 0) {
      throw new Error('GitHub Release returned an empty binary.');
    }

    if (process.platform !== 'win32') {
      await fs.chmod(temporaryPath, 0o755);
    }
    await fs.rename(temporaryPath, destination);
  } catch (error) {
    await fs.rm(temporaryPath, { force: true }).catch(() => {});
    throw error;
  }
}

async function main() {
  const target = TARGETS[`${process.platform}-${process.arch}`];
  if (!target) {
    throw new Error(
      `No VOID client release is available for ${process.platform}/${process.arch}. Supported targets: macOS arm64, Linux x64, and Windows x64.`,
    );
  }

  const binDirectory = path.join(__dirname, 'bin');
  const destination = path.join(binDirectory, target.executable);
  await fs.mkdir(binDirectory, { recursive: true });
  await download(new URL(encodeURIComponent(target.asset), RELEASE_URL), destination);
}

main().catch((error) => {
  console.error(`[void-engine] Installation failed: ${error.message}`);
  console.error('[void-engine] Check your internet connection and try installing the package again.');
  process.exitCode = 1;
});
