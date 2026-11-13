import fs from 'node:fs/promises';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const appName = (process.env.APP_NAME || 'VOID').trim();
const appSlug = (process.env.APP_SLUG || 'void').trim();
const version = (process.env.APP_VERSION || '1.0.0').trim();
const engineProfile = (process.env.ENGINE_PROFILE || 'lite').trim();
const binary = process.env.MASTER_BINARY_PATH || 'target/release/void';
const entitlements = 'macos/void.entitlements';
const outputDir = 'output';
const staging = 'dmg_staging';
const appPath = path.join(staging, `${appSlug}.app`);
const contents = path.join(appPath, 'Contents');

if (!appName || !/^[a-z0-9_]+$/.test(appSlug) || !/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(version)
    || !['lite', 'max'].includes(engineProfile)) {
  throw new Error('Invalid app name, slug, or semantic app version');
}

function run(command, args) {
  const result = spawnSync(command, args, { encoding: 'utf8' });
  if (result.status !== 0) throw new Error(`${command} failed: ${result.stderr || result.stdout}`);
}
function xml(value) {
  return String(value).replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;').replaceAll("'", '&apos;');
}

await fs.rm(staging, { recursive: true, force: true });
await fs.rm(outputDir, { recursive: true, force: true });
await fs.mkdir(path.join(contents, 'MacOS'), { recursive: true });
await fs.mkdir(path.join(contents, 'Resources', 'AppIcon.iconset'), { recursive: true });
await fs.copyFile(binary, path.join(contents, 'MacOS', 'void'));
await fs.chmod(path.join(contents, 'MacOS', 'void'), 0o755);
await fs.copyFile('void.json', path.join(contents, 'Resources', 'void.json'));
await fs.copyFile('void.json', path.join(contents, 'MacOS', 'void.json'));

const iconset = path.join(contents, 'Resources', 'AppIcon.iconset');
for (const size of [16, 32, 128, 256, 512]) {
  run('sips', ['-z', String(size), String(size), 'branding/icon.png', '--out', path.join(iconset, `icon_${size}x${size}.png`)]);
}
for (const [size, name] of [[32, 'icon_16x16@2x.png'], [64, 'icon_32x32@2x.png'], [256, 'icon_128x128@2x.png'], [512, 'icon_256x256@2x.png'], [1024, 'icon_512x512@2x.png']]) {
  run('sips', ['-z', String(size), String(size), 'branding/icon.png', '--out', path.join(iconset, name)]);
}
run('iconutil', ['-c', 'icns', iconset, '-o', path.join(contents, 'Resources', 'AppIcon.icns')]);

const bundleId = `com.void.${appSlug}`;
const info = `<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n<plist version="1.0"><dict>\n` +
  `<key>CFBundleExecutable</key><string>void</string>\n<key>CFBundleIdentifier</key><string>${xml(bundleId)}</string>\n` +
  `<key>CFBundleName</key><string>${xml(appName)}</string>\n<key>CFBundleDisplayName</key><string>${xml(appName)}</string>\n` +
  '<key>CFBundleIconFile</key><string>AppIcon</string>\n<key>CFBundlePackageType</key><string>APPL</string>\n' +
  `<key>CFBundleShortVersionString</key><string>${xml(version)}</string>\n<key>CFBundleVersion</key><string>${xml(version)}</string>\n` +
  '<key>NSHighResolutionCapable</key><true/>\n' +
  '<key>NSCameraUsageDescription</key><string>VOID needs camera access for websites you choose to use.</string>\n' +
  '<key>NSMicrophoneUsageDescription</key><string>VOID needs microphone access for websites you choose to use.</string>\n' +
  (engineProfile === 'max'
    ? '<key>NSLocationUsageDescription</key><string>VOID uses location for websites you choose to use.</string>\n' +
      '<key>NSBluetoothAlwaysUsageDescription</key><string>VOID connects to Bluetooth devices when you choose to use a website feature.</string>\n'
    : '') +
  '</dict></plist>\n';
await fs.writeFile(path.join(contents, 'Info.plist'), info);
await fs.writeFile(path.join(contents, 'PkgInfo'), 'APPL????');
await fs.mkdir(outputDir, { recursive: true });
// The App Sandbox only takes effect when the entitlements travel with the
// signature; ad-hoc signing is enough for a locally distributed bundle.
await fs.access(entitlements);
run('codesign', [
  '--force',
  '--deep',
  '--sign',
  '-',
  '--entitlements',
  entitlements,
  appPath,
]);
await fs.symlink('/Applications', path.join(staging, 'Applications'));
run('hdiutil', ['create', '-volname', appName, '-srcfolder', staging, '-ov', '-format', 'UDZO', path.join(outputDir, `${appSlug}.dmg`)]);
console.log(`Packaged ${appName} as ${path.join(outputDir, `${appSlug}.dmg`)}`);
