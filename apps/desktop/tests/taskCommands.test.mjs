import assert from "node:assert/strict";
import test from "node:test";
import { addDownload, invokeTaskCommand } from "../src/taskCommands.ts";
import { taskDisplayName } from "../src/taskPresentation.ts";

test("adding a download creates and queues the same generated task with Tauri's camelCase argument", async () => {
  const calls = [];
  const invoke = async (command, args) => {
    calls.push({ command, args });
    if (command === "task_queue") {
      assert.equal(typeof args.taskId, "string");
      assert.equal(args.task_id, undefined);
    }
    return true;
  };

  const first = await addDownload(invoke, "127.0.0.1:39100", "https://example.com/a.zip", "/tmp/a.zip");
  const second = await addDownload(invoke, "127.0.0.1:39100", "https://example.com/a.zip", "/tmp/b.zip");

  assert.equal(first.queued, true);
  assert.equal(second.queued, true);
  assert.notEqual(first.id, second.id);
  assert.match(first.id, /^download-[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  assert.deepEqual(calls.map(({ command }) => command), ["task_create", "task_queue", "task_create", "task_queue"]);
  assert.equal(calls[0].args.id, first.id);
  assert.equal(calls[1].args.taskId, first.id);
  assert.equal(calls[2].args.id, second.id);
  assert.equal(calls[3].args.taskId, second.id);
});

test("queue failure keeps the generated ID available without creating a second task", async () => {
  const calls = [];
  const failure = new Error("queue unavailable");
  const result = await addDownload(async (command, args) => {
    calls.push({ command, args });
    if (command === "task_queue") throw failure;
  }, "127.0.0.1:39100", "https://example.com/a.zip", "/tmp/a.zip");

  assert.equal(result.queued, false);
  assert.equal(result.error, failure);
  assert.deepEqual(calls.map(({ command }) => command), ["task_create", "task_queue"]);
  assert.equal(calls[1].args.taskId, calls[0].args.id);
});

test("every row task action passes taskId to Tauri", async () => {
  for (const command of ["task_queue", "task_pause", "task_resume", "task_remove"]) {
    const calls = [];
    await invokeTaskCommand(async (name, args) => { calls.push({ name, args }); return true; }, command, "server", "hidden-id");
    assert.deepEqual(calls, [{ name: command, args: { server: "server", taskId: "hidden-id" } }]);
  }
});

test("download titles use destination filename, then source filename or host", () => {
  assert.equal(taskDisplayName({ destination: "/Users/me/Downloads/report.zip", source: "https://example.com/original.zip" }), "report.zip");
  assert.equal(taskDisplayName({ destination: "", source: "https://example.com/%E6%8A%A5%E5%91%8A.zip" }), "报告.zip");
  assert.equal(taskDisplayName({ destination: "", source: "https://example.com/" }), "example.com");
  assert.equal(taskDisplayName({ destination: "/", source: "magnet:?xt=abc" }), null);
});
