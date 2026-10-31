import test from "node:test";
import assert from "node:assert/strict";
import { gateConfig, waitForMainCi } from "./main-ci-gate.mjs";

const sha = "a".repeat(40);
const env = { GITHUB_REF: "refs/heads/main", GITHUB_EVENT_NAME: "push", GITHUB_REPOSITORY: "liveait/Nexum", GITHUB_REPOSITORY_ID: "123", GITHUB_SHA: sha, GH_TOKEN: "private-test-token" };
const completed = { status: "completed", conclusion: "success" };
const run = (changes = {}) => ({ id: 42, run_attempt: 1, head_sha: sha, head_branch: "main", event: "push", path: ".github/workflows/ci.yml", repository: { id: 123, full_name: "liveait/Nexum" }, head_repository: { id: 123, full_name: "liveait/Nexum" }, ...completed, ...changes });
// Names reflect the real ci.yml/API contract, independently of the gate's list.
const stepNames = {
  Rust: ["Checkout", "Install Rust", "Install system deps (Tauri)", "Check formatting", "Check", "Test", "Test Server managed tests", "Clippy"],
  "Desktop macOS": ["Checkout", "Install pnpm", "Install Node.js", "Install Rust", "Install Desktop dependencies", "Build Desktop frontend", "Test Desktop task flow", "Build macOS app bundle", "Verify macOS app bundle"],
};
const jobs = (attempt = 1) => Object.entries(stepNames).map(([name, names], i) => ({
  id: 100 + i, run_id: 42, run_attempt: attempt, head_sha: sha, head_branch: "main", name, ...completed,
  steps: ["Set up job", ...names, "Complete job"].map((name, number) => ({ name, number, ...completed })),
}));

function backend({ runs = [run()], jobList = jobs(), confirmation = run(), main = { ref: "refs/heads/main", object: { type: "commit", sha } } } = {}) {
  const calls = [];
  let polls = 0;
  let confirming = false;
  const value = (entry) => typeof entry === "function" ? entry(polls) : entry;
  const fetchImpl = async (url, init) => {
    calls.push(url);
    assert.equal(url.origin, "https://api.github.com");
    assert.equal(init.method, "GET");
    assert.equal(init.redirect, "error");
    assert.equal(init.headers.Authorization, `Bearer ${env.GH_TOKEN}`);
    assert.ok(init.signal instanceof AbortSignal);
    let data;
    if (url.pathname.endsWith("/actions/workflows/ci.yml/runs")) {
      assert.equal(url.searchParams.get("branch"), "main");
      assert.equal(url.searchParams.get("event"), "push");
      assert.equal(url.searchParams.get("head_sha"), sha);
      assert.equal(url.searchParams.get("per_page"), "100");
      if (!confirming) polls += 1;
      const list = confirming ? [value(confirmation)] : value(runs);
      confirming = false;
      data = Array.isArray(list) ? { total_count: list.length, workflow_runs: list } : list;
    } else if (/\/actions\/runs\/42\/attempts\/\d+\/jobs$/.test(url.pathname)) {
      assert.equal(url.searchParams.get("per_page"), "100");
      const list = value(jobList);
      confirming = true;
      data = Array.isArray(list) ? { total_count: list.length, jobs: list } : list;
    } else if (url.pathname.endsWith("/git/ref/heads/main")) data = value(main);
    else assert.fail(`Unexpected API request ${url}`);
    return { ok: true, status: 200, json: async () => structuredClone(data) };
  };
  return { fetchImpl, calls };
}

function clock() {
  let time = 0;
  return { now: () => time, sleep: async (ms) => { time += ms; }, timeoutMs: 100, pollMs: 10 };
}

test("gate rejects non-main/non-production context and invalid identity or credentials", () => {
  for (const changes of [{ GITHUB_REF: "refs/pull/1/merge" }, { GITHUB_REF: "refs/heads/feature" }, { GITHUB_EVENT_NAME: "pull_request" }, { GITHUB_EVENT_NAME: "release" }, { GITHUB_SHA: "short" }, { GITHUB_REPOSITORY: "https://evil.example/repo" }, { GITHUB_REPOSITORY: "../repo" }, { GITHUB_REPOSITORY: "owner/.." }, { GITHUB_REPOSITORY_ID: "0" }, { GH_TOKEN: "" }]) {
    assert.throws(() => gateConfig({ ...env, ...changes }));
  }
  assert.equal(gateConfig({ ...env, GITHUB_EVENT_NAME: "workflow_dispatch" }).sha, sha);
});

