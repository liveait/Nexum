import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { verifyIcnsContainer, verifyIconContainer } from "./verify-icon-container.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const sourcePath = "assets/brand/icon-graphite-v1.png";
const source = join(root, sourcePath);
const markSourcePath = "assets/brand/nexum-mark.png";
const markSource = join(root, markSourcePath);
const desktop = join(root, "apps/desktop");
const icons = join(desktop, "src-tauri/icons");
const packageJson = JSON.parse(readFileSync(join(desktop, "package.json"), "utf8"));
const cliDirectory = join(desktop, "node_modules/@tauri-apps/cli");
const cliPackage = JSON.parse(readFileSync(join(cliDirectory, "package.json"), "utf8"));
const cli = join(cliDirectory, cliPackage.bin.tauri);
const temporary = mkdtempSync(join(tmpdir(), "nexum-brand-icons-"));
function tauri(args, capture = false) {
  // Execute the installed official CLI directly, without a package-manager
  // wrapper that can install packages or prepend policy output to --version.
  const result = spawnSync(process.execPath, [cli, ...args], {
    cwd: root, encoding: "utf8", stdio: capture ? ["ignore", "pipe", "inherit"] : "inherit",
  });
  if (result.error) throw result.error;
  assert.equal(result.status, 0, "Tauri icon generation failed");
  return result.stdout?.trim();
}
function sha256(bytes) { return createHash("sha256").update(bytes).digest("hex"); }

// The CLI's ICNS record order varies between runs. Sorting complete records
// makes the container reproducible while preserving each generated payload.
function orderedIcns(bytes) {
  assert.equal(bytes.toString("ascii", 0, 4), "icns");
  assert.equal(bytes.readUInt32BE(4), bytes.length);
  const records = [];
  for (let position = 8; position < bytes.length;) {
    assert.ok(position + 8 <= bytes.length);
    const length = bytes.readUInt32BE(position + 4);
    assert.ok(length >= 8 && position + length <= bytes.length);
    records.push(Buffer.from(bytes.subarray(position, position + length)));
    position += length;
  }
  records.sort((left, right) => Buffer.compare(left.subarray(0, 4), right.subarray(0, 4)));
  return Buffer.concat([bytes.subarray(0, 8), ...records]);
}

// Tauri's standard ICO omits 128px. Retain every generated PNG frame and add
// that representation without decoding, repainting or re-encoding any pixels.
function withCanonical128(ico, canonical) {
  assert.equal(ico.readUInt16LE(0), 0);
  assert.equal(ico.readUInt16LE(2), 1);
  const count = ico.readUInt16LE(4);
  assert.ok(6 + count * 16 <= ico.length);
  const frames = [];
  for (let index = 0; index < count; index++) {
    const entry = Buffer.from(ico.subarray(6 + index * 16, 22 + index * 16));
    const size = entry.readUInt32LE(8);
    const offset = entry.readUInt32LE(12);
    assert.ok(size > 0 && offset >= 6 + count * 16 && offset + size <= ico.length);
    frames.push({ entry, payload: Buffer.from(ico.subarray(offset, offset + size)) });
  }
  const existing = frames.find(({ entry }) => entry[0] === 128 && entry[1] === 128);
  if (existing) existing.payload = canonical;
  else {
    const entry = Buffer.alloc(16);
    entry[0] = 128;
    entry[1] = 128;
    entry.writeUInt16LE(1, 4);
    entry.writeUInt16LE(32, 6);
    frames.push({ entry, payload: canonical });
  }
  const header = Buffer.from(ico.subarray(0, 6));
  header.writeUInt16LE(frames.length, 4);
  let offset = 6 + frames.length * 16;
  for (const frame of frames) {
    frame.entry.writeUInt32LE(frame.payload.length, 8);
    frame.entry.writeUInt32LE(offset, 12);
    offset += frame.payload.length;
  }
  return Buffer.concat([header, ...frames.map(({ entry }) => entry), ...frames.map(({ payload }) => payload)]);
}

