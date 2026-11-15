import fs from 'node:fs/promises';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const appName = (process.env.APP_NAME || 'VOID').trim();
const appSlug = (process.env.APP_SLUG || 'void').trim();
const version = (process.env.APP_VERSION || '1.0.0').trim();
const engineProfile = (process.env.ENGINE_PROFILE || 'lite').trim();
const binary = process.env.MASTER_BINARY_PATH || 'target/release/void';
const entitlements = 'macos/void.entitlements';
const installerBgPng = path.resolve('branding/installer-bg.png');
const installerBgTiff = path.resolve('branding/installer-bg.tiff');
const outputDir = 'output';
const staging = 'dmg_staging';
const appBundleName = `${appSlug}.app`;
const appPath = path.join(staging, appBundleName);
const contents = path.join(appPath, 'Contents');

if (!appName || !/^[a-z0-9_]+$/.test(appSlug) || !/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(version)
    || !['lite', 'max'].includes(engineProfile)) {
  throw new Error('Invalid app name, slug, or semantic app version');
}

function run(command, args, opts = {}) {
  const result = spawnSync(command, args, { encoding: 'utf8', ...opts });
  if (result.status !== 0) throw new Error(`${command} failed: ${result.stderr || result.stdout}`);
  return result.stdout || '';
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
const appIconIcns = path.resolve(contents, 'Resources', 'AppIcon.icns');
run('iconutil', ['-c', 'icns', iconset, '-o', appIconIcns]);

const bundleId = `com.void.${appSlug}`;
const info = `<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n<plist version="1.0"><dict>\n` +
  `<key>CFBundleExecutable</key><string>void</string>\n<key>CFBundleIdentifier</key><string>${xml(bundleId)}</string>\n` +
  `<key>CFBundleName</key><string>${xml(appName)}</string>\n<key>CFBundleDisplayName</key><string>${xml(appName)}</string>\n` +
  '<key>CFBundleIconFile</key><string>AppIcon.icns</string>\n<key>CFBundlePackageType</key><string>APPL</string>\n' +
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

const finalDmg = path.resolve(outputDir, `${appSlug}.dmg`);
await fs.rm(finalDmg, { force: true });

function ensureDmgbuild() {
  const check = spawnSync('python3', ['-c', 'import dmgbuild'], { encoding: 'utf8' });
  if (check.status === 0) return true;
  const pipArgsList = [
    ['-m', 'pip', 'install', '--break-system-packages', '--quiet', 'dmgbuild'],
    ['-m', 'pip', 'install', '--quiet', 'dmgbuild'],
    ['-m', 'pip', 'install', '--user', '--break-system-packages', '--quiet', 'dmgbuild'],
    ['-m', 'pip', 'install', '--user', '--quiet', 'dmgbuild'],
  ];
  for (const args of pipArgsList) {
    const res = spawnSync('python3', args, { encoding: 'utf8' });
    if (res.status === 0) {
      const recheck = spawnSync('python3', ['-c', 'import dmgbuild'], { encoding: 'utf8' });
      if (recheck.status === 0) return true;
    }
  }
  return false;
}

let bgFile = installerBgPng;
try {
  await fs.access(installerBgTiff);
  bgFile = installerBgTiff;
} catch {
  // Fallback to PNG if TIFF is absent
}

if (ensureDmgbuild()) {
  const settingsPath = path.join(outputDir, 'dmg_settings.py');
  const absAppPath = path.resolve(appPath);
  const settingsPy = `
filename = ${JSON.stringify(finalDmg)}
volume_name = ${JSON.stringify(appName)}
format = 'UDZO'
compression_level = 9
files = [(${JSON.stringify(absAppPath)}, ${JSON.stringify(appBundleName)})]
symlinks = {'Applications': '/Applications'}
icon = ${JSON.stringify(appIconIcns)}
background = ${JSON.stringify(bgFile)}
window_rect = ((140, 140), (800, 528))
icon_size = 104
text_size = 13
icon_locations = {
    ${JSON.stringify(appBundleName)}: (200, 240),
    'Applications': (600, 240),
}
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
`;
  await fs.writeFile(settingsPath, settingsPy, 'utf8');
  const buildPy = `
import ds_store
import dmgbuild

_orig_exit = ds_store.DSStore.__exit__
def _patched_exit(self, exc_type, exc_val, exc_tb):
    try:
        icvp = dict(self['.']['icvp'])
        icvp['backgroundColorRed'] = 0.0
        icvp['backgroundColorGreen'] = 0.0
        icvp['backgroundColorBlue'] = 0.0
        self['.']['icvp'] = icvp
    except Exception:
        pass
    return _orig_exit(self, exc_type, exc_val, exc_tb)

ds_store.DSStore.__exit__ = _patched_exit
dmgbuild.build_dmg(${JSON.stringify(finalDmg)}, ${JSON.stringify(appName)}, ${JSON.stringify(settingsPath)})
`;
  run('python3', ['-c', buildPy]);
  await fs.rm(settingsPath, { force: true });
} else {
  await fs.symlink('/Applications', path.join(staging, 'Applications'));
  await fs.mkdir(path.join(staging, '.background'), { recursive: true });
  await fs.copyFile(bgFile, path.join(staging, '.background', path.basename(bgFile)));
  await fs.copyFile(appIconIcns, path.join(staging, '.VolumeIcon.icns'));

  const rwDmg = path.join(outputDir, `${appSlug}-rw.dmg`);
  await fs.rm(rwDmg, { force: true });
  run('hdiutil', ['create', '-volname', appName, '-srcfolder', staging, '-ov', '-format', 'UDRW', rwDmg]);

  let mountPoint = `/Volumes/${appName}`;
  try {
    const attachOut = run('hdiutil', ['attach', '-readwrite', '-noverify', '-noautoopen', rwDmg]);
    const match = attachOut.match(/\/Volumes\/[^\r\n]+/);
    if (match) mountPoint = match[0].trim();
    spawnSync('SetFile', ['-a', 'C', mountPoint]);
    spawnSync('SetFile', ['-a', 'V', path.join(mountPoint, '.VolumeIcon.icns')]);
    const bgPosix = path.join(mountPoint, '.background', path.basename(bgFile));
    const volDiskName = path.basename(mountPoint);
    const appleScript = `
      tell application "Finder"
        tell disk "${volDiskName.replaceAll('"', '\\"')}"
          open
          set current view of container window to icon view
          set toolbar visible of container window to false
          set statusbar visible of container window to false
          set the bounds of container window to {140, 140, 940, 668}
          set viewOptions to the icon view options of container window
          set arrangement of viewOptions to not arranged
          set icon size of viewOptions to 104
          set text size of viewOptions to 13
          set background color of viewOptions to {0, 0, 0}
          set background picture of viewOptions to POSIX file "${bgPosix.replaceAll('"', '\\"')}"
          set position of item "${appBundleName}" of container window to {200, 240}
          set position of item "Applications" of container window to {600, 240}
          close
          open
          update without registering applications
          delay 2
        end tell
      end tell
    `;
    run('osascript', ['-e', appleScript]);
    spawnSync('sync', []);
  } finally {
    spawnSync('hdiutil', ['detach', mountPoint, '-quiet']);
  }

  run('hdiutil', ['convert', rwDmg, '-format', 'UDZO', '-imagekey', 'zlib-level=9', '-o', finalDmg]);
  await fs.rm(rwDmg, { force: true });
}

// Also apply the app icon directly onto the .dmg file itself
spawnSync('osascript', [
  '-l',
  'JavaScript',
  '-e',
  `ObjC.import("AppKit"); const img = $.NSImage.alloc.initWithContentsOfFile(${JSON.stringify(appIconIcns)}); $.NSWorkspace.sharedWorkspace.setIconForFileOptions(img, ${JSON.stringify(finalDmg)}, 0);`,
]);

console.log(`Packaged ${appName} as ${finalDmg}`);