test("accepts only a same-repository main push with real successful jobs/steps and current main", async () => {
  const api = backend();
  const accepted = await waitForMainCi(env, { ...api, ...clock() });
  assert.deepEqual(accepted, { id: 42, attempt: 1, sha, url: "https://github.com/liveait/Nexum/actions/runs/42" });
  assert.equal(api.calls.length, 4);
  assert.ok(api.calls[1].pathname.endsWith("/runs/42/attempts/1/jobs"));
  assert.equal(api.calls[2].href, api.calls[0].href);
  assert.ok(api.calls[3].pathname.endsWith("/git/ref/heads/main"));
});

test("API filters cannot substitute for verifying SHA, branch, event, repository and workflow", async () => {
  for (const change of [
    { head_sha: "b".repeat(40) }, { head_branch: "feature" }, { event: "pull_request" },
    { repository: { id: 999, full_name: "liveait/Nexum" } }, { repository: { id: 123, full_name: "foreign/Nexum" } },
    { head_repository: { id: 999, full_name: "liveait/Nexum" } }, { head_repository: null },
    { path: ".github/workflows/other.yml" }, { path: ".github/workflows/ci.yml@feature" }, { id: 0 }, { run_attempt: 0 },
  ]) await assert.rejects(waitForMainCi(env, { ...backend({ runs: [run(change)] }), ...clock() }), /identity/);
});

test("a newer failed run cannot fall back to an older successful run of the same SHA", async () => {
  await assert.rejects(waitForMainCi(env, { ...backend({ runs: [run(), run({ id: 43, conclusion: "failure" })] }), ...clock() }), /completed\/success/);
});

test("failed, cancelled, skipped, neutral or incomplete workflow conclusions block deployment", async () => {
  for (const conclusion of ["failure", "cancelled", "skipped", "timed_out", "neutral", "action_required", null]) {
    await assert.rejects(waitForMainCi(env, { ...backend({ runs: [run({ conclusion })] }), ...clock() }), /completed\/success/);
  }
  await assert.rejects(waitForMainCi(env, { ...backend({ runs: [run({ status: "unknown" })] }), ...clock() }), /completed\/success/);
});

test("workflow success cannot hide a missing or unsuccessful required job", async () => {
  for (let i = 0; i < 2; i += 1) {
    await assert.rejects(waitForMainCi(env, { ...backend({ jobList: jobs().filter((_, j) => i !== j) }), ...clock() }), /exactly one/);
    for (const conclusion of ["failure", "cancelled", "skipped", "timed_out", null]) {
      const list = jobs(); list[i].conclusion = conclusion;
      await assert.rejects(waitForMainCi(env, { ...backend({ jobList: list }), ...clock() }), /completed\/success/);
    }
    const list = jobs(); list[i].status = "in_progress";
    await assert.rejects(waitForMainCi(env, { ...backend({ jobList: list }), ...clock() }), /completed\/success/);
  }
});

test("every named CI step is required even when its job and workflow report success", async () => {
  for (let i = 0; i < 2; i += 1) {
    for (const name of stepNames[jobs()[i].name]) {
      const missing = jobs(); missing[i].steps = missing[i].steps.filter((step) => step.name !== name);
      await assert.rejects(waitForMainCi(env, { ...backend({ jobList: missing }), ...clock() }), /missing required step/);
      for (const conclusion of ["failure", "cancelled", "skipped", null]) {
        const list = jobs(); list[i].steps.find((step) => step.name === name).conclusion = conclusion;
        await assert.rejects(waitForMainCi(env, { ...backend({ jobList: list }), ...clock() }), /completed\/success/);
      }
    }
  }
});

test("wrong run/attempt/SHA/branch and duplicate or absent step evidence are rejected", async () => {
  for (const change of [{ id: 0 }, { run_id: 99 }, { run_attempt: 2 }, { head_sha: "b".repeat(40) }, { head_branch: "feature" }]) {
    const list = jobs(); Object.assign(list[0], change);
    await assert.rejects(waitForMainCi(env, { ...backend({ jobList: list }), ...clock() }), /identity/);
  }
  for (const mutate of [
    (list) => list.push(structuredClone(list[0])),
    (list) => { list[1].id = list[0].id; },
    (list) => { list[0].steps = []; },
    (list) => list[0].steps.push({ name: "Test", ...completed }),
    (list) => list[0].steps.push({ name: "Extra validation", status: "completed", conclusion: "skipped" }),
  ]) {
    const list = jobs(); mutate(list);
    await assert.rejects(waitForMainCi(env, { ...backend({ jobList: list }), ...clock() }));
  }
});

