import assert from "node:assert/strict";
import test from "node:test";
import { valueForContext } from "../src/taskContext.ts";

test("a Server change hides its old task snapshot and selection until the new snapshot arrives", () => {
  const firstServer = { server: "127.0.0.1:39100", credentialRevision: 0 };
  const secondServer = { server: "127.0.0.1:39101", credentialRevision: 0 };
  const oldTasks = { context: firstServer, value: [{ id: "shared-id", destination: "/old/file" }] };
  const oldSelection = { context: firstServer, value: "shared-id" };

  assert.equal(valueForContext(oldTasks, firstServer)?.[0].destination, "/old/file");
  assert.equal(valueForContext(oldTasks, secondServer), null);
  assert.equal(valueForContext(oldSelection, secondServer), null);

  const newTasks = { context: secondServer, value: [{ id: "shared-id", destination: "/new/file" }] };
  assert.equal(valueForContext(newTasks, secondServer)?.[0].destination, "/new/file");
});

test("a credential change and a return to the same address require a fresh snapshot", () => {
  const original = { server: "127.0.0.1:39100", credentialRevision: 0 };
  const changedCredential = { server: "127.0.0.1:39100", credentialRevision: 1 };
  const returnedToAddress = { server: "127.0.0.1:39100", credentialRevision: 0 };
  const oldTasks = { context: original, value: [{ id: "old-task" }] };

  assert.equal(valueForContext(oldTasks, changedCredential), null);
  assert.equal(valueForContext(oldTasks, returnedToAddress), null);
  assert.deepEqual(valueForContext({ context: returnedToAddress, value: [{ id: "fresh-task" }] }, returnedToAddress), [{ id: "fresh-task" }]);
});
