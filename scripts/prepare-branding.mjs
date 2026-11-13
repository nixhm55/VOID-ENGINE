import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import zlib from 'node:zlib';

const outputDir = 'branding';
const maxIconBytes = 4 * 1024 * 1024;
const maxPageBytes = 2 * 1024 * 1024;
const maxIconCandidates = 64;
const pngSignature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function pngChunk(type, data) {
  const name = Buffer.from(type);
  const chunk = Buffer.alloc(data.length + 12);
  chunk.writeUInt32BE(data.length, 0);
  name.copy(chunk, 4);
  data.copy(chunk, 8);
  chunk.writeUInt32BE(crc32(Buffer.concat([name, data])), data.length + 8);
  return chunk;
}

function encodePng(width, height, rgba) {
  const rows = Buffer.alloc((width * 4 + 1) * height);
  for (let y = 0; y < height; y++) {
    const dst = y * (width * 4 + 1);
    rows[dst] = 0;
    rgba.copy(rows, dst + 1, y * width * 4, (y + 1) * width * 4);
  }
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header[8] = 8;
  header[9] = 6;
  return Buffer.concat([
    pngSignature,
    pngChunk('IHDR', header),
    pngChunk('IDAT', zlib.deflateSync(rows, { level: 9 })),
    pngChunk('IEND', Buffer.alloc(0)),
  ]);
}

function generatedFallbackIcon() {
  const size = 256;
  const rgba = Buffer.alloc(size * size * 4);
  const setPixel = (x, y, color) => {
    const offset = (y * size + x) * 4;
    rgba[offset] = color[0];
    rgba[offset + 1] = color[1];
    rgba[offset + 2] = color[2];
    rgba[offset + 3] = color[3] ?? 255;
  };
  const segmentDistance = (x, y, x1, y1, x2, y2) => {
    const dx = x2 - x1;
    const dy = y2 - y1;
    const length = dx * dx + dy * dy;
    const t = length ? Math.max(0, Math.min(1, ((x - x1) * dx + (y - y1) * dy) / length)) : 0;
    return Math.hypot(x - (x1 + t * dx), y - (y1 + t * dy));
  };
  const background = [24, 29, 40, 255];
  const orange = [245, 133, 25, 255];
  const white = [255, 255, 255, 255];
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const cx = Math.max(48, Math.min(size - 48, x));
      const cy = Math.max(48, Math.min(size - 48, y));
      if (Math.hypot(x - cx, y - cy) <= 48) setPixel(x, y, background);
      const radius = Math.hypot(x - 128, y - 128);
      if (radius >= 61 && radius <= 79) setPixel(x, y, orange);
      if (segmentDistance(x, y, 86, 93, 128, 165) <= 10
          || segmentDistance(x, y, 128, 165, 170, 93) <= 10) setPixel(x, y, white);
    }
  }
  return encodePng(size, size, rgba);
}

function pngDimensions(bytes) {
  if (bytes.length < 24 || !bytes.subarray(0, 8).equals(pngSignature)) return null;
  const width = bytes.readUInt32BE(16);
  const height = bytes.readUInt32BE(20);
  if (!width || !height || width > 2048 || height > 2048 || bytes.length > maxIconBytes) return null;
  return { width, height };
}

