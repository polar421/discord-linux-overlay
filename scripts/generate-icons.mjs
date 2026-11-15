#!/usr/bin/env node
/**
 * Generates the application icons without any image tooling or dependencies.
 *
 * The mark is a rounded blurple square with three white voice bars and a green
 * "speaking" dot — deliberately distinct from Overlayed's artwork.
 *
 * Usage: node scripts/generate-icons.mjs
 */
import { deflateSync } from "node:zlib";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

const crcTable = new Uint32Array(256);
for (let n = 0; n < 256; n += 1) {
  let c = n;
  for (let k = 0; k < 8; k += 1) {
    c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  }
  crcTable[n] = c >>> 0;
}

function crc32(buffer) {
  let c = 0xffffffff;
  for (const byte of buffer) {
    c = crcTable[(c ^ byte) & 0xff] ^ (c >>> 8);
  }
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const typeBuffer = Buffer.from(type, "ascii");
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([typeBuffer, data])));
  return Buffer.concat([length, typeBuffer, data, crc]);
}

function encodePng(size, rgba) {
  const stride = size * 4 + 1;
  const raw = Buffer.alloc(stride * size);
  for (let y = 0; y < size; y += 1) {
    raw[y * stride] = 0; // no filter
    rgba.copy(raw, y * stride + 1, y * size * 4, (y + 1) * size * 4);
  }

  const header = Buffer.alloc(13);
  header.writeUInt32BE(size, 0);
  header.writeUInt32BE(size, 4);
  header[8] = 8; // bit depth
  header[9] = 6; // RGBA
  header[10] = 0;
  header[11] = 0;
  header[12] = 0;

  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", header),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

function insideRoundedRect(x, y, left, top, right, bottom, radius) {
  if (x < left || x > right || y < top || y > bottom) return false;
  const nearLeft = x < left + radius;
  const nearRight = x > right - radius;
  const nearTop = y < top + radius;
  const nearBottom = y > bottom - radius;
  if (!(nearLeft || nearRight) || !(nearTop || nearBottom)) return true;

  const cx = nearLeft ? left + radius : right - radius;
  const cy = nearTop ? top + radius : bottom - radius;
  return Math.hypot(x - cx, y - cy) <= radius;
}

const lerp = (a, b, t) => a + (b - a) * t;

// sRGB colours (0-255).
const TOP = [88, 101, 242]; // blurple
const BOTTOM = [124, 58, 237]; // violet
const GREEN = [35, 165, 90];

function shade(x, y) {
  const inset = 0.03;
  const radius = 0.24;

  if (!insideRoundedRect(x, y, inset, inset, 1 - inset, 1 - inset, radius)) {
    return [0, 0, 0, 0];
  }

  const t = (x + y) / 2;
  const r = lerp(TOP[0], BOTTOM[0], t);
  const g = lerp(TOP[1], BOTTOM[1], t);
  const b = lerp(TOP[2], BOTTOM[2], t);

  // Three voice bars in the centre.
  const bars = [
    { cx: 0.34, halfHeight: 0.13 },
    { cx: 0.5, halfHeight: 0.21 },
    { cx: 0.66, halfHeight: 0.16 },
  ];
  for (const bar of bars) {
    const width = 0.052;
    const top = 0.5 - bar.halfHeight;
    const bottom = 0.5 + bar.halfHeight;
    if (
      insideRoundedRect(
        x,
        y,
        bar.cx - width,
        top,
        bar.cx + width,
        bottom,
        width,
      )
    ) {
      return [255, 255, 255, 255];
    }
  }

  // Speaking dot, bottom-right.
  if (Math.hypot(x - 0.79, y - 0.79) <= 0.085) {
    if (Math.hypot(x - 0.79, y - 0.79) <= 0.055) {
      return [...GREEN, 255];
    }
    return [...TOP, 255];
  }

  return [r, g, b, 255];
}

function render(size) {
  const supersample = 4;
  const buffer = Buffer.alloc(size * size * 4);

  for (let py = 0; py < size; py += 1) {
    for (let px = 0; px < size; px += 1) {
      let r = 0;
      let g = 0;
      let b = 0;
      let a = 0;

      for (let sy = 0; sy < supersample; sy += 1) {
        for (let sx = 0; sx < supersample; sx += 1) {
          const x = (px + (sx + 0.5) / supersample) / size;
          const y = (py + (sy + 0.5) / supersample) / size;
          const [sr, sg, sb, sa] = shade(x, y);
          const alpha = sa / 255;
          r += sr * alpha;
          g += sg * alpha;
          b += sb * alpha;
          a += sa;
        }
      }

      const samples = supersample * supersample;
      const alphaAverage = a / samples;
      const alpha = alphaAverage / 255;
      const index = (py * size + px) * 4;

      buffer[index] = alpha > 0 ? Math.round(r / samples / alpha) : 0;
      buffer[index + 1] = alpha > 0 ? Math.round(g / samples / alpha) : 0;
      buffer[index + 2] = alpha > 0 ? Math.round(b / samples / alpha) : 0;
      buffer[index + 3] = Math.round(alphaAverage);
    }
  }

  return encodePng(size, buffer);
}

const outputs = [
  ["src-tauri/icons/32x32.png", 32],
  ["src-tauri/icons/128x128.png", 128],
  ["src-tauri/icons/128x128@2x.png", 256],
  ["src-tauri/icons/icon.png", 512],
  ["packaging/icons/32x32/io.github.polar421.DiscordLinuxOverlay.png", 32],
  ["packaging/icons/64x64/io.github.polar421.DiscordLinuxOverlay.png", 64],
  ["packaging/icons/128x128/io.github.polar421.DiscordLinuxOverlay.png", 128],
  ["packaging/icons/256x256/io.github.polar421.DiscordLinuxOverlay.png", 256],
];

for (const [relativePath, size] of outputs) {
  const target = join(root, relativePath);
  mkdirSync(dirname(target), { recursive: true });
  writeFileSync(target, render(size));
  console.log(`wrote ${relativePath} (${size}x${size})`);
}

const logo = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" width="64" height="64">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#5865f2" />
      <stop offset="1" stop-color="#7c3aed" />
    </linearGradient>
  </defs>
  <rect x="2" y="2" width="60" height="60" rx="15" fill="url(#bg)" />
  <g fill="#fff">
    <rect x="18" y="24" width="7" height="16" rx="3.5" />
    <rect x="28.5" y="18" width="7" height="28" rx="3.5" />
    <rect x="39" y="22" width="7" height="20" rx="3.5" />
  </g>
  <circle cx="50" cy="50" r="5.5" fill="#23a55a" stroke="#5865f2" stroke-width="2" />
</svg>
`;

mkdirSync(join(root, "assets"), { recursive: true });
writeFileSync(join(root, "assets/logo.svg"), logo);
console.log("wrote assets/logo.svg");
