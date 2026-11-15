import fs from 'node:fs';
import http from 'node:http';
import https from 'node:https';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import zlib from 'node:zlib';

const outputDir = 'branding';
const maxIconBytes = 4 * 1024 * 1024;
const maxPageBytes = 2 * 1024 * 1024;
const maxIconCandidates = 64;
const pngSignature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
// Sites behind an edge/CDN (Google included) serve a reduced document to
// unknown clients, so discovery has to look like a real browser request.
const BROWSER_USER_AGENT = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36';
const PAGE_ACCEPT = 'text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8';
const ICON_ACCEPT = 'image/avif,image/webp,image/svg+xml,image/png,image/*;q=0.8,*/*;q=0.2';
const FETCH_ATTEMPTS = 3;
// undici's 16 KiB response-header ceiling is what broke discovery; 64 KiB
// covers the cookie-heavy edges (Google, Cloudflare Access) with headroom.
const maxResponseHeaderBytes = 64 * 1024;

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

// Glossy black plate with a matte, carved-in "V". Everything is drawn from
// signed distance fields so the result stays sharp at any export size and needs
// no fonts, SVG rasterizer, or Canvas implementation at build time.
function generatedFallbackIcon() {
  const size = 1024;
  const rgba = Buffer.alloc(size * size * 4);
  const half = size / 2;
  const plateRadius = size * 0.226;
  const clamp01 = (value) => (value < 0 ? 0 : value > 1 ? 1 : value);
  const smooth = (edge0, edge1, value) => {
    const t = clamp01((value - edge0) / (edge1 - edge0));
    return t * t * (3 - 2 * t);
  };

  const plateDistance = (x, y) => {
    const qx = Math.abs(x - half) - (half - plateRadius);
    const qy = Math.abs(y - half) - (half - plateRadius);
    return Math.hypot(Math.max(qx, 0), Math.max(qy, 0))
      + Math.min(Math.max(qx, qy), 0) - plateRadius;
  };

  // The V is two overlapping capsules; the overlap rounds the vertex naturally.
  const strokes = [
    [0.344, 0.289, 0.5, 0.713],
    [0.656, 0.289, 0.5, 0.713],
  ].map(([x1, y1, x2, y2]) => [x1 * size, y1 * size, x2 * size, y2 * size]);
  const strokeHalfWidth = size * 0.057;
  const letterDistance = (x, y) => {
    let nearest = null;
    for (const [x1, y1, x2, y2] of strokes) {
      const dx = x2 - x1;
      const dy = y2 - y1;
      const lengthSquared = dx * dx + dy * dy;
      const t = lengthSquared ? clamp01(((x - x1) * dx + (y - y1) * dy) / lengthSquared) : 0;
      const closestX = x1 + t * dx;
      const closestY = y1 + t * dy;
      const distance = Math.hypot(x - closestX, y - closestY);
      if (!nearest || distance < nearest.distance) {
        nearest = {
          distance,
          normalY: distance ? (y - closestY) / distance : 0,
        };
      }
    }
    return nearest;
  };

  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const plate = plateDistance(x, y);
      const coverage = smooth(1.2, -1.2, plate);
      if (coverage <= 0) continue;

      // Glossy base: off-centre sheen near the top left, a vertical fade, and
      // a soft vignette so the plate reads as glass rather than flat grey.
      const sheenX = (x - size * 0.32) / size;
      const sheenY = (y - size * 0.16) / size;
      const sheen = Math.exp(-(sheenX * sheenX + sheenY * sheenY) * 9) * 0.19;
      const topLight = (1 - y / size) * 0.055;
      const vignette = 1 - 0.22 * smooth(0.55, 1.05, Math.hypot(x - half, y - half) / (size * 0.62));
      let value = (0.035 + sheen + topLight) * vignette;
      value += Math.exp(-Math.abs(plate) / 2) * 0.07;

      const { distance, normalY } = letterDistance(x, y);
      const inside = strokeHalfWidth - distance;
      if (inside > -3) {
        const letter = smooth(1.5, -1.5, -inside);
        // Matte fill: almost no gradient, then a carved wall. The upper wall
        // falls into shadow and the lower wall catches a little bounce light,
        // which is what makes the letter look recessed instead of painted on.
        const depth = Math.max(0, inside);
        const wall = 1 - Math.exp(-depth / 18);
        const matte = 0.052 + 0.008 * (y / size)
          - wall * Math.max(0, -normalY) * 0.62
          + wall * Math.max(0, normalY) * 0.16;
        value = value * (1 - letter) + Math.max(0, matte) * letter;
        const bevel = Math.exp(-Math.abs(inside) / 2.2) * (1 - letter);
        value += bevel * (0.10 * Math.max(0, normalY) - 0.12 * Math.max(0, -normalY));
      }

      const offset = (y * size + x) * 4;
      const channel = Math.round(clamp01(value) * 255);
      rgba[offset] = channel;
      rgba[offset + 1] = channel;
      rgba[offset + 2] = channel;
      rgba[offset + 3] = Math.round(coverage * 255);
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
  if (bytes.subarray(0, 4).toString() === 'GIF8') return '.gif';
  if (bytes[0] === 0x42 && bytes[1] === 0x4d) return '.bmp';
  if (isSvg(bytes, candidate)) return '.svg';
  return '.img';
}