test("incomplete or oversized API listings never hide absent CI evidence", async () => {
  for (const runs of [{ total_count: 1, workflow_runs: [] }, { total_count: 101, workflow_runs: [run()] }, { total_count: 1, workflow_runs: null }, [run(), run()]]) {
    await assert.rejects(waitForMainCi(env, { ...backend({ runs }), ...clock() }), /listing/);
  }
  for (const jobList of [{ total_count: 3, jobs: jobs() }, { total_count: 101, jobs: jobs() }, { total_count: 2, jobs: null }]) {
    await assert.rejects(waitForMainCi(env, { ...backend({ jobList }), ...clock() }), /listing/);
  }
});

test("missing then queued/running CI is polled until success within the deadline", async () => {
  const states = [[], [run({ status: "queued", conclusion: null })], [run({ status: "queued", conclusion: null })], [run({ status: "in_progress", conclusion: null })], [run()]];
  const api = backend({ runs: (poll) => states[poll - 1] });
  const progress = []; const time = clock();
  assert.equal((await waitForMainCi(env, { ...api, ...time, onProgress: (message) => progress.push(message) })).id, 42);
  assert.equal(time.now(), 40);
  assert.equal(progress.length, 3);
  assert.equal(api.calls.filter((url) => url.pathname.endsWith("/jobs")).length, 1);
});

test("a rerun started after reading jobs waits for the new attempt's own evidence", async () => {
  const api = backend({
    runs: (poll) => [run({ run_attempt: poll === 1 ? 1 : 2 })],
    jobList: (poll) => jobs(poll === 1 ? 1 : 2),
    confirmation: (poll) => run({ run_attempt: 2, ...(poll === 1 ? { status: "queued", conclusion: null } : {}) }),
  });
  const result = await waitForMainCi(env, { ...api, ...clock() });
  assert.equal(result.attempt, 2);
  assert.deepEqual(api.calls.filter((url) => url.pathname.endsWith("/jobs")).map((url) => url.pathname.split("/").at(-2)), ["1", "2"]);
});

test("missing or pending CI times out without accepting another SHA or starting deployment", async () => {
  for (const runs of [[], [run({ status: "in_progress", conclusion: null })]]) {
    const api = backend({ runs }); const time = clock();
    await assert.rejects(waitForMainCi(env, { ...api, ...time, timeoutMs: 30 }), /Timed out.*dispatch Website/);
    assert.equal(time.now(), 30);
    assert.equal(api.calls.length, 3);
  }
});

test("confirmation failure or main advancing rejects the old build", async () => {
  for (const confirmation of [run({ conclusion: "failure" }), run({ id: 43 }), run({ event: "workflow_dispatch" })]) {
    await assert.rejects(waitForMainCi(env, { ...backend({ confirmation }), ...clock() }));
  }
  for (const main of [{ ref: "refs/heads/main", object: { type: "commit", sha: "b".repeat(40) } }, { ref: "refs/heads/feature", object: { type: "commit", sha } }, { ref: "refs/heads/main", object: { type: "tag", sha } }]) {
    await assert.rejects(waitForMainCi(env, { ...backend({ main }), ...clock() }), /Dispatch Website from latest main/);
  }
});

test("API errors fail closed with actionable messages without disclosing the token or body", async () => {
  const cases = [
    ...[401, 403, 500].map((status) => async () => ({ ok: false, status, json: async () => ({ message: env.GH_TOKEN }) })),
    async () => { throw new Error(env.GH_TOKEN); },
    async () => ({ ok: true, json: async () => { throw new Error(env.GH_TOKEN); } }),
  ];
  for (const fetchImpl of cases) {
    await assert.rejects(waitForMainCi(env, { fetchImpl, ...clock() }), (error) => {
      assert.doesNotMatch(error.message, new RegExp(env.GH_TOKEN));
      assert.match(error.message, /no Pages deployment was permitted/);
      return true;
    });
  }
});

test("a response completing beyond the overall deadline cannot pass", async () => {
  const api = backend(); let time = 0;
  const fetchImpl = async (url, init) => {
    const response = await api.fetchImpl(url, init);
    if (url.pathname.endsWith("/git/ref/heads/main")) time = 101;
    return response;
  };
  await assert.rejects(waitForMainCi(env, { fetchImpl, now: () => time, sleep: async () => {}, timeoutMs: 100 }), /Timed out/);
});
