import { pathToFileURL } from "node:url";

const REQUIRED_STEPS = {
  Rust: ["Checkout", "Install Rust", "Install system deps (Tauri)", "Check formatting", "Check", "Test", "Test Server managed tests", "Clippy"],
  "Desktop macOS": ["Checkout", "Install pnpm", "Install Node.js", "Install Rust", "Install Desktop dependencies", "Build Desktop frontend", "Test Desktop task flow", "Build macOS app bundle", "Verify macOS app bundle"],
};
const PENDING = new Set(["queued", "requested", "waiting", "pending", "in_progress"]);
const positiveInteger = (value) => Number.isSafeInteger(value) && value > 0;

export function gateConfig(env) {
  if (env.GITHUB_REF !== "refs/heads/main" || !["push", "workflow_dispatch"].includes(env.GITHUB_EVENT_NAME)) {
    throw new Error("The Pages CI gate only runs for main push or main workflow_dispatch.");
  }
  if (!/^[a-z0-9][a-z0-9-]*\/[\w.-]+$/i.test(env.GITHUB_REPOSITORY ?? "")
    || [".", ".."].includes(env.GITHUB_REPOSITORY.split("/")[1]) || !/^[a-f0-9]{40}$/.test(env.GITHUB_SHA ?? "")) {
    throw new Error("A valid GITHUB_REPOSITORY and full GITHUB_SHA are required.");
  }
  if (!/^\d+$/.test(env.GITHUB_REPOSITORY_ID ?? "") || !positiveInteger(Number(env.GITHUB_REPOSITORY_ID))) {
    throw new Error("A valid GITHUB_REPOSITORY_ID is required.");
  }
  if (!env.GH_TOKEN) throw new Error("GH_TOKEN with actions:read and contents:read is required for the Pages CI gate.");
  return { repository: env.GITHUB_REPOSITORY, repositoryId: Number(env.GITHUB_REPOSITORY_ID), sha: env.GITHUB_SHA, token: env.GH_TOKEN };
}

function assertRunIdentity(run, config) {
  const sameRepository = (repo) => repo?.id === config.repositoryId && typeof repo.full_name === "string" && repo.full_name.toLowerCase() === config.repository.toLowerCase();
  if (!positiveInteger(run?.id) || !positiveInteger(run.run_attempt)
    || run.head_sha !== config.sha || run.head_branch !== "main" || run.event !== "push"
    || typeof run.path !== "string" || !/^\.github\/workflows\/ci\.yml(?:@(?:refs\/heads\/)?main)?$/.test(run.path)
    || !sameRepository(run.repository) || !sameRepository(run.head_repository)) {
    throw new Error("CI run identity does not match this repository's main push SHA and ci.yml.");
  }
}

function completedSuccessfully(value, label) {
  if (value.status !== "completed" || value.conclusion !== "success") {
    throw new Error(`${label} must be completed/success; missing, skipped, cancelled or failed validation blocks Pages deployment.`);
  }
}

function assertJobs(jobs, run, config) {
  if (jobs.some((job) => !positiveInteger(job?.id) || typeof job.name !== "string" || job.name.length === 0)) throw new Error("CI job identity is missing.");
  for (const name of Object.keys(REQUIRED_STEPS)) {
    if (jobs.filter((job) => job.name === name).length !== 1) throw new Error(`CI requires exactly one ${name} job.`);
  }
  if (new Set(jobs.map((job) => job.id)).size !== jobs.length) throw new Error("CI job identities are duplicated.");
  for (const job of jobs) {
    if (!positiveInteger(job.id) || job.run_id !== run.id || job.run_attempt !== run.run_attempt
      || job.head_sha !== config.sha || job.head_branch !== "main") throw new Error("CI job identity does not match the accepted run attempt and SHA.");
    completedSuccessfully(job, `CI job ${job.name}`);
    if (!Array.isArray(job.steps) || job.steps.length === 0) throw new Error(`CI job ${job.name} has no step evidence.`);
    for (const name of REQUIRED_STEPS[job.name] ?? []) {
      if (job.steps.filter((step) => step.name === name).length !== 1) throw new Error(`CI job ${job.name} is missing required step ${name}, or contains it more than once.`);
    }
    for (const step of job.steps) completedSuccessfully(step, `CI step ${step.name}`);
  }
}

