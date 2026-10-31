const repository = "liveait/Nexum";
const maximumManifestBytes = 64 * 1024;

function requireValue(condition, message) {
  if (!condition) throw new Error(`Invalid release metadata: ${message}`);
}

function assetUrl(value, tag, name) {
  const expected = `https://github.com/${repository}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(name)}`;
  requireValue(value === expected, "asset URL must belong to this repository and release");
  return value;
}

/** M3 acceptance is a publisher attestation; API digests additionally bind it to the asset. */
export function validatedRelease(release, manifest) {
  requireValue(!release.draft && !release.prerelease, "only stable public releases are downloadable");
  requireValue(/^v\d+\.\d+\.\d+$/.test(release.tag_name), "stable version tag required");
  requireValue(manifest?.schema_version === 1, "unsupported manifest schema");
  requireValue(manifest.version === release.tag_name.slice(1), "version does not match release");
  requireValue(/^[a-f0-9]{40}$/.test(manifest.source_sha), "full source SHA required");
  requireValue(manifest.m3_accepted === true, "M3 acceptance is required");
  requireValue(Array.isArray(manifest.artifacts) && manifest.artifacts.length > 0, "no accepted artifacts");
  const architectures = new Set();
  const artifacts = manifest.artifacts.map((entry) => {
    requireValue(entry.os === "macos" && ["aarch64", "x86_64"].includes(entry.arch), "unvalidated platform");
    requireValue(!architectures.has(entry.arch), "duplicate architecture");
    architectures.add(entry.arch);
    requireValue(/^\d+(\.\d+){0,2}$/.test(entry.minimum_os), "minimum macOS version required");
    requireValue(entry.signing === "developer-id" && entry.notarized === true && entry.gatekeeper_verified === true, "distribution checks incomplete");
    requireValue(typeof entry.name === "string" && /^[A-Za-z0-9][A-Za-z0-9._-]*\.(dmg|zip)$/.test(entry.name), "invalid installer name");
    const asset = release.assets?.find((candidate) => candidate.name === entry.name);
    requireValue(asset?.state === "uploaded", "installer is missing or incomplete");
    requireValue(/^[a-f0-9]{64}$/.test(entry.sha256), "SHA256 required");
    requireValue(asset.digest === `sha256:${entry.sha256}`, "installer digest mismatch");
    requireValue(Number.isSafeInteger(entry.size) && entry.size > 0 && entry.size === asset.size, "installer size mismatch");
    return { ...entry, url: assetUrl(asset.browser_download_url, release.tag_name, entry.name) };
  });
  return {
    available: true,
    version: manifest.version,
    source_sha: manifest.source_sha,
    url: `https://github.com/${repository}/releases/tag/${encodeURIComponent(release.tag_name)}`,
    artifacts,
  };
}

export async function fetchRelease(fetcher = fetch) {
  const response = await fetcher(`https://api.github.com/repos/${repository}/releases/latest`, {
    headers: { Accept: "application/vnd.github+json", "User-Agent": "Nexum-website-build" },
    signal: AbortSignal.timeout(15_000),
  });
  if (response.status === 404) return { available: false };
  if (!response.ok) throw new Error(`GitHub release lookup failed: ${response.status}`);
  const release = await response.json();
  const asset = release.assets?.find((item) => item.name === "nexum-release.json");
  requireValue(asset?.state === "uploaded" && asset.size > 0 && asset.size <= maximumManifestBytes, "bounded nexum-release.json required");
  const url = assetUrl(asset.browser_download_url, release.tag_name, asset.name);
  const metadata = await fetcher(url, { signal: AbortSignal.timeout(15_000) });
  if (!metadata.ok) throw new Error(`Release manifest lookup failed: ${metadata.status}`);
  // Bound streamed bytes as well as the untrusted API size, before JSON parsing.
  requireValue(metadata.body, "release manifest body missing");
  const reader = metadata.body.getReader();
  const chunks = [];
  let count = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      count += value.byteLength;
      requireValue(count <= maximumManifestBytes, "release manifest too large");
      chunks.push(value);
    }
  } finally {
    await reader.cancel();
  }
  return validatedRelease(release, JSON.parse(Buffer.concat(chunks).toString("utf8")));
}
