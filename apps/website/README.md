# Nexum 官网 / Nexum website

无第三方依赖的中英文静态站。正式下载默认关闭；源码和预览不代表 M3 已验收。

A bilingual static site without third-party dependencies. Official downloads are disabled by default; source and previews do not establish M3 acceptance.

## 构建与验证 / Build and verification

在仓库根目录运行，产物为 `apps/website/dist`；没有包依赖，无需安装或锁文件。预览可用 `python3 -m http.server 4175 --directory apps/website/dist`。

Run from the repository root; output is `apps/website/dist`. There are no package dependencies to install or lock. Preview with `python3 -m http.server 4175 --directory apps/website/dist`.

```bash
node --test apps/website/*.test.mjs
node apps/website/build.mjs
```

## GitHub Pages 配置 / GitHub Pages configuration

官网使用本仓库的 GitHub Pages 项目站点，默认地址为 `https://liveait.github.io/Nexum/`，中英文路径为 `/Nexum/zh/` 和 `/Nexum/en/`。在仓库 Settings → Pages 将 Source 设为 GitHub Actions。可后续配置独立域名；当前构建通过 `SITE_BASE_PATH` 支持项目子路径和域名根路径。

The website uses this repository's GitHub Pages project site at `https://liveait.github.io/Nexum/`, with `/Nexum/zh/` and `/Nexum/en/`. Select GitHub Actions under Settings → Pages. A custom domain can follow later; `SITE_BASE_PATH` supports both project prefixes and domain roots.

`.github/workflows/website.yml` 在 PR 中只构建并上传 `website-preview` 产物，不部署生产。只有 `main` 的 push 或手动 workflow_dispatch 才上传 Pages 产物；构建成功后，`main-ci` 屏障确认同仓库、同 SHA 的 `ci.yml` main push 验证，再允许进入 `github-pages` environment 部署。生产使用独立并发组，不被 PR 取消；deploy job 独占 `pages: write`／`id-token: write`。远端 environment 的分支规则需另行核实，源码不配置或绕过它。工作流进入 main 并首次部署通过前，不宣称官网已上线。

The website workflow only builds/uploads `website-preview` artifacts for PRs. Only main push or main workflow_dispatch uploads a Pages artifact. After the build succeeds, the main-ci gate verifies the same repository/SHA's ci.yml main push before allowing deployment through github-pages. Its separate concurrency group prevents PRs from cancelling production; only deploy receives Pages/OIDC write permissions. Verify remote environment branch rules separately; source neither configures nor bypasses them. The website is not live until the workflow is on main and deployment succeeds.

屏障使用只读 Actions API，最多等待 30 分钟，每 15 秒查询一次。它核对 run 的仓库 ID／名称、来源仓库、SHA、main 分支、push 事件和 workflow 路径，选择该 SHA 最新的 run，并读取其当前 attempt 的 jobs。Rust、Desktop macOS 及 `ci.yml` 中全部命名步骤必须存在且为 completed/success，其他实际步骤也不得失败或跳过；只看 workflow 总成功不足以放行。CI 的 job／步骤名称改变时需同步此契约。仅屏障 job 获得 `actions: read`，API token 不进入构建产物或日志。

The gate uses read-only Actions APIs, polling every 15 seconds for at most 30 minutes. It verifies repository ID/name, source repository, SHA, main branch, push event and workflow path, selects that SHA's latest run and reads its current attempt's jobs. Rust, Desktop macOS and every named ci.yml step must exist with completed/success; other actual steps must also pass. Workflow success alone is insufficient. Update this contract when CI job/step names change. Only the gate job receives actions:read; the API token never enters build artifacts or logs.

放行前再次核对 run／attempt，并确认 main ref 仍指向构建 SHA。缺失或未结束的 CI 等待到超时；失败、取消、跳过、证据不完整、API 错误或 main 已前进均阻止部署。先修复／重跑 CI，再从最新 main 手动运行 Website；不得自动切换 SHA 后复用旧产物。这一屏障不能替代 M3 分发验收。

Before permitting deployment, the gate rechecks the run/attempt and confirms main still points to the build SHA. Missing or pending CI waits until timeout; failure, cancellation, skipped or incomplete evidence, API errors or main advancing blocks deployment. Fix/rerun CI, then dispatch Website from latest main. Never switch SHAs while reusing an old artifact. This gate does not replace M3 distribution acceptance.