function icoToPng(bytes) {
  if (bytes.length < 22 || bytes.readUInt16LE(0) !== 0 || bytes.readUInt16LE(2) !== 1) return null;
  const count = bytes.readUInt16LE(4);
  const entries = [];
  for (let index = 0; index < count; index++) {
    const at = 6 + index * 16;
    if (at + 16 > bytes.length) break;
    const width = bytes[at] || 256;
    const height = bytes[at + 1] || 256;
    entries.push({ width, height, bits: bytes.readUInt16LE(at + 6), size: bytes.readUInt32LE(at + 8), offset: bytes.readUInt32LE(at + 12) });
  }
  entries.sort((a, b) => b.width * b.height - a.width * a.height || b.bits - a.bits);
  for (const entry of entries) {
    const image = bytes.subarray(entry.offset, entry.offset + entry.size);
    if (pngDimensions(image)) return image;
    if (entry.offset + entry.size > bytes.length || entry.size < 40) continue;
    const headerSize = image.readUInt32LE(0);
    if (headerSize < 40 || headerSize > image.length) continue;
    const width = Math.abs(image.readInt32LE(4));
    const doubledHeight = Math.abs(image.readInt32LE(8));
    const height = Math.floor(doubledHeight / 2);
    const bits = image.readUInt16LE(14);
    if (!width || !height || width > 2048 || height > 2048 || ![24, 32].includes(bits)) continue;
    if (image.readUInt32LE(16) !== 0) continue;
    const bytesPerPixel = bits / 8;
    const xorStride = Math.floor((width * bits + 31) / 32) * 4;
    const andStride = Math.floor((width + 31) / 32) * 4;
    const xorStart = headerSize;
    if (xorStart + xorStride * height > image.length) continue;
    const andStart = xorStart + xorStride * height;
    const rgba = Buffer.alloc(width * height * 4);
    let hasAlpha = false;
    for (let y = 0; y < height; y++) {
      const sourceY = height - 1 - y;
      for (let x = 0; x < width; x++) {
        const src = xorStart + sourceY * xorStride + x * bytesPerPixel;
        const dst = (y * width + x) * 4;
        rgba[dst] = image[src + 2];
        rgba[dst + 1] = image[src + 1];
        rgba[dst + 2] = image[src];
        const alpha = bits === 32 ? image[src + 3] : 255;
        hasAlpha ||= alpha !== 0;
        rgba[dst + 3] = alpha;
      }
    }
    for (let y = 0; y < height; y++) {
      const sourceY = height - 1 - y;
      for (let x = 0; x < width; x++) {
        const dst = (y * width + x) * 4 + 3;
        const maskOffset = andStart + sourceY * andStride + (x >> 3);
        const masked = maskOffset < image.length && ((image[maskOffset] >> (7 - (x & 7))) & 1) !== 0;
        if (masked) rgba[dst] = 0;
        else if (!hasAlpha) rgba[dst] = 255;
      }
    }
    return encodePng(width, height, rgba);
  }
  return null;
}

function isSvg(bytes, candidate) {
  const mime = (candidate.type || '').toLowerCase();
  const pathname = new URL(candidate.url).pathname.toLowerCase();
  return mime.includes('svg') || pathname.endsWith('.svg')
    || /^\s*(?:<\?xml\b[^?]*\?>\s*)?(?:<!--[\s\S]*?-->\s*)*<svg\b/i.test(bytes.toString('utf8', 0, 512));
}

