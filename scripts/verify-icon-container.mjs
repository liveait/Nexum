import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { inflateSync } from "node:zlib";

function crc32(bytes) {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ ((crc & 1) ? 0xedb88320 : 0);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function paeth(left, above, diagonal) {
  const estimate = left + above - diagonal;
  const distances = [left, above, diagonal].map((value) => Math.abs(estimate - value));
  return distances[0] <= distances[1] && distances[0] <= distances[2] ? left : distances[1] <= distances[2] ? above : diagonal;
}

// Decode the project's non-interlaced RGBA8 PNG frames, including CRC, zlib
// and row filters. A readable directory alone does not prove a usable icon.
function decodePng(bytes, width, height) {
  assert.ok(bytes.subarray(0, 8).equals(Buffer.from("89504e470d0a1a0a", "hex")), "frame must contain a PNG");
  let position = 8;
  let header = false;
  let ended = false;
  const compressed = [];
  while (position < bytes.length) {
    assert.ok(position + 12 <= bytes.length, "truncated PNG chunk");
    const length = bytes.readUInt32BE(position);
    const end = position + length + 12;
    assert.ok(end <= bytes.length, "PNG chunk extends beyond its frame");
    const type = bytes.toString("ascii", position + 4, position + 8);
    const data = bytes.subarray(position + 8, end - 4);
    assert.equal(crc32(bytes.subarray(position + 4, end - 4)), bytes.readUInt32BE(end - 4), `${type} CRC mismatch`);
    if (!header) {
      assert.equal(type, "IHDR", "PNG must start with IHDR");
      assert.equal(length, 13, "invalid IHDR length");
      assert.equal(data.readUInt32BE(0), width, "PNG width differs from ICO directory");
      assert.equal(data.readUInt32BE(4), height, "PNG height differs from ICO directory");
      assert.ok(data.subarray(8).equals(Buffer.from([8, 6, 0, 0, 0])), "brand frames must be non-interlaced RGBA8");
      header = true;
    } else {
      assert.notEqual(type, "IHDR", "duplicate IHDR");
      if (type === "IDAT") compressed.push(data);
      if (type === "IEND") {
        assert.equal(length, 0, "invalid IEND length");
        assert.equal(end, bytes.length, "data after PNG IEND");
        ended = true;
      }
    }
    position = end;
  }
  assert.ok(header && ended && compressed.length, "incomplete PNG");
  const stride = width * 4;
  const expected = (stride + 1) * height;
  const filtered = inflateSync(Buffer.concat(compressed), { maxOutputLength: expected });
  assert.equal(filtered.length, expected, "incomplete PNG pixel rows");
  const pixels = Buffer.alloc(stride * height);
  for (let y = 0; y < height; y++) {
    const filter = filtered[y * (stride + 1)];
    assert.ok(filter <= 4, "invalid PNG row filter");
    for (let x = 0; x < stride; x++) {
      const offset = y * stride + x;
      const left = x >= 4 ? pixels[offset - 4] : 0;
      const above = y > 0 ? pixels[offset - stride] : 0;
      const diagonal = y > 0 && x >= 4 ? pixels[offset - stride - 4] : 0;
      const predictor = [0, left, above, Math.floor((left + above) / 2), paeth(left, above, diagonal)][filter];
      pixels[offset] = (filtered[y * (stride + 1) + x + 1] + predictor) & 0xff;
    }
  }
  return pixels;
}

function decodeIcnsRgb(bytes, alpha, size) {
  const planeLength = size * size;
  assert.equal(alpha.length, planeLength, "incomplete ICNS alpha mask");
  const pixels = Buffer.alloc(planeLength * 4);
  let position = 0;
  for (let channel = 0; channel < 3; channel++) {
    let output = 0;
    while (output < planeLength) {
      assert.ok(position < bytes.length, "truncated ICNS RGB run");
      const control = bytes[position++];
      const length = control < 128 ? control + 1 : control - 125;
      assert.ok(output + length <= planeLength, "ICNS RGB run crosses its plane");
      if (control < 128) {
        assert.ok(position + length <= bytes.length, "truncated ICNS RGB literal");
        for (let index = 0; index < length; index++) pixels[(output + index) * 4 + channel] = bytes[position++];
      } else {
        assert.ok(position < bytes.length, "missing ICNS RGB run value");
        const value = bytes[position++];
        for (let index = 0; index < length; index++) pixels[(output + index) * 4 + channel] = value;
      }
      output += length;
    }
  }
  assert.equal(position, bytes.length, "data after ICNS RGB planes");
  for (let index = 0; index < planeLength; index++) pixels[index * 4 + 3] = alpha[index];
  return pixels;
}

export function verifyIcnsContainer(icnsPath, canonicalPngPath) {
  const bytes = readFileSync(icnsPath);
  assert.ok(bytes.length >= 8, "truncated ICNS header");
  assert.equal(bytes.toString("ascii", 0, 4), "icns", "expected an ICNS container");
  assert.equal(bytes.readUInt32BE(4), bytes.length, "ICNS length mismatch");
  const records = new Map();
  for (let position = 8; position < bytes.length;) {
    assert.ok(position + 8 <= bytes.length, "truncated ICNS record");
    const type = bytes.toString("ascii", position, position + 4);
    const length = bytes.readUInt32BE(position + 4);
    assert.ok(length >= 8 && position + length <= bytes.length, "ICNS record outside container");
    assert.ok(!records.has(type), `duplicate ICNS ${type}`);
    records.set(type, bytes.subarray(position + 8, position + length));
    position += length;
  }
  const pngSizes = new Map([["ic07", 128], ["ic08", 256], ["ic09", 512], ["ic10", 1024], ["ic11", 32], ["ic12", 64], ["ic13", 256], ["ic14", 512]]);
  const rgbSizes = new Map([["is32", [16, "s8mk"]], ["il32", [32, "l8mk"]]]);
  const frames = [];
  for (const [type, payload] of records) {
    let pixels;
    let size;
    if (pngSizes.has(type)) {
      size = pngSizes.get(type);
      pixels = decodePng(payload, size, size);
    } else if (rgbSizes.has(type)) {
      const [dimension, mask] = rgbSizes.get(type);
      size = dimension;
      assert.ok(records.has(mask), `missing ICNS ${mask}`);
      pixels = decodeIcnsRgb(payload, records.get(mask), size);
    } else {
      assert.ok(type === "s8mk" || type === "l8mk", `unsupported ICNS representation ${type}`);
      continue;
    }
    frames.push({ type, width: size, height: size, pixelSha256: createHash("sha256").update(pixels).digest("hex") });
  }
  for (const size of [16, 32, 64, 128, 256, 512, 1024]) assert.ok(frames.some((frame) => frame.width === size), `missing ${size}px ICNS representation`);
  assert.ok(records.get("ic07")?.equals(readFileSync(canonicalPngPath)), "128px ICNS frame differs from approved canonical PNG");
  return frames;
}

export function verifyIconContainer(iconPath, canonicalPngPath) {
  const bytes = readFileSync(iconPath);
  const canonical = readFileSync(canonicalPngPath);
  assert.ok(bytes.length >= 6, "truncated ICO header");
  assert.equal(bytes.readUInt16LE(0), 0, "invalid ICO reserved field");
  assert.equal(bytes.readUInt16LE(2), 1, "expected an ICO container");
  const count = bytes.readUInt16LE(4);
  assert.ok(count > 0 && 6 + count * 16 <= bytes.length, "truncated ICO directory");
  const frames = [];
  const ranges = [];
  for (let index = 0; index < count; index++) {
    const entry = 6 + index * 16;
    const width = bytes[entry] || 256;
    const height = bytes[entry + 1] || 256;
    const size = bytes.readUInt32LE(entry + 8);
    const offset = bytes.readUInt32LE(entry + 12);
    assert.ok(size > 0 && offset >= 6 + count * 16 && offset + size <= bytes.length,
      `${width}x${height} entry outside ICO: offset ${offset} + size ${size} exceeds ${bytes.length} bytes or overlaps directory`);
    assert.ok(ranges.every(([start, end]) => offset + size <= start || offset >= end), "overlapping ICO frames");
    ranges.push([offset, offset + size]);
    const payload = bytes.subarray(offset, offset + size);
    const pixels = decodePng(payload, width, height);
    if (width === 128 && height === 128) assert.ok(payload.equals(canonical), "128px ICO frame differs from approved canonical PNG");
    frames.push({ width, height, bytes: size, pixelSha256: createHash("sha256").update(pixels).digest("hex") });
  }
  for (const size of [16, 32, 48, 128]) assert.ok(frames.some((frame) => frame.width === size && frame.height === size), `missing ${size}px brand frame`);
  return frames;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const directory = process.argv[2] ? resolve(process.argv[2]) : join(dirname(fileURLToPath(import.meta.url)), "../apps/desktop/src-tauri/icons");
  const frames = verifyIconContainer(join(directory, "icon.ico"), join(directory, "icon.png"));
  console.log(`Verified Nexum ICO frames: ${frames.map(({ width, height }) => `${width}x${height}`).join(", ")}; 128px matches canonical PNG.`);
  const icnsFrames = verifyIcnsContainer(join(directory, "icon.icns"), join(directory, "icon.png"));
  console.log(`Verified Nexum ICNS representations: ${icnsFrames.map(({ type, width }) => `${type} ${width}px`).join(", ")}; 128px matches canonical PNG.`);
}
