import assert from "node:assert/strict";
import test from "node:test";
import { addDownload, invokeTaskCommand } from "../src/taskCommands.ts";
import { taskDisplayName } from "../src/taskPresentation.ts";

test("adding a download lets the backend choose its ID and destination before queueing", async () => {
  const calls = [];
  const invoke = async (command, args) => {
    calls.push({ command, args });
    if (command === "task_create") return { id: "backend-id", destination: "/tmp/a (1).zip" };
    return true;
  };

  const result = await addDownload(invoke, "https://example.com/a.zip", "a.zip", "/tmp");
  assert.deepEqual(result, { id: "backend-id", destination: "/tmp/a (1).zip", queued: true });
  assert.deepEqual(calls, [
    { command: "task_create", args: { source: "https://example.com/a.zip", filename: "a.zip", directory: "/tmp" } },
    { command: "task_queue", args: { taskId: "backend-id" } },
  ]);
});

test("queue failure keeps the backend ID available without creating a second task", async () => {
  const calls = [];
  const failure = new Error("queue unavailable");
  const result = await addDownload(async (command, args) => {
    calls.push({ command, args });
    if (command === "task_queue") throw failure;
    return { id: "created-id", destination: "/tmp/a.zip" };
  }, "https://example.com/a.zip", "a.zip", "/tmp");

  assert.equal(result.queued, false);
  assert.equal(result.id, "created-id");
  assert.equal(result.error, failure);
  assert.deepEqual(calls.map(({ command }) => command), ["task_create", "task_queue"]);
  assert.equal(calls[1].args.taskId, "created-id");
});

test("an invalid create response never queues an unspecified task", async () => {
  const calls = [];
  await assert.rejects(addDownload(async (command, args) => {
    calls.push({ command, args });
    return {};
  }, "https://example.com/a.zip", "a.zip", "/tmp"), /could not be created/);
  assert.deepEqual(calls.map(({ command }) => command), ["task_create"]);
});

test("every row action passes only taskId to the backend's current connection", async () => {
  for (const command of ["task_queue", "task_pause", "task_resume", "task_remove"]) {
    const calls = [];
    await invokeTaskCommand(async (name, args) => { calls.push({ name, args }); return true; }, command, "hidden-id");
    assert.deepEqual(calls, [{ name: command, args: { taskId: "hidden-id" } }]);
  }
});

test("download titles use destination filename, then source filename or host", () => {
  assert.equal(taskDisplayName({ destination: "/Users/me/Downloads/report.zip", source: "https://example.com/original.zip" }), "report.zip");
  assert.equal(taskDisplayName({ destination: "", source: "https://example.com/%E6%8A%A5%E5%91%8A.zip" }), "报告.zip");
  assert.equal(taskDisplayName({ destination: "", source: "https://example.com/" }), "example.com");
  assert.equal(taskDisplayName({ destination: "/", source: "magnet:?xt=abc" }), null);
});