function isSafeSvg(bytes) {
  const svg = bytes.toString('utf8');
  return /^\s*(?:<\?xml\b[^?]*\?>\s*)?(?:<!--[\s\S]*?-->\s*)*<svg\b/i.test(svg)
    && !/<!DOCTYPE|<!ENTITY|<\s*(?:script|foreignObject|iframe|object|embed)\b/i.test(svg)
    && !/\b(?:href|src)\s*=\s*["']\s*(?!#)[^"']*["']/i.test(svg)
    && !/\b(?:on[a-z0-9_-]+)\s*=/i.test(svg)
    && !/@import\b/i.test(svg)
    && !/url\(\s*(?!#)[^)]+\)/i.test(svg);
}

function imageExtension(candidate, bytes) {
  const mime = (candidate.type || '').toLowerCase().split(';', 1)[0].trim();
  const mimeExtensions = {
    'image/avif': '.avif',
    'image/bmp': '.bmp',
    'image/gif': '.gif',
    'image/jpeg': '.jpg',
    'image/png': '.png',
    'image/svg+xml': '.svg',
    'image/vnd.microsoft.icon': '.ico',
    'image/webp': '.webp',
    'image/x-icon': '.ico',
  };
  if (mimeExtensions[mime]) return mimeExtensions[mime];
  const pathname = new URL(candidate.url).pathname;
  const extension = path.extname(pathname).toLowerCase();
  if (/^\.(?:avif|bmp|gif|ico|jpe?g|png|svg|webp)$/.test(extension)) return extension;
  if (bytes.subarray(0, 4).toString() === 'RIFF' && bytes.subarray(8, 12).toString() === 'WEBP') return '.webp';
  if (bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) return '.jpg';
  if (isSvg(bytes, candidate)) return '.svg';
  return '.img';
}

function convertImageToPng(bytes, candidate) {
  if (pngDimensions(bytes)) return bytes;
  const ico = icoToPng(bytes);
  if (ico) return ico;
  if (isSvg(bytes, candidate) && !isSafeSvg(bytes)) {
    throw new Error('SVG icon contains unsupported or external content');
  }

  const temporaryDirectory = fs.mkdtempSync(path.join(os.tmpdir(), 'void-image-icon-'));
  const source = path.join(temporaryDirectory, `icon${imageExtension(candidate, bytes)}`);
  const output = path.join(temporaryDirectory, 'icon.png');
  try {
    fs.writeFileSync(source, bytes);
    const converters = [];
    if (process.platform === 'darwin') {
      converters.push(['sips', ['-s', 'format', 'png', source, '--out', output]]);
    }
    converters.push(['magick', [source, `png:${output}`]]);
    if (process.platform !== 'win32') converters.push(['convert', [source, output]]);
    converters.push(['ffmpeg', ['-v', 'error', '-i', source, '-frames:v', '1', output]]);
    for (const [command, args] of converters) {
      const result = spawnSync(command, args, {
        encoding: 'utf8',
        timeout: 10000,
        maxBuffer: 1024 * 1024,
      });
      if (result.error || result.status !== 0) continue;
      try {
        const png = fs.readFileSync(output);
        if (pngDimensions(png)) return png;
      } catch {
        // Try the next available decoder.
      }
      fs.rmSync(output, { force: true });
    }
    throw new Error(`No installed image decoder could convert ${candidate.url}`);
  } finally {
    fs.rmSync(temporaryDirectory, { recursive: true, force: true });
  }
}

async function readBoundedResponse(response, limit, label) {
  const declaredLength = Number(response.headers.get('content-length') || 0);
  if (declaredLength > limit) throw new Error(`${label} exceeds the ${limit}-byte size limit`);
  if (!response.body) return Buffer.alloc(0);

  const reader = response.body.getReader();
  const chunks = [];
  let total = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > limit) {
      await reader.cancel();
      throw new Error(`${label} exceeds the ${limit}-byte size limit`);
    }
    chunks.push(Buffer.from(value));
  }
  return Buffer.concat(chunks, total);
}

async function fetchBytes(url) {
  const response = await fetch(url, {
    redirect: 'follow',
    signal: AbortSignal.timeout(8000),
    headers: { 'User-Agent': 'Mozilla/5.0 (compatible; VOID icon fetcher)', Accept: 'image/png,image/x-icon,image/*;q=0.8,*/*;q=0.2' },
  });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  return readBoundedResponse(response, maxIconBytes, 'Icon');
}

function parseTagAttributes(tag) {
  const attrs = {};
  for (const match of tag.matchAll(/([\w:-]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))/gi)) {
    attrs[match[1].toLowerCase()] = match[2] ?? match[3] ?? match[4];
  }
  return attrs;
}

function iconArea(sizes) {
  const entries = sizes.match(/\d+\s*x\s*\d+/gi) || [];
  return entries.reduce((largest, size) => {
    const [width, height] = size.toLowerCase().split('x').map(Number);
    return Math.max(largest, width * height);
  }, 0);
}

function resolveImageUrl(value, baseUrl, area = 0) {
  try {
    const url = new URL(value, baseUrl);
    if (url.protocol !== 'https:' && url.protocol !== 'http:') return null;
    return { url: url.href, area };
  } catch {
    return null;
  }
}

