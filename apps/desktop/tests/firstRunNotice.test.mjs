import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { FirstRunNoticeController } from "../src/firstRunNotice.ts";

beforeEach(() => { globalThis.window = {}; });
afterEach(() => { clearMocks(); delete globalThis.window; });
function deferred() {
  let resolve;
  const promise = new Promise((done) => { resolve = done; });
  return { promise, resolve };
}

test("first-run eligibility uses one native IPC read per controller", async () => {
  const read = deferred();
  const calls = [];
  const states = [];
  mockIPC((command, args) => { calls.push({ command, args }); return read.promise; });
  const controller = new FirstRunNoticeController((visible) => states.push(visible));
  const pending = controller.load();
  await controller.load();
  assert.deepEqual(calls, [{ command: "check_fresh_install", args: {} }]);
  assert.deepEqual(states, []);
  read.resolve(true);
  await pending;
  await controller.load();
  assert.deepEqual(states, [true]);
  assert.equal(calls.length, 1);
});

test("continuing a welcome cannot be reversed by another task refresh or load", async () => {
  let reads = 0;
  const states = [];
  mockIPC(() => { ++reads; return true; });
  const controller = new FirstRunNoticeController((visible) => states.push(visible));
  await controller.load();
  controller.dismiss();
  await controller.load();
  assert.deepEqual(states, [true, false]);
  assert.equal(reads, 1);
});

test("a late native reply cannot revive a continued welcome", async () => {
  const read = deferred();
  const states = [];
  mockIPC(() => read.promise);
  const controller = new FirstRunNoticeController((visible) => states.push(visible));
  const pending = controller.load();
  controller.dismiss();
  read.resolve(true);
  await pending;
  assert.deepEqual(states, [false]);
});

test("disposed controllers ignore pending native replies and issue no new reads", async () => {
  const read = deferred();
  let reads = 0;
  const states = [];
  mockIPC(() => { ++reads; return read.promise; });
  const controller = new FirstRunNoticeController((visible) => states.push(visible));
  const pending = controller.load();
  controller.dispose();
  read.resolve(true);
  await pending;
  await controller.load();
  controller.dismiss();
  assert.deepEqual(states, []);
  assert.equal(reads, 1);
});

test("existing profiles and failed reads do not show a fresh-install welcome", async () => {
  const states = [];
  mockIPC(() => false);
  const existing = new FirstRunNoticeController((visible) => states.push(visible));
  await existing.load();
  assert.deepEqual(states, [false]);
  mockIPC(() => { throw new Error("IPC unavailable"); });
  const unavailable = new FirstRunNoticeController((visible) => states.push(visible));
  await unavailable.load();
  assert.deepEqual(states, [false]);
});
