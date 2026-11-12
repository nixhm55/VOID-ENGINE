import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';

const outputDir = 'branding';
const maxIconBytes = 256 * 1024;
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

async function fetchBytes(url) {
  const response = await fetch(url, {
    redirect: 'follow',
    signal: AbortSignal.timeout(8000),
    headers: { 'User-Agent': 'Mozilla/5.0 (compatible; VOID icon fetcher)', Accept: 'image/png,image/x-icon,image/*;q=0.8,*/*;q=0.2' },
  });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  const declaredLength = Number(response.headers.get('content-length') || 0);
  if (declaredLength > maxIconBytes) throw new Error('Icon exceeds the 256 KiB size limit');
  const bytes = Buffer.from(await response.arrayBuffer());
  if (bytes.length > maxIconBytes) throw new Error('Icon exceeds the 256 KiB size limit');
  return bytes;
}

function parseIconLinks(html, pageUrl) {
  const links = [];
  for (const match of html.matchAll(/<link\b[^>]*>/gi)) {
    const attrs = {};
    for (const attr of match[0].matchAll(/([\w:-]+)\s*=\s*(["'])(.*?)\2/gi)) attrs[attr[1].toLowerCase()] = attr[3];
    const rel = (attrs.rel || '').toLowerCase();
    if (!/(icon|apple-touch-icon)/.test(rel) || !attrs.href) continue;
    try {
      const url = new URL(attrs.href, pageUrl);
      if (url.protocol === 'https:' || url.protocol === 'http:') {
        links.push({ url: url.href, score: /apple-touch-icon/i.test(rel) ? 1 : 0, png: /png/i.test(attrs.type || '') || /\.png(?:$|\?)/i.test(url.href) ? 0 : 1 });
      }
    } catch {}
  }
  return links.sort((a, b) => a.score - b.score || a.png - b.png).map((item) => item.url);
}

async function discoverIcon() {
  const explicit = (process.env.ICON_URL || '').trim();
  const site = (process.env.TARGET_URL || process.env.APP_URL || '').trim();
  const candidates = [];
  if (explicit) candidates.push(explicit);
  if (site) {
    try {
      const pageUrl = new URL(site.includes('://') ? site : `https://${site}`).href;
      const response = await fetch(pageUrl, { signal: AbortSignal.timeout(8000), headers: { 'User-Agent': 'Mozilla/5.0' } });
      if (response.ok) {
        const html = (await response.text()).slice(0, 2 * 1024 * 1024);
        const resolvedPageUrl = response.url || pageUrl;
        candidates.push(...parseIconLinks(html, resolvedPageUrl));
      }
      const origin = new URL(pageUrl).origin;
      candidates.push(`${origin}/favicon.ico`, `${origin}/favicon.png`, `${origin}/apple-touch-icon.png`);
    } catch (error) {
      console.warn(`Could not inspect the target page for an icon: ${error.message}`);
    }
  }

  for (const candidate of [...new Set(candidates)]) {
    try {
      const bytes = await fetchBytes(candidate);
      const png = pngDimensions(bytes) ? bytes : icoToPng(bytes);
      if (png && pngDimensions(png)) {
        console.log(`Using app icon from ${candidate}`);
        return png;
      }
    } catch (error) {
      console.warn(`Skipping app icon ${candidate}: ${error.message}`);
    }
  }
  throw new Error('No supported PNG/ICO site icon was found. Provide the site icon URL through ICON_URL.');
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
