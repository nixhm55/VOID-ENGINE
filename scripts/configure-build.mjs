import fs from 'node:fs';

const inputUrl = (process.env.TARGET_URL || process.env.APP_URL || '').trim();
const configuredName = (process.env.APP_NAME || '').trim();
if (configuredName.length > 80 || /[\u0000-\u001f\u007f]/u.test(configuredName)) {
  throw new Error('APP_NAME must be at most 80 characters and contain no control characters');
}

function titleFromUrl(value) {
  try {
    const hostname = new URL(value.includes('://') ? value : `https://${value}`).hostname
      .replace(/^www\./i, '');
    const segment = hostname.split('.')[0] || 'VOID';
    return segment.replace(/[-_]+/g, ' ').replace(/\b\p{L}/gu, (letter) => letter.toUpperCase());
  } catch {
    return 'VOID';
  }
}

const appName = !configuredName || configuredName.toLowerCase() === 'void'
  ? (inputUrl ? titleFromUrl(inputUrl) : 'VOID')
  : configuredName;
const appSlug = appName.normalize('NFKD').replace(/[^\x00-\x7F]/g, '')
  .toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/^_+|_+$/g, '') || 'void';
const packageSegment = /^[a-z]/.test(appSlug) ? appSlug : `app_${appSlug}`;
const derivedPackage = `com.void.${packageSegment}`;
const requestedPackage = (process.env.ANDROID_PACKAGE || '').trim();
const androidPackage = requestedPackage
  ? requestedPackage.split('.').map((part) => part.toLowerCase()
    .replace(/[^a-z0-9_]/g, '_')).map((part) => /^[a-z]/.test(part) ? part : `app_${part}`).join('.')
  : derivedPackage;
const appVersion = (process.env.APP_VERSION || '1.0.0').trim();

const config = JSON.parse(fs.readFileSync('void.json', 'utf8'));
config.title = appName;
config.url = inputUrl;
config.icon_url = (process.env.ICON_URL || '').trim();
config.hide_title_bar = true;
if (inputUrl) delete config.assets;
config.width = Number(process.env.APP_WIDTH) || config.width || 1200;
config.height = Number(process.env.APP_HEIGHT) || config.height || 780;
config.fullscreen = process.env.APP_FULLSCREEN === 'true';
config.new_window = process.env.APP_NEW_WINDOW === 'true';
fs.writeFileSync('void.json', `${JSON.stringify(config, null, 2)}\n`);
fs.mkdirSync('dist', { recursive: true });
fs.writeFileSync('dist/void.json', `${JSON.stringify(config, null, 2)}\n`);

let manifest = fs.readFileSync('Cargo.toml', 'utf8');
function setManifestValue(table, key, value) {
  const header = `[${table}]`;
  const start = manifest.indexOf(header);
  if (start < 0) throw new Error(`Cargo.toml is missing ${header}`);
  const bodyStart = start + header.length;
  const nextTable = manifest.indexOf('\n[', bodyStart);
  const end = nextTable < 0 ? manifest.length : nextTable + 1;
  const body = manifest.slice(bodyStart, end);
  const escapedKey = key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const fieldPattern = new RegExp(`^(${escapedKey}\\s*=\\s*)`, 'm');
  const match = fieldPattern.exec(body);
  const line = `${key} = ${JSON.stringify(value)}`;
  let nextBody;
  if (!match) {
    nextBody = `${body.trimEnd()}\n${line}\n`;
  } else {
    const valueStart = match.index + match[0].length;
    let valueEnd = body.indexOf('\n', valueStart);
    if (valueEnd < 0) valueEnd = body.length;
    if (body.slice(valueStart).trimStart().startsWith('[')) {
      let depth = 0;
      let quoted = false;
      let escaped = false;
      let started = false;
      for (let index = valueStart; index < body.length; index++) {
        const char = body[index];
        if (quoted) {
          if (escaped) escaped = false;
          else if (char === '\\') escaped = true;
          else if (char === '"') quoted = false;
          continue;
        }
        if (char === '"') quoted = true;
        else if (char === '[') { depth++; started = true; }
        else if (char === ']') {
          depth--;
          if (started && depth === 0) { valueEnd = index + 1; break; }
        }
      }
      if (depth !== 0) throw new Error(`Cargo.toml has an unterminated array for ${key}`);
    }
    nextBody = `${body.slice(0, match.index)}${line}${body.slice(valueEnd)}`;
  }
  manifest = `${manifest.slice(0, bodyStart)}${nextBody}${manifest.slice(end)}`;
}

setManifestValue('package.metadata.bundle', 'name', appName);
setManifestValue('package.metadata.bundle', 'identifier', androidPackage);
setManifestValue('package.metadata.bundle', 'icon', ['branding/icon.png']);
setManifestValue('package.metadata.bundle', 'version', appVersion);
setManifestValue('package.metadata.deb', 'name', appSlug);
setManifestValue('package.metadata.deb', 'assets', [
  ['target/release/void', 'usr/bin/', '755'],
  ['branding/icon.png', 'usr/share/icons/hicolor/256x256/apps/void.png', '644'],
  ['branding/void.desktop', 'usr/share/applications/void.desktop', '644'],
]);
setManifestValue('package.metadata.android', 'package', androidPackage);
setManifestValue('package.metadata.android', 'resources', 'branding/android-res');
setManifestValue('package.metadata.android.application', 'label', appName);
setManifestValue('package.metadata.android.application', 'theme', '@android:style/Theme.Material.NoActionBar');
setManifestValue('package.metadata.android.application', 'icon', '@mipmap/ic_launcher');
fs.writeFileSync('Cargo.toml', manifest);

const desktop = `[Desktop Entry]\nType=Application\nName=${appName.replace(/[\r\n]/g, ' ')}\nExec=void\nIcon=void\nCategories=Utility;Network;\nTerminal=false\n`;
fs.mkdirSync('branding', { recursive: true });
fs.writeFileSync('branding/void.desktop', desktop);

const outputs = [
  `app_name=${appName}`,
  `app_slug=${appSlug}`,
  `android_package=${androidPackage}`,
].join('\n') + '\n';
if (process.env.GITHUB_OUTPUT) fs.appendFileSync(process.env.GITHUB_OUTPUT, outputs);
if (process.env.GITHUB_ENV) fs.appendFileSync(process.env.GITHUB_ENV, outputs);

console.log(`Configured ${appName} (${androidPackage})`);
