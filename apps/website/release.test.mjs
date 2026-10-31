import test from "node:test";
import assert from "node:assert/strict";
import { validatedRelease, fetchRelease } from "./release.mjs";

function fixture() {
  const artifact = { os: "macos", arch: "aarch64", minimum_os: "12", name: "Nexum-0.1.0-aarch64.dmg", size: 42, sha256: "a".repeat(64), signing: "developer-id", notarized: true, gatekeeper_verified: true };
  const release = { tag_name: "v0.1.0", draft: false, prerelease: false, assets: [{ name: artifact.name, size: artifact.size, digest: `sha256:${artifact.sha256}`, state: "uploaded", browser_download_url: `https://github.com/liveait/Nexum/releases/download/v0.1.0/${artifact.name}` }] };
  const manifest = { schema_version: 1, version: "0.1.0", source_sha: "b".repeat(40), m3_accepted: true, artifacts: [artifact] };
  return { release, manifest };
}

test("only accepted artifacts with matching GitHub digest and size become downloads", () => {
  const { release, manifest } = fixture();
  const result = validatedRelease(release, manifest);
  assert.equal(result.available, true);
  assert.equal(result.artifacts[0].url, release.assets[0].browser_download_url);
  assert.equal(result.source_sha, manifest.source_sha);
});

test("rejects incomplete acceptance, mismatched artifacts, unsupported platforms and foreign URLs", () => {
  const mutations = [
    ({ manifest }) => { manifest.m3_accepted = false; },
    ({ manifest }) => { manifest.artifacts[0].notarized = false; },
    ({ manifest }) => { manifest.artifacts[0].signing = "ad-hoc"; },
    ({ manifest }) => { manifest.artifacts[0].gatekeeper_verified = false; },
    ({ manifest }) => { manifest.artifacts[0].os = "windows"; },
    ({ manifest }) => { manifest.artifacts[0].arch = "unknown"; },
    ({ manifest }) => { manifest.artifacts.push(manifest.artifacts[0]); },
    ({ manifest }) => { manifest.version = "0.2.0"; },
    ({ manifest }) => { manifest.artifacts[0].size++; },
    ({ manifest }) => { manifest.artifacts[0].sha256 = "c".repeat(64); },
    ({ release }) => { release.prerelease = true; },
    ({ release }) => { release.assets[0].browser_download_url = "https://example.com/installer.dmg"; },
    ({ release }) => { release.assets[0].digest = null; },
    ({ release }) => { release.assets[0].state = "new"; },
  ];
  for (const mutate of mutations) {
    const input = fixture();
    mutate(input);
    assert.throws(() => validatedRelease(input.release, input.manifest), /Invalid release metadata/);
  }
});

test("a repository without releases has no download link; API outages fail the build", async () => {
  assert.deepEqual(await fetchRelease(async () => new Response(null, { status: 404 })), { available: false });
  await assert.rejects(fetchRelease(async () => new Response(null, { status: 503 })), /lookup failed/);
});

test("fetches a bounded public manifest and rejects oversized streamed bodies", async () => {
  const { release, manifest } = fixture();
  release.assets.push({ name: "nexum-release.json", size: 1024, state: "uploaded", browser_download_url: "https://github.com/liveait/Nexum/releases/download/v0.1.0/nexum-release.json" });
  let requests = 0;
  const fetcher = async () => new Response(JSON.stringify(++requests === 1 ? release : manifest));
  assert.equal((await fetchRelease(fetcher)).available, true);
  requests = 0;
  await assert.rejects(fetchRelease(async () => new Response(++requests === 1 ? JSON.stringify(release) : " ".repeat(65537))), /too large/);
});