// Quick Look and the other rasterizers honour an SVG's intrinsic size, so a
// 48px logo would land as a 48px app icon. Pin the root width/height (the
// viewBox keeps the artwork proportional) before the vector is rendered.
function inflateSvgIntrinsicSize(markup, size) {
  const root = markup.match(/<svg\b[^>]*>/i);
  if (!root) return markup;
  const tag = root[0]
    .replace(/\swidth\s*=\s*(?:"[^"]*"|'[^']*')/i, '')
    .replace(/\sheight\s*=\s*(?:"[^"]*"|'[^']*')/i, '')
    .replace(/^<svg\b/i, `<svg width="${size}" height="${size}"`);
  return markup.slice(0, root.index) + tag + markup.slice(root.index + root[0].length);
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
    fs.writeFileSync(
      source,
      isSvg(bytes, candidate)
        ? Buffer.from(inflateSvgIntrinsicSize(bytes.toString('utf8'), 1024), 'utf8')
        : bytes,
    );
    // Each entry is [command, args, produced file]. `sips` cannot read SVG, so
    // qlmanage (Quick Look, present on every macOS runner), rsvg-convert and
    // Inkscape cover scalable icons before the raster-only decoders.
    const converters = [];
    if (process.platform === 'darwin') {
      converters.push(['sips', ['-s', 'format', 'png', source, '--out', output], output]);
      converters.push(['qlmanage', ['-t', '-s', '1024', '-o', temporaryDirectory, source], `${source}.png`]);
    }
    converters.push(['rsvg-convert', ['-w', '1024', '-h', '1024', '-o', output, source], output]);
    converters.push(['inkscape', [source, '--export-type=png', `--export-filename=${output}`, '--export-width=1024'], output]);
    converters.push(['magick', [source, `png:${output}`], output]);
    if (process.platform !== 'win32') converters.push(['convert', [source, output], output]);
    converters.push(['ffmpeg', ['-v', 'error', '-i', source, '-frames:v', '1', output], output]);
    for (const [command, args, produced] of converters) {
      const result = spawnSync(command, args, {
        encoding: 'utf8',
        timeout: 10000,
        maxBuffer: 1024 * 1024,
      });
      if (result.error || result.status !== 0) continue;
      try {
        const png = fs.readFileSync(produced);
        if (pngDimensions(png)) return png;
      } catch {
        // Try the next available decoder.
      }
      fs.rmSync(produced, { force: true });
    }
    throw new Error(`No installed image decoder could convert ${candidate.url}`);
  } finally {
    fs.rmSync(temporaryDirectory, { recursive: true, force: true });
  }
}

function decodeBody(body, encoding) {
  if (!body.length) return body;
  try {
    if (encoding === 'gzip') return zlib.gunzipSync(body);
    if (encoding === 'deflate') return zlib.inflateSync(body);
    if (encoding === 'br') return zlib.brotliDecompressSync(body);
    if (encoding === 'zstd' && typeof zlib.zstdDecompressSync === 'function') return zlib.zstdDecompressSync(body);
  } catch {
    // Fall back to the raw bytes; a decoder failure is not fatal here.
  }
  return body;
}

// Node's global fetch (undici) rejects any response whose header block exceeds
// 16 KiB. Authenticated edges such as Google's reply with a much larger
// Set-Cookie block, so `fetch` threw UND_ERR_HEADERS_OVERFLOW and the page was
// never inspected at all. node:https raises that ceiling, and keeps redirects,
// compression and the size limit in one place.
function requestBytes(url, headers, limit, label, redirectsLeft = 5) {
  return new Promise((resolve, reject) => {
    let target;
    try {
      target = new URL(url);
    } catch (error) {
      reject(error);
      return;
    }
    const transport = target.protocol === 'http:' ? http : https;
    const request = transport.get(target, { headers, maxHeaderSize: maxResponseHeaderBytes, timeout: 8000 }, (response) => {
      const status = response.statusCode || 0;
      const location = response.headers.location;
      if (location && status >= 300 && status < 400 && redirectsLeft > 0) {
        response.resume();
        resolve(requestBytes(new URL(location, target).href, headers, limit, label, redirectsLeft - 1));
        return;
      }
      const declaredLength = Number(response.headers['content-length'] || 0);
      if (declaredLength > limit) {
        response.destroy();
        reject(new Error(`${label} exceeds the ${limit}-byte size limit`));
        return;
      }
      const chunks = [];
      let total = 0;
      response.on('data', (chunk) => {
        total += chunk.length;
        if (total > limit) {
          response.destroy();
          reject(new Error(`${label} exceeds the ${limit}-byte size limit`));
          return;
        }
        chunks.push(chunk);
      });
      response.on('end', () => resolve({
        status,
        url: target.href,
        body: decodeBody(Buffer.concat(chunks, total), String(response.headers['content-encoding'] || '').toLowerCase()),
      }));
      response.on('error', reject);
    });
    request.on('timeout', () => request.destroy(new Error(`${label} request timed out`)));
    request.on('error', reject);
  });
}

async function fetchWithRetry(url, { accept, referer, limit = maxIconBytes, label = 'Resource' } = {}) {
  const headers = {
    'User-Agent': BROWSER_USER_AGENT,
    Accept: accept || ICON_ACCEPT,
    'Accept-Language': 'en-US,en;q=0.9',
  };
  if (referer) headers.Referer = referer;
  let lastError;
  for (let attempt = 1; attempt <= FETCH_ATTEMPTS; attempt++) {
    try {
      const result = await requestBytes(url, headers, limit, label);
      if (result.status < 200 || result.status >= 300) throw new Error(`HTTP ${result.status}`);
      return result;
    } catch (error) {
      lastError = error;
      if (attempt < FETCH_ATTEMPTS) await new Promise((resolve) => setTimeout(resolve, 300 * attempt));
    }
  }
  throw lastError;
}

async function fetchBytes(url, referer) {
  const result = await fetchWithRetry(url, { accept: ICON_ACCEPT, referer, label: 'Icon' });
  return result.body;
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
    // `sizes="any"` marks a scalable icon: rank it like a 512px asset instead
    // of letting it sort behind every raster favicon.
    const declared = (attrs.sizes || '').trim().toLowerCase();
    const area = declared === 'any' ? 512 * 512 : iconArea(declared);
    const candidate = resolveImageUrl(attrs.href, pageUrl, area);
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

async function fetchPage(url) {
  const result = await fetchWithRetry(url, { accept: PAGE_ACCEPT, limit: maxPageBytes, label: 'Target page' });
  return { text: result.body.toString('utf8'), url: result.url };
}

// Client-rendered apps build their <link> tags at runtime, but the URLs usually
// travel in the bootstrap payload (inline web manifests, Next.js props, Angular
// transfer state). Reading those keeps an SPA from looking icon-less.
function parseEmbeddedIconUrls(html, pageUrl) {
  const found = [];
  for (const block of html.matchAll(/["']icons["']\s*:\s*\[([\s\S]{0,4000}?)\]/gi)) {
    for (const src of block[1].matchAll(/["']src["']\s*:\s*["']([^"']+)["']/gi)) {
      const candidate = resolveImageUrl(src[1], pageUrl, 0);
      if (candidate) found.push({ ...candidate, source: 'embedded', apple: 0, type: '' });
    }
  }
  return found;
}

async function fetchManifestIcons(manifestUrl) {
  try {
    const result = await fetchWithRetry(manifestUrl, {
      accept: 'application/manifest+json,application/json;q=0.9,*/*;q=0.2',
      limit: maxPageBytes,
      label: 'Manifest',
    });
    const manifest = JSON.parse(result.body.toString('utf8'));
    return (Array.isArray(manifest.icons) ? manifest.icons : [])
      .filter((icon) => icon && typeof icon.src === 'string')
      .map((icon) => {
        const candidate = resolveImageUrl(icon.src, result.url || manifestUrl, iconArea(icon.sizes || ''));
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
      const page = await fetchPage(initialPageUrl);
      resolvedPageUrl = page.url;
      candidates.push(...parseIconLinks(page.text, resolvedPageUrl));
      candidates.push(...parseEmbeddedIconUrls(page.text, resolvedPageUrl));
      for (const manifestUrl of parseManifestUrls(page.text, resolvedPageUrl)) {
        candidates.push(...await fetchManifestIcons(manifestUrl));
      }
    } catch (error) {
      console.warn(`Could not inspect the target page for an icon: ${error.message}`);
    }
  }
  for (const pageUrl of new Set([resolvedPageUrl, initialPageUrl].filter(Boolean))) {
    const origin = new URL(pageUrl).origin;
    candidates.push(
      { url: `${origin}/apple-touch-icon.png`, area: 180 * 180, apple: 1, source: 'site fallback' },
      { url: `${origin}/apple-touch-icon-180x180.png`, area: 180 * 180, apple: 1, source: 'site fallback' },
      { url: `${origin}/apple-touch-icon-precomposed.png`, area: 180 * 180, apple: 1, source: 'site fallback' },
      { url: `${origin}/favicon.ico`, area: 0, apple: 0, source: 'site fallback' },
      { url: `${origin}/favicon.png`, area: 0, apple: 0, source: 'site fallback' },
      { url: `${origin}/favicon.svg`, area: 512 * 512, apple: 0, source: 'site fallback', type: 'image/svg+xml' },
      { url: `${origin}/icon.svg`, area: 512 * 512, apple: 0, source: 'site fallback', type: 'image/svg+xml' },
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
  const referer = resolvedPageUrl || initialPageUrl || undefined;
  const tryCandidate = async (candidate) => {
    try {
      const bytes = await fetchBytes(candidate.url, referer);
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
