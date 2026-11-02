import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { EMPTY_UPGRADE_NOTICE, UpgradeNoticeController } from "../src/upgradeNotice.ts";

beforeEach(() => { globalThis.window = {}; });
afterEach(() => { clearMocks(); delete globalThis.window; });
const info = { old_version: "0.0.9", new_version: "0.1.0" };
function deferred() {
  let resolve;
  const promise = new Promise((done) => { resolve = done; });
  return { promise, resolve };
}

test("Dismiss invokes backend acknowledgement without a supplied version and waits for persistence", async () => {
  const completion = deferred();
  const calls = [];
  mockIPC((command, args) => {
    calls.push({ command, args });
    return command === "get_upgrade_info" ? info : completion.promise;
  });
  let state = EMPTY_UPGRADE_NOTICE;
  const controller = new UpgradeNoticeController((next) => { state = next; });
  await controller.load();
  const dismiss = controller.dismiss();
  await controller.dismiss(); // A second click cannot issue a second write.
  assert.deepEqual(state, { info, busy: true, failed: false });
  assert.deepEqual(calls.map((call) => call.command), ["get_upgrade_info", "acknowledge_upgrade"]);
  assert.deepEqual(calls[1].args, {});
  completion.resolve(null);
  await dismiss;
  assert.deepEqual(state, EMPTY_UPGRADE_NOTICE);
});

test("failed acknowledgement keeps the visible upgrade and can be retried", async () => {
  let attempts = 0;
  mockIPC((command) => {
    if (command === "get_upgrade_info") return info;
    if (++attempts === 1) throw new Error("storage unavailable");
  });
  let state = EMPTY_UPGRADE_NOTICE;
  const controller = new UpgradeNoticeController((next) => { state = next; });
  await controller.load();
  await controller.dismiss();
  assert.deepEqual(state, { info, busy: false, failed: true });
  await controller.dismiss();
  assert.deepEqual(state, EMPTY_UPGRADE_NOTICE);
  assert.equal(attempts, 2);
});

test("an old pending upgrade read cannot resurrect a successfully dismissed notice", async () => {
  const stale = deferred();
  let reads = 0;
  mockIPC((command) => command === "get_upgrade_info" ? (++reads === 1 ? info : stale.promise) : null);
  let state = EMPTY_UPGRADE_NOTICE;
  const controller = new UpgradeNoticeController((next) => { state = next; });
  await controller.load();
  const pendingRead = controller.load();
  await controller.dismiss();
  assert.equal(state.info, null);
  stale.resolve(info);
  await pendingRead;
  assert.deepEqual(state, EMPTY_UPGRADE_NOTICE);
});

test("read failures preserve an existing notice and no-upgrade state cannot acknowledge", async () => {
  const commands = [];
  mockIPC((command) => { commands.push(command); return null; });
  let state = EMPTY_UPGRADE_NOTICE;
  const controller = new UpgradeNoticeController((next) => { state = next; });
  await controller.load();
  await controller.dismiss();
  assert.deepEqual(commands, ["get_upgrade_info"]);
  mockIPC(() => info);
  await controller.load();
  mockIPC(() => { throw new Error("read unavailable"); });
  await controller.load();
  assert.deepEqual(state, { info, busy: false, failed: false });
});

test("disposed banner controllers ignore pending reads and acknowledgements", async () => {
  const read = deferred();
  let updates = 0;
  mockIPC(() => read.promise);
  const controller = new UpgradeNoticeController(() => { ++updates; });
  const loading = controller.load();
  controller.dispose();
  read.resolve(info);
  await loading;
  assert.equal(updates, 0);
  const ack = deferred();
  mockIPC((command) => command === "get_upgrade_info" ? info : ack.promise);
  const other = new UpgradeNoticeController(() => { ++updates; });
  await other.load();
  const pendingAck = other.dismiss();
  other.dispose();
  const before = updates;
  ack.resolve(null);
  await pendingAck;
  assert.equal(updates, before);
});
