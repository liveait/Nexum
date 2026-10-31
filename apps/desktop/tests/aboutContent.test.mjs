import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { openProjectLink, readAboutVersion } from "../src/aboutContent.ts";

beforeEach(() => { globalThis.window = {}; });
afterEach(() => { clearMocks(); delete globalThis.window; });

test("About reads the running package version through the real Tauri app API", async () => {
  const calls = [];
  mockIPC((command) => { calls.push(command); return "9.4.2-qa.3+build7"; });
  assert.deepEqual(await readAboutVersion(), { state: "ready", version: "9.4.2-qa.3+build7" });
  assert.deepEqual(calls, ["plugin:app|version"]);
});

test("unavailable or invalid version responses fail visibly and can be retried", async () => {
  for (const value of ["", "   ", null, undefined, {}]) {
    mockIPC(() => value);
    assert.deepEqual(await readAboutVersion(), { state: "error" });
  }
  mockIPC(() => { throw new Error("version IPC unavailable"); });
  assert.deepEqual(await readAboutVersion(), { state: "error" });
  mockIPC(() => "2.8.1");
  assert.deepEqual(await readAboutVersion(), { state: "ready", version: "2.8.1" });
});

test("project links use only finite backend IDs rather than URLs or webview navigation", async () => {
  const calls = [];
  mockIPC((command, args) => { calls.push({ command, args }); });
  for (const link of ["website", "source", "documentation"]) await openProjectLink(link);
  assert.deepEqual(calls, ["website", "source", "documentation"].map((link) => ({ command: "open_project_link", args: { link } })));
  for (const link of ["https://example.com", "file:///tmp/file", "javascript:alert(1)", "__proto__", "Website", ""]) {
    await assert.rejects(openProjectLink(link), /Unsupported/);
  }
  assert.equal(calls.length, 3);
});

test("external-open failures remain failures and a subsequent user retry can succeed", async () => {
  mockIPC(() => { throw new Error("browser unavailable"); });
  await assert.rejects(openProjectLink("website"), /browser unavailable/);
  mockIPC(() => null);
  await assert.doesNotReject(openProjectLink("website"));
});
