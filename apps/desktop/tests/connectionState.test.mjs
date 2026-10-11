import assert from "node:assert/strict";
import test from "node:test";
import { isConnectionReady } from "../src/connectionState.ts";

test("managed requests wait for the running child, including bounded restart and failure states", () => {
  for (const state of ["starting", "restarting", "failed", "stopped", "external"]) {
    assert.equal(isConnectionReady("managed", { mode: "managed", state, generation: 1, error: null }), false);
  }
  assert.equal(isConnectionReady("managed", null), false);
  assert.equal(isConnectionReady("managed", { mode: "managed", state: "running", generation: 2, error: null }), true);
});

test("mode transitions never treat an old route as ready", () => {
  assert.equal(isConnectionReady("managed", { mode: "external", state: "external", generation: 1, error: null }), false);
  assert.equal(isConnectionReady("external", { mode: "managed", state: "running", generation: 1, error: null }), false);
  assert.equal(isConnectionReady("external", { mode: "external", state: "external", generation: 1, error: null }), true);
});