try {
  const sourceBytes = readFileSync(source);
  assert.ok(sourceBytes.subarray(0, 8).equals(Buffer.from("89504e470d0a1a0a", "hex")), "source must be a PNG");
  assert.equal(sourceBytes.readUInt32BE(16), sourceBytes.readUInt32BE(20), "source must be square");
  assert.ok(sourceBytes.readUInt32BE(16) >= 1024, "use the high-resolution selected source, never upscale the 128px derivative");
  const version = tauri(["--version"], true);
  assert.equal(version, `tauri-cli ${packageJson.devDependencies["@tauri-apps/cli"]}`, "use the pinned local Tauri CLI");
  const standard = join(temporary, "standard");
  const png = join(temporary, "png");
  const markPng = join(temporary, "mark-png");
  tauri(["icon", source, "--output", standard]);
  const sizes = [16, 32, 48, 128, 180, 1024];
  tauri(["icon", source, "--output", png, ...sizes.flatMap((size) => ["--png", String(size)])]);
  const markBytes = readFileSync(markSource);
  assert.ok(markBytes.subarray(0, 8).equals(Buffer.from("89504e470d0a1a0a", "hex")), "mark source must be a PNG");
  assert.equal(markBytes.readUInt32BE(16), markBytes.readUInt32BE(20), "mark source must be square");
  assert.ok(markBytes.readUInt32BE(16) >= 128, "never upscale the transparent mark");
  tauri(["icon", markSource, "--output", markPng, "--png", "128"]);
  const icns = join(temporary, "icon.icns");
  writeFileSync(icns, orderedIcns(readFileSync(join(standard, "icon.icns"))));
  mkdirSync(icons, { recursive: true });
  const outputs = new Map([
    ["apps/desktop/src-tauri/icons/icon.png", join(png, "128x128.png")],
    ["apps/desktop/src-tauri/icons/icon-1024.png", join(png, "1024x1024.png")],
    ["apps/desktop/src-tauri/icons/apple-touch-icon.png", join(png, "180x180.png")],
    ["apps/desktop/src-tauri/icons/brand-mark.png", join(markPng, "128x128.png")],
    ["apps/desktop/src-tauri/icons/icon.icns", icns],
    ...[16, 32, 48, 128].map((size) => [`apps/extension/icons/nexum-${size}.png`, join(png, `${size}x${size}.png`)]),
  ]);
  for (const [destination, generated] of outputs) copyFileSync(generated, join(root, destination));
  const icoPath = "apps/desktop/src-tauri/icons/icon.ico";
  writeFileSync(join(root, icoPath), withCanonical128(readFileSync(join(standard, "icon.ico")), readFileSync(join(icons, "icon.png"))));
  const frames = verifyIconContainer(join(root, icoPath), join(icons, "icon.png"));
  const icnsFrames = verifyIcnsContainer(join(icons, "icon.icns"), join(icons, "icon.png"));
  const manifest = {
    source: sourcePath,
    source_sha256: sha256(sourceBytes),
    source_dimensions: [sourceBytes.readUInt32BE(16), sourceBytes.readUInt32BE(20)],
    mark_source: markSourcePath,
    mark_source_sha256: sha256(markBytes),
    mark_source_dimensions: [markBytes.readUInt32BE(16), markBytes.readUInt32BE(20)],
    tauri_cli: version,
    generation_command: "node scripts/generate-brand-icons.mjs",
    canonical_png_size: 128,
    icns_record_order: "type identifiers sorted; generated record payloads unchanged",
    ico_sizes: frames.map(({ width }) => width),
    icns_sizes: [...new Set(icnsFrames.map(({ width }) => width))].sort((left, right) => left - right),
    outputs: Object.fromEntries([...outputs.keys(), icoPath].sort().map((path) => [path, sha256(readFileSync(join(root, path)))])),
  };
  writeFileSync(join(root, "assets/brand/icon-manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(`Generated Desktop, Apple touch and extension icons from ${sourcePath} (${manifest.source_sha256}), and the transparent brand mark from ${markSourcePath} (${manifest.mark_source_sha256}).`);
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
