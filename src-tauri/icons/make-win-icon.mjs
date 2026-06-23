// Regenerates src-tauri/icons/icon.ico — the Windows executable / taskbar icon.
//
// `tauri icon` derives every platform icon (incl. macOS .icns) from one source and
// would overwrite them all; this script touches ONLY icon.ico so the macOS icon is
// left exactly as-is.
//
// Why a Windows-specific render: icon-source.svg keeps Apple's safe-area margin (the
// squircle fills ~80% of the 1024 canvas). On the Windows taskbar that margin sits
// next to full-bleed neighbours, so the icon looks small / cropped and is hard to
// identify. Here we scale the art up to fill the canvas and bake every taskbar size
// as its own frame so each is crisp.
//
// Deps: rsvg-convert (librsvg) on PATH. No npm packages — the .ico is assembled by hand.
// Run:  node src-tauri/icons/make-win-icon.mjs

import { readFileSync, writeFileSync, mkdtempSync, rmSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { join, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';

const dir = dirname(fileURLToPath(import.meta.url));
const src = join(dir, 'icon-source.svg');
const out = join(dir, 'icon.ico');

const WIN_FILL = 1.214; // squircle (824px) -> ~1000px on the 1024 canvas
const sizes = [16, 24, 32, 48, 64, 128, 256];

const inner = readFileSync(src, 'utf8')
  .replace(/^[\s\S]*?<svg[^>]*>/, '')
  .replace(/<\/svg>\s*$/, '');
const winSvg = `<svg width="1024" height="1024" viewBox="0 0 1024 1024" xmlns="http://www.w3.org/2000/svg"><g transform="translate(512 512) scale(${WIN_FILL}) translate(-512 -512)">${inner}</g></svg>`;

const work = mkdtempSync(join(tmpdir(), 'dmvoice-ico-'));
try {
  const svgPath = join(work, 'win.svg');
  writeFileSync(svgPath, winSvg);

  // Render each size to a PNG frame via librsvg.
  const frames = sizes.map((size) => {
    const png = join(work, `${size}.png`);
    execFileSync('rsvg-convert', ['-w', String(size), '-h', String(size), svgPath, '-o', png]);
    return { size, data: readFileSync(png) };
  });

  // Assemble a PNG-frame .ico (ICONDIR + per-frame ICONDIRENTRY + PNG payloads).
  const header = Buffer.alloc(6 + 16 * frames.length);
  header.writeUInt16LE(0, 0); // reserved
  header.writeUInt16LE(1, 2); // type = icon
  header.writeUInt16LE(frames.length, 4);
  let offset = 6 + 16 * frames.length;
  frames.forEach((f, i) => {
    const o = 6 + i * 16;
    header.writeUInt8(f.size >= 256 ? 0 : f.size, o);
    header.writeUInt8(f.size >= 256 ? 0 : f.size, o + 1);
    header.writeUInt8(0, o + 2); // palette
    header.writeUInt8(0, o + 3); // reserved
    header.writeUInt16LE(1, o + 4); // colour planes
    header.writeUInt16LE(32, o + 6); // bits per pixel
    header.writeUInt32LE(f.data.length, o + 8);
    header.writeUInt32LE(offset, o + 12);
    offset += f.data.length;
  });
  writeFileSync(out, Buffer.concat([header, ...frames.map((f) => f.data)]));
  console.log(`wrote ${out} (${frames.length} frames: ${sizes.join(', ')})`);
} finally {
  rmSync(work, { recursive: true, force: true });
}
