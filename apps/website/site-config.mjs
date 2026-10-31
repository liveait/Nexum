export function normalizeBasePath(value = "/") {
  if (value === "" || value === "/") return "/";
  if (!/^\/(?:[A-Za-z0-9_-]+\/)*[A-Za-z0-9_-]+\/?$/.test(value)) {
    throw new Error("SITE_BASE_PATH must be a local path such as /Nexum/");
  }
  return value.endsWith("/") ? value : `${value}/`;
}

export function productionBuild(env) {
  return env.GITHUB_ACTIONS === "true"
    && env.GITHUB_REF === "refs/heads/main"
    && ["push", "workflow_dispatch"].includes(env.GITHUB_EVENT_NAME);
}

export function buildSettings(env, args = []) {
  const production = productionBuild(env);
  const explicitSync = args.includes("--sync-release");
  if (env.GITHUB_ACTIONS === "true" && explicitSync && !production) {
    throw new Error("Only main push/dispatch may synchronize production downloads");
  }
  return {
    basePath: normalizeBasePath(env.SITE_BASE_PATH),
    syncRelease: production || explicitSync,
  };
}
