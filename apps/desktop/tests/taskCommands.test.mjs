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

test("retrying a created download only queues its original ID and preserves its allocated path", async () => {
  const calls = [];
  const result = await addDownload(async (command, args) => {
    calls.push({ command, args });
    if (command === "task_create") return { id: "duplicate-id", destination: "/tmp/a (2).zip" };
    return true;
  }, "https://example.com/a.zip", "a.zip", "/tmp", {
    created: { id: "created-id", destination: "/tmp/a (1).zip" },
  });
  assert.deepEqual(result, { id: "created-id", destination: "/tmp/a (1).zip", queued: true });
  assert.deepEqual(calls, [{ command: "task_queue", args: { taskId: "created-id" } }]);
});

test("a context invalidated while creation is pending cannot queue on the next connection", async () => {
  const calls = [];
  let current = true;
  await assert.rejects(addDownload(async (command, args) => {
    calls.push({ command, args });
    current = false;
    return { id: "old-context-id" };
  }, "https://example.com/a.zip", "a.zip", "/tmp", { isCurrent: () => current }), /connection changed/i);
  assert.deepEqual(calls.map(({ command }) => command), ["task_create"]);
});

test("queue refusal remains retryable instead of reporting a queued download", async () => {
  const result = await addDownload(async (command) => command === "task_create" ? { id: "created-id" } : false,
    "https://example.com/a.zip", "a.zip", "/tmp");
  assert.equal(result.queued, false);
  assert.equal(result.id, "created-id");
});

test("creation is retained before the first queue attempt and survives repeated queue failures", async () => {
  let created = null;
  const calls = [];
  let queueAttempts = 0;
  const invoke = async (command, args) => {
    calls.push({ command, args });
    if (command === "task_create") return { id: "created-once", destination: "/tmp/a (1).zip" };
    assert.equal(created?.id, "created-once");
    if (++queueAttempts < 3) throw new Error("temporarily unavailable");
    return true;
  };
  for (let attempt = 0; attempt < 3; attempt += 1) {
    const result = await addDownload(invoke, "https://example.com/a.zip", "a.zip", "/tmp", {
      created,
      onCreated: (value) => { created = value; },
    });
    assert.equal(result.queued, attempt === 2);
    assert.equal(result.id, "created-once");
  }
  assert.deepEqual(calls.map(({ command }) => command), ["task_create", "task_queue", "task_queue", "task_queue"]);
  assert.ok(calls.slice(1).every(({ args }) => args.taskId === "created-once"));
});

test("a closed or replaced Add attempt cannot retry its old ID", async () => {
  let calls = 0;
  await assert.rejects(addDownload(async () => { calls += 1; return true; }, "https://example.com/a.zip", "a.zip", "/tmp", {
    created: { id: "old-id" }, isCurrent: () => false,
  }), /connection changed/i);
  assert.equal(calls, 0);
});

test("invalidating an attempt when its created ID is retained still prevents queueing", async () => {
  const calls = [];
  let current = true;
  const result = await addDownload(async (command) => {
    calls.push(command);
    return { id: "old-id" };
  }, "https://example.com/a.zip", "a.zip", "/tmp", {
    isCurrent: () => current,
    onCreated: () => { current = false; },
  });
  assert.equal(result.queued, false);
  assert.match(result.error.message, /connection changed/i);
  assert.deepEqual(calls, ["task_create"]);
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
