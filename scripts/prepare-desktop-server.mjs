import { createHash } from "node:crypto";
import { chmodSync, copyFileSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
function command(program, args, capture = false) {
  const result = spawnSync(program, args, {
    cwd: root,
    encoding: "utf8",
    stdio: capture ? ["ignore", "pipe", "inherit"] : "inherit",
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${program} failed (${result.status})`);
  return result.stdout;
}

const metadata = JSON.parse(command("cargo", ["metadata", "--locked", "--format-version", "1", "--no-deps"], true));
const server = metadata.packages.find((entry) => entry.name === "nexum-server");
const desktop = metadata.packages.find((entry) => entry.name === "nexum-desktop");
if (server.version !== desktop.version) throw new Error("Server and Desktop package versions must match");
const config = JSON.parse(readFileSync(join(root, "apps", "desktop", "src-tauri", "tauri.conf.json"), "utf8"));
if (config.version !== desktop.version) throw new Error("App bundle and Desktop package versions must match");
const host = command("rustc", ["-vV"], true).match(/^host: (.+)$/m)?.[1];
const target = process.env.TAURI_ENV_TARGET_TRIPLE || host;
if (!target || target.startsWith("universal-")) {
  throw new Error("Prepare a Desktop App for one explicit Rust target architecture");
}
const debug = process.argv.includes("--dev") || process.env.TAURI_ENV_DEBUG === "true";
const profile = debug ? "debug" : "release";
command("cargo", ["build", "--locked", "-p", "nexum-server", "--target", target, ...(!debug ? ["--release"] : [])]);

const staging = join(root, "apps", "desktop", "src-tauri", "resources");
mkdirSync(staging, { recursive: true });
const executable = join(staging, "nexum-server");
const temporary = `${executable}.tmp`;
const source = join(metadata.target_directory, target, profile, target.includes("windows") ? "nexum-server.exe" : "nexum-server");
copyFileSync(source, temporary);
chmodSync(temporary, 0o755);
if (target.endsWith("apple-darwin")) {
  if (process.platform !== "darwin") throw new Error("A macOS Server bundle must be prepared on macOS");
  // Sign before hashing. Keep this as a resource rather than an externalBin:
  // Tauri's sidecar signing would otherwise change the embedded digest later.
  command("/usr/bin/codesign", ["--force", "--sign", "-", temporary]);
  command("/usr/bin/codesign", ["--verify", "--strict", temporary]);
}
renameSync(temporary, executable);
const sha256 = createHash("sha256").update(readFileSync(executable)).digest("hex");
writeFileSync(join(staging, "nexum-server-build.json"), `${JSON.stringify({ target, server_version: server.version, sha256 }, null, 2)}\n`);
console.log(`Prepared ${target} Server ${server.version} for Desktop (${profile}).`);