function parseIconLinks(html, pageUrl) {
  const links = [];
  for (const match of html.matchAll(/<link\b[^>]*>/gi)) {
    const attrs = parseTagAttributes(match[0]);
    const rels = (attrs.rel || '').toLowerCase().split(/\s+/);
    const isAppleIcon = rels.some((rel) => rel.startsWith('apple-touch-icon'));
    if (!rels.some((rel) => ['icon', 'shortcut', 'mask-icon', 'fluid-icon'].includes(rel)) && !isAppleIcon) continue;
    if (!attrs.href) continue;
    const candidate = resolveImageUrl(attrs.href, pageUrl, iconArea(attrs.sizes || ''));
    if (candidate) links.push({ ...candidate, source: 'page', apple: isAppleIcon ? 1 : 0, type: attrs.type || '' });
  }
  return links;
}

function parseManifestUrls(html, pageUrl) {
  const manifests = [];
  for (const match of html.matchAll(/<link\b[^>]*>/gi)) {
    const attrs = parseTagAttributes(match[0]);
    const rels = (attrs.rel || '').toLowerCase().split(/\s+/);
    if (!rels.includes('manifest') || !attrs.href) continue;
    const candidate = resolveImageUrl(attrs.href, pageUrl);
    if (candidate) manifests.push(candidate.url);
  }
  return [...new Set(manifests)];
}

async function fetchManifestIcons(manifestUrl) {
  try {
    const response = await fetch(manifestUrl, {
      redirect: 'follow',
      signal: AbortSignal.timeout(8000),
      headers: { 'User-Agent': 'Mozilla/5.0', Accept: 'application/manifest+json,application/json;q=0.9,*/*;q=0.2' },
    });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const text = (await readBoundedResponse(response, maxPageBytes, 'Manifest')).toString('utf8');
    const manifest = JSON.parse(text);
    return (Array.isArray(manifest.icons) ? manifest.icons : [])
      .filter((icon) => icon && typeof icon.src === 'string')
      .map((icon) => {
        const candidate = resolveImageUrl(icon.src, response.url || manifestUrl, iconArea(icon.sizes || ''));
        return candidate ? { ...candidate, source: 'manifest', apple: 0, type: icon.type || '' } : null;
      })
      .filter(Boolean);
  } catch (error) {
    console.warn(`Could not inspect the target site's icon manifest: ${error.message}`);
    return [];
  }
}