API 契约见 [workflow runs](https://docs.github.com/en/rest/actions/workflow-runs#list-workflow-runs-for-a-workflow)、[attempt jobs](https://docs.github.com/en/rest/actions/workflow-jobs#list-jobs-for-a-workflow-run-attempt) 和 [Git refs](https://docs.github.com/en/rest/git/refs#get-a-reference)。

See the official API contracts for [workflow runs](https://docs.github.com/en/rest/actions/workflow-runs#list-workflow-runs-for-a-workflow), [attempt jobs](https://docs.github.com/en/rest/actions/workflow-jobs#list-jobs-for-a-workflow-run-attempt) and [Git refs](https://docs.github.com/en/rest/git/refs#get-a-reference).

本地模拟项目路径时，使用以下构建；HTTP 服务需要把 dist 挂载到 `/Nexum/`，不能只在 `/` 打开页面。默认不设置变量仍支持根路径本地预览。

Use this build to simulate the project prefix locally; the HTTP server must mount dist at `/Nexum/`. The default without the variable still supports root-path previews.

```bash
SITE_BASE_PATH=/Nexum/ node apps/website/build.mjs
```

参考 [GitHub Pages 快速入门](https://docs.github.com/zh/pages/quickstart)和[自定义 Actions 工作流](https://docs.github.com/zh/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages)。

See the [GitHub Pages quickstart](https://docs.github.com/en/pages/quickstart) and [custom Actions workflows](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages).

## 现有品牌图标 / Existing brand icon

页头和页尾使用透明底 `apps/desktop/src-tauri/icons/brand-mark.png`；favicon 使用已选石墨灰底应用图标 `icon.png`，Apple touch 使用同目录独立生成的 180px `apple-touch-icon.png`。两种用途保留同一 N 识别，源图、生成命令和哈希见[品牌资源](../../docs/BRANDING.md)。

Header/footer use the transparent `brand-mark.png`; favicon uses the selected graphite App tile `icon.png`, while Apple touch uses its separately generated 180px derivative. Both variants retain the N identity. See [Branding](../../docs/BRANDING.md) for sources, generation and hashes.

官网界面图 `assets/downloads-native.jpg` 是隔离的 Nexum Brand QA 原生 release 候选截图，源码为 `9965cd25c94fde23d3dfb775b67c4be6f63130a0`（Desktop 源码与 PR #69 的 `df2042d` 相同）。截图原字节保留，展示 24px 透明 N 侧栏和唯一的顶部新增入口；不代表最终签名分发产物。

The website image `assets/downloads-native.jpg` is an unaltered capture of the isolated Nexum Brand QA native release candidate at `9965cd25c94fde23d3dfb775b67c4be6f63130a0` (the same Desktop source as PR #69 at `df2042d`). It shows the 24px transparent N sidebar and single toolbar Add entry. It is not the final signed distribution artifact.

## 发布元数据 / Release metadata

仅 GitHub Actions 的 main push／dispatch 或显式 `node apps/website/build.mjs --sync-release` 在构建时读取公开 GitHub latest release，浏览器不接收私有 token。无 release（404）显示待发布；网络／API 或元数据错误使构建失败，避免发布错误下载。稳定 release 发布后，独立的 website-release 工作流只请求 website.yml 在 main 执行，不 checkout 或执行 tag 源码；保持 main 的 OIDC／environment 分支约束。也可手动在 main 运行 website.yml。非 main 的 Actions 即使传入同步参数也会失败，PR 不能解锁正式下载。

Only main push/dispatch in GitHub Actions or explicit `node apps/website/build.mjs --sync-release` reads the public GitHub latest release at build time; browsers receive no private token. A missing release (404) shows pending status; network/API or metadata errors fail the build to avoid publishing incorrect downloads. After a stable release, the separate website-release workflow dispatches website.yml on main without checking out/executing tag source, retaining main OIDC/environment restrictions. Manual dispatch on main also works. Non-main Actions reject explicit sync arguments; PRs cannot unlock downloads.

发布者在通过 M3 后随稳定 release 上传 `nexum-release.json`。它是发布者的验收声明，不能替代签名／公证检查。安装包大小和 SHA256 必须与 [GitHub Release assets](https://docs.github.com/en/rest/releases/assets) 的 `size`、`digest` 匹配；下载 URL 仅允许本仓库该版本的产物。未验收架构不填入数组。示例字段值是占位，不能直接发布。

After M3 passes, the publisher uploads `nexum-release.json` with a stable release. This is a publisher acceptance attestation, not a substitute for signing/notarization checks. Installer sizes and SHA256 values must match `size` and `digest` from [GitHub Release assets](https://docs.github.com/en/rest/releases/assets); download URLs must belong to this repository and release. Omit unvalidated architectures. Example values are placeholders, not publishable metadata.

```json
{
  "schema_version": 1,
  "version": "0.1.0",
  "source_sha": "REPLACE_WITH_40_CHARACTER_SOURCE_SHA",
  "m3_accepted": true,
  "artifacts": [{
    "os": "macos",
    "arch": "aarch64",
    "minimum_os": "REPLACE_WITH_VALIDATED_MINIMUM",
    "name": "Nexum-0.1.0-aarch64.dmg",
    "size": 1,
    "sha256": "REPLACE_WITH_64_CHARACTER_SHA256",
    "signing": "developer-id",
    "notarized": true,
    "gatekeeper_verified": true
  }]
}
```
