import test from "node:test";
import assert from "node:assert/strict";
import { cp, mkdir, mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFileSync } from "node:child_process";
import { buildSettings, normalizeBasePath } from "./site-config.mjs";

test("base paths support project hosting without accepting external or ambiguous paths", () => {
  assert.equal(normalizeBasePath(), "/");
  assert.equal(normalizeBasePath("/Nexum"), "/Nexum/");
  assert.equal(normalizeBasePath("/Nexum/"), "/Nexum/");
  for (const path of ["Nexum", "//example.com/", "/../", "/Nexum//", "/Nexum?token=x"]) {
    assert.throws(() => normalizeBasePath(path));
  }
});

test("only main push/dispatch can synchronize downloads inside Actions", () => {
  const main = { GITHUB_ACTIONS: "true", GITHUB_REF: "refs/heads/main", GITHUB_EVENT_NAME: "push" };
  assert.equal(buildSettings(main).syncRelease, true);
  assert.equal(buildSettings({ ...main, GITHUB_EVENT_NAME: "workflow_dispatch" }).syncRelease, true);
  for (const env of [
    { ...main, GITHUB_EVENT_NAME: "pull_request" },
    { ...main, GITHUB_REF: "refs/pull/1/merge", GITHUB_EVENT_NAME: "pull_request" },
    { ...main, GITHUB_REF: "refs/heads/feature", GITHUB_EVENT_NAME: "workflow_dispatch" },
    { ...main, GITHUB_REF: "refs/tags/v0.1.0", GITHUB_EVENT_NAME: "release" },
  ]) {
    assert.equal(buildSettings(env).syncRelease, false);
    assert.throws(() => buildSettings(env, ["--sync-release"]), /Only main/);
  }
  assert.equal(buildSettings({}, ["--sync-release"]).syncRelease, true);
});

test("builds both languages and all local resources under the project prefix", async () => {
  const root = await mkdtemp(join(tmpdir(), "nexum-website-build-"));
  const site = join(root, "apps/website");
  try {
    await mkdir(site, { recursive: true });
    for (const name of ["build.mjs", "release.mjs", "site-config.mjs", "style.css"]) {
      await cp(new URL(name, import.meta.url), join(site, name));
    }
    await cp(new URL("assets/", import.meta.url), join(site, "assets"), { recursive: true });
    await mkdir(join(root, "apps/desktop/src-tauri/icons"), { recursive: true });
    const canonical = new URL("../desktop/src-tauri/icons/icon.png", import.meta.url);
    await cp(canonical, join(root, "apps/desktop/src-tauri/icons/icon.png"));
    execFileSync(process.execPath, [join(site, "build.mjs")], {
      env: { ...process.env, SITE_BASE_PATH: "/Nexum", GITHUB_ACTIONS: "", GITHUB_REF: "", GITHUB_EVENT_NAME: "" },
    });
    const output = join(site, "dist");
    assert.deepEqual(await readFile(join(output, "assets/nexum.png")), await readFile(canonical));
    for (const locale of ["zh", "en"]) {
      const html = await readFile(join(output, locale, "index.html"), "utf8");
      const resources = [...html.matchAll(/(?:href|src|srcset)="([^"]+)"/g)].map((match) => match[1]);
      assert.ok(resources.includes("/Nexum/assets/nexum.png"));
      assert.ok(resources.includes("/Nexum/zh/") && resources.includes("/Nexum/en/"));
      for (const path of resources.filter((path) => path.startsWith("/"))) {
        assert.ok(path.startsWith("/Nexum/"), path);
        const target = path.slice("/Nexum/".length);
        await readFile(join(output, target.endsWith("/") ? `${target}index.html` : target));
      }
      assert.doesNotMatch(html, /releases\/download\//);
    }
    const index = await readFile(join(output, "index.html"), "utf8");
    assert.match(index, /url=\/Nexum\/zh\//);
    assert.deepEqual(JSON.parse(await readFile(join(output, "release.json"), "utf8")), { available: false });
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