async function discoverIcon() {
  const explicit = (process.env.ICON_URL || '').trim();
  const site = (process.env.TARGET_URL || process.env.APP_URL || '').trim();
  const candidates = [];
  if (explicit) candidates.push({ url: explicit, area: 0, apple: 0, png: 0, explicit: true });
  let initialPageUrl = '';
  let resolvedPageUrl = '';
  if (site) {
    try {
      initialPageUrl = new URL(site.includes('://') ? site : `https://${site}`).href;
      if (!['http:', 'https:'].includes(new URL(initialPageUrl).protocol)) {
        throw new Error('Target URL must use HTTP or HTTPS');
      }
      resolvedPageUrl = initialPageUrl;
      const response = await fetch(initialPageUrl, {
        signal: AbortSignal.timeout(8000),
        headers: { 'User-Agent': 'Mozilla/5.0', Accept: 'text/html,application/xhtml+xml,*/*;q=0.2' },
      });
      if (response.ok) {
        const html = (await readBoundedResponse(response, maxPageBytes, 'Target page')).toString('utf8');
        resolvedPageUrl = response.url || initialPageUrl;
        candidates.push(...parseIconLinks(html, resolvedPageUrl));
        const manifestUrls = parseManifestUrls(html, resolvedPageUrl);
        for (const manifestUrl of manifestUrls) {
          candidates.push(...await fetchManifestIcons(manifestUrl));
        }
      }
    } catch (error) {
      console.warn(`Could not inspect the target page for an icon: ${error.message}`);
    }
  }
  for (const pageUrl of new Set([resolvedPageUrl, initialPageUrl].filter(Boolean))) {
    const origin = new URL(pageUrl).origin;
    candidates.push(
      { url: `${origin}/apple-touch-icon.png`, area: 180 * 180, apple: 1, source: 'site fallback' },
      { url: `${origin}/apple-touch-icon-precomposed.png`, area: 180 * 180, apple: 1, source: 'site fallback' },
      { url: `${origin}/favicon.ico`, area: 0, apple: 0, source: 'site fallback' },
      { url: `${origin}/favicon.png`, area: 0, apple: 0, source: 'site fallback' },
      { url: `${origin}/favicon.svg`, area: 0, apple: 0, source: 'site fallback', type: 'image/svg+xml' },
    );
  }

  const uniqueByUrl = new Map();
  for (const candidate of candidates) {
    const previous = uniqueByUrl.get(candidate.url);
    if (!previous) uniqueByUrl.set(candidate.url, { ...candidate });
    else {
      previous.area = Math.max(previous.area || 0, candidate.area || 0);
      previous.apple = Math.max(previous.apple || 0, candidate.apple || 0);
      previous.type ||= candidate.type;
      previous.explicit ||= candidate.explicit;
    }
  }
  const explicitCandidate = [...uniqueByUrl.values()].find((candidate) => candidate.explicit);
  const tryCandidate = async (candidate) => {
    try {
      const bytes = await fetchBytes(candidate.url);
      const png = convertImageToPng(bytes, candidate);
      const dimensions = png && pngDimensions(png);
      return dimensions
        ? { ...candidate, png, dimensions, size: Math.min(dimensions.width, dimensions.height) }
        : null;
    } catch (error) {
      console.warn(`Skipping app icon ${candidate.url}: ${error.message}`);
      return null;
    }
  };

  if (explicitCandidate) {
    const explicitIcon = await tryCandidate(explicitCandidate);
    if (explicitIcon) {
      console.log(`Using app icon from ${explicitIcon.url} (${explicitIcon.dimensions.width}x${explicitIcon.dimensions.height})`);
      return explicitIcon.png;
    }
  }

  const siteCandidates = [...uniqueByUrl.values()]
    .filter((candidate) => !candidate.explicit)
    .sort((a, b) => (b.apple || 0) - (a.apple || 0) || b.area - a.area)
    .slice(0, maxIconCandidates);
  const found = [];
  let nextCandidate = 0;
  await Promise.all(Array.from({ length: Math.min(4, siteCandidates.length) }, async () => {
    while (nextCandidate < siteCandidates.length) {
      const candidate = siteCandidates[nextCandidate++];
      const icon = await tryCandidate(candidate);
      if (icon) found.push(icon);
    }
  }));
  found.sort((a, b) => b.size - a.size || (b.apple || 0) - (a.apple || 0) || b.area - a.area);
  if (found.length) {
    const best = found[0];
    console.log(`Using app icon from ${best.url} (${best.dimensions.width}x${best.dimensions.height})`);
    return best.png;
  }
  console.warn('No supported site icon was found; using the generated VOID fallback icon.');
  return generatedFallbackIcon();
}

fs.mkdirSync(outputDir, { recursive: true });
const png = await discoverIcon();
fs.writeFileSync(path.join(outputDir, 'icon.png'), png);

const androidRes = path.join(outputDir, 'android-res');
fs.rmSync(androidRes, { recursive: true, force: true });
const androidIconDirectory = path.join(androidRes, 'mipmap-xxxhdpi');
fs.mkdirSync(androidIconDirectory, { recursive: true });
fs.writeFileSync(path.join(androidIconDirectory, 'ic_launcher.png'), png);

const ico = Buffer.alloc(22 + png.length);
ico.writeUInt16LE(0, 0);
ico.writeUInt16LE(1, 2);
ico.writeUInt16LE(1, 4);
ico[6] = 0;
ico[7] = 0;
ico.writeUInt16LE(1, 10);
ico.writeUInt16LE(32, 12);
ico.writeUInt32LE(png.length, 14);
ico.writeUInt32LE(22, 18);
png.copy(ico, 22);
fs.writeFileSync(path.join(outputDir, 'app.ico'), ico);