// Only GET the official API; never follow an API-supplied URL with the token.
export async function waitForMainCi(env = process.env, {
  fetchImpl = fetch,
  now = () => performance.now(),
  sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
  timeoutMs = 30 * 60_000,
  pollMs = 15_000,
  onProgress = () => {},
} = {}) {
  const config = gateConfig(env);
  if (!Number.isFinite(timeoutMs) || timeoutMs <= 0 || !Number.isFinite(pollMs) || pollMs <= 0) throw new Error("CI wait and poll durations must be positive and finite.");
  const deadline = now() + timeoutMs;
  const prefix = `/repos/${config.repository}`;
  const runsPath = `${prefix}/actions/workflows/ci.yml/runs?branch=main&event=push&head_sha=${config.sha}&per_page=100`;
  const timeout = () => new Error("Timed out waiting for this SHA's main push CI. Check CI's Rust/Desktop jobs, then dispatch Website from latest main; no Pages deployment was permitted.");
  async function get(path) {
    if (now() >= deadline) throw timeout();
    let response;
    try {
      response = await fetchImpl(new URL(path, "https://api.github.com"), {
        method: "GET", redirect: "error",
        headers: { Accept: "application/vnd.github+json", Authorization: `Bearer ${config.token}`, "X-GitHub-Api-Version": "2022-11-28" },
        signal: AbortSignal.timeout(Math.max(1, Math.floor(Math.min(20_000, deadline - now())))),
      });
    } catch {
      throw new Error("Cannot read the GitHub Actions API. Check connectivity and retry Website; no Pages deployment was permitted.");
    }
    if (!response.ok) throw new Error(`GitHub API returned HTTP ${response.status}. Check gate actions:read/contents:read permissions and retry Website; no Pages deployment was permitted.`);
    try { return await response.json(); } catch { throw new Error("GitHub API returned invalid JSON; no Pages deployment was permitted."); }
  }
  async function latestRun() {
    const data = await get(runsPath);
    if (!Array.isArray(data?.workflow_runs) || !Number.isSafeInteger(data.total_count) || data.total_count < 0
      || data.total_count > 100 || data.workflow_runs.length !== data.total_count) throw new Error("CI run listing is incomplete or exceeds the 100-run safety bound.");
    for (const run of data.workflow_runs) assertRunIdentity(run, config);
    if (new Set(data.workflow_runs.map((run) => run.id)).size !== data.workflow_runs.length) throw new Error("CI run listing contains duplicate identities.");
    return data.workflow_runs.reduce((latest, run) => !latest || run.id > latest.id ? run : latest, null);
  }
  let previousMessage;
  while (now() < deadline) {
    const run = await latestRun();
    let message = "Waiting for this SHA's main push CI run to appear.";
    if (run) {
      if (PENDING.has(run.status)) message = `Waiting for main CI run ${run.id}, attempt ${run.run_attempt}: ${run.status}.`;
      else {
        completedSuccessfully(run, "Main push CI workflow");
        const data = await get(`${prefix}/actions/runs/${run.id}/attempts/${run.run_attempt}/jobs?per_page=100`);
        if (!Array.isArray(data?.jobs) || data.total_count !== data.jobs.length || data.total_count > 100) throw new Error("CI job listing is incomplete or exceeds the 100-job safety bound.");
        assertJobs(data.jobs, run, config);
        // Re-read the filtered listing: neither a new run nor a rerun may reuse old jobs.
        const confirmed = await latestRun();
        if (!confirmed || confirmed.id !== run.id) throw new Error("The latest CI run changed during validation. Retry Website after main CI completes.");
        if (confirmed.run_attempt !== run.run_attempt || PENDING.has(confirmed.status)) {
          message = "CI was rerun during validation; waiting for its current attempt.";
        } else {
          completedSuccessfully(confirmed, "Confirmed main push CI workflow");
          const main = await get(`${prefix}/git/ref/heads/main`);
          if (main?.ref !== "refs/heads/main" || main.object?.type !== "commit" || main.object.sha !== config.sha) {
            throw new Error("main no longer matches this deployment SHA. Dispatch Website from latest main; the old artifact must not be deployed.");
          }
          if (now() >= deadline) throw timeout();
          return { id: run.id, attempt: run.run_attempt, sha: config.sha, url: `https://github.com/${config.repository}/actions/runs/${run.id}` };
        }
      }
    }
    if (message !== previousMessage) { onProgress(message); previousMessage = message; }
    if (now() >= deadline) throw timeout();
    await sleep(Math.min(pollMs, deadline - now()));
  }
  throw timeout();
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const run = await waitForMainCi(process.env, { onProgress: console.log });
    console.log(`Pages CI gate accepted ${run.url}, attempt ${run.attempt}, SHA ${run.sha}.`);
  } catch (error) {
    console.error(`Pages CI gate: ${error.message}`);
    process.exitCode = 1;
  }
}
