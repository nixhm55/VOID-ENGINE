#!/usr/bin/env node
'use strict';

const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const executableName = process.platform === 'win32' ? 'void.exe' : 'void';
const executablePath = path.join(__dirname, executableName);

if (!fs.existsSync(executablePath)) {
  console.error('[void-engine] The platform client is missing. Reinstall with: npm install --global void-engine');
  process.exitCode = 1;
} else {
  const result = spawnSync(executablePath, process.argv.slice(2), {
    stdio: 'inherit',
    windowsHide: true,
  });

  if (result.error) {
    console.error(`[void-engine] Could not start the client: ${result.error.message}`);
    process.exitCode = 1;
  } else if (result.signal) {
    const signalNumber = os.constants.signals[result.signal];
    process.exitCode = typeof signalNumber === 'number' ? 128 + signalNumber : 1;
  } else {
    process.exitCode = typeof result.status === 'number' ? result.status : 1;
  }
}
