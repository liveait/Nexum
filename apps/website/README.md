# Nexum 官网 / Nexum website

无第三方依赖的中英文静态站。正式下载默认关闭；源码和预览不代表 M3 已验收。

A bilingual static site without third-party dependencies. Official downloads are disabled by default; source and previews do not establish M3 acceptance.

## 构建与验证 / Build and verification

在仓库根目录运行，产物为 `apps/website/dist`；没有包依赖，无需安装或锁文件。预览可用 `python3 -m http.server 4175 --directory apps/website/dist`。

Run from the repository root; output is `apps/website/dist`. There are no package dependencies to install or lock. Preview with `python3 -m http.server 4175 --directory apps/website/dist`.

```bash
node --test apps/website/release.test.mjs
node apps/website/build.mjs
```

## Cloudflare Pages 配置 / Cloudflare Pages configuration

在 Cloudflare Pages 连接此 GitHub 仓库，仓库根目录为构建根目录，构建命令 `node apps/website/build.mjs`，输出目录 `apps/website/dist`，Node.js 版本 22，生产分支 `main`。其他分支使用独立预览地址；脚本在预览中强制关闭正式下载，且不写源码或生产元数据。配置自定义域名及 HTTPS 后分别验证中英文路径、切换、资源和链接。项目创建、域名和实际部署尚待完成。

Connect this GitHub repository in Cloudflare Pages. Use the repository root, build command `node apps/website/build.mjs`, output `apps/website/dist`, Node.js 22, and production branch `main`. Other branches use separate preview URLs; the script withholds official downloads in previews and never writes source or production metadata. Configure the domain/HTTPS, then verify both languages, switching, assets and links. Project creation, domain configuration and actual deployment remain open.

参考 [Cloudflare Pages GitHub 集成](https://developers.cloudflare.com/pages/configuration/git-integration/github-integration/)和[预览部署](https://developers.cloudflare.com/pages/configuration/preview-deployments/)。

See [Cloudflare Pages GitHub integration](https://developers.cloudflare.com/pages/configuration/git-integration/github-integration/) and [preview deployments](https://developers.cloudflare.com/pages/configuration/preview-deployments/).

## 发布元数据 / Release metadata

仅 `CF_PAGES_BRANCH=main` 或显式 `node apps/website/build.mjs --sync-release` 在构建时读取公开 GitHub latest release，浏览器不接收私有 token。无 release（404）显示待发布；网络／API 或元数据错误使构建失败，避免发布错误下载。发布后重建站点以同步；可在 Pages 手动重建或通过其独立部署 Hook 接入发布流程，不在 PR 中调用生产 Hook。

Only `CF_PAGES_BRANCH=main` or explicit `node apps/website/build.mjs --sync-release` reads the public GitHub latest release at build time; browsers receive no private token. A missing release (404) shows pending status; network/API or metadata errors fail the build to avoid publishing incorrect downloads. Rebuild after a release to synchronize, manually in Pages or through a separate deployment hook in the release flow. Never call the production hook from a PR.

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
