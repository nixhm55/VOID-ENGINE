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
const desktop = `[Desktop Entry]\nType=Application\nName=${appName.replace(/[\r\n]/g, ' ')}\nExec=${appSlug}\nIcon=${appSlug}\nCategories=Utility;Network;\nTerminal=false\n`;
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
