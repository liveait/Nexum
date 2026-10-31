import { cp, mkdir, rm, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { fetchRelease } from "./release.mjs";
import { buildSettings } from "./site-config.mjs";

const base = new URL("./", import.meta.url);
const output = new URL("dist/", base);
const settings = buildSettings(process.env, process.argv.slice(2));
const sitePath = (path) => `${settings.basePath}${path}`;
const release = settings.syncRelease ? await fetchRelease() : { available: false };

const messages = {
  zh: {
    lang: "zh-CN", other: "en", language: "English", title: "下载，回到简单。", eyebrow: "为 macOS 打造的独立下载器",
    intro: "粘贴链接，选好位置。Nexum 在后台处理下载，让你继续手上的事。",
    download: "查看下载进展", source: "查看源码", features: "功能", downloads: "下载", docs: "文档", changelog: "更新日志",
    caption: "原生界面预览 · 2026 年 10 月 release 候选产物", number: "01 / 下载，清楚可见",
    heading: "从开始，到下一次继续。", detail: "用一个窗口管理 HTTP 与 HTTPS 下载，看到进度，也掌握控制。",
    items: [["进度始终可见", "任务列表、即时速率和可信长度下的预计时间，让下载状态一目了然。"], ["随时暂停，继续下载", "暂停、恢复、限速，以及经来源校验的跨重启续传，保留下载的连续性。"], ["双击启动，后台运行", "App 内置本地下载服务，支持默认下载目录，也保留连接外部 Server 的选择。"]],
    readyTitle: "Nexum，现已就绪。", releaseTitle: "下一站，正式发布。", pending: "macOS 首个正式版本正在验收。", pendingDetail: "签名、公证与安装升级检查完成后，这里将提供经过验证的安装包。",
    releaseDetail: "从 GitHub Releases 下载。请核对下列文件大小与 SHA256。", verified: "Developer ID 签名 · 已公证 · Gatekeeper 已验证",
    minimum: "最低 macOS", downloadLabel: "下载", limits: "当前专注 HTTP / HTTPS。多连接、FTP、BT / 磁力、浏览器配对和其他平台仍在路线图中。",
    project: "了解项目", projectDetail: "源码、设计决策与开发记录公开，能力以实际验收范围为准。", feedback: "反馈问题", plan: "开发计划", releaseNotes: "发布记录", footer: "轻量、清晰，继续前行。",
  },
  en: {
    lang: "en", other: "zh", language: "中文", title: "A little less waiting.", eyebrow: "An independent downloader for macOS",
    intro: "Paste a link. Pick a place. Nexum takes care of the download while you get back to what matters.",
    download: "Follow the release", source: "Explore the source", features: "Features", downloads: "Download", docs: "Docs", changelog: "Changelog",
    caption: "Native interface preview · October 2026 release candidate", number: "01 / A clear view of every download",
    heading: "From start to picking up again.", detail: "One window for HTTP and HTTPS downloads, with progress you can see and controls you can reach.",
    items: [["Know where things stand", "A task list, live transfer speed and estimates when the file length is known keep progress in view."], ["Pause. Resume. Carry on.", "Pause, resume, bandwidth limits and source-validated recovery across restarts keep downloads moving."], ["Open the app and get started", "A bundled local service handles downloads in the background. Choose a default folder or connect to an external server."]],
    readyTitle: "Nexum for your Mac.", releaseTitle: "Next stop: the first release.", pending: "The first macOS release is in validation.", pendingDetail: "Verified installers will appear here after distribution signing, notarization and install/upgrade checks pass.",
    releaseDetail: "Download from GitHub Releases. Check the file size and SHA256 below.", verified: "Developer ID signed · Notarized · Gatekeeper verified",
    minimum: "Minimum macOS", downloadLabel: "Download", limits: "Focused on HTTP / HTTPS today. Multi-connection downloads, FTP, BT / magnet, browser pairing and other platforms remain on the roadmap.",
    project: "An open view of the project.", projectDetail: "Source, design decisions and development notes are public. Supported capabilities follow actual acceptance results.", feedback: "Report an issue", plan: "Development plan", releaseNotes: "Release history", footer: "Less clutter. More continuity.",
  },
};

const escape = (value) => String(value).replace(/[&<>"']/g, (char) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[char]);
function page(locale) {
  const t = messages[locale];
  const cards = t.items.map(([title, description], i) => `<article><span class="step">0${i + 1}</span><h3>${title}</h3><p>${description}</p></article>`).join("");
  const packages = release.available ? `<p>${t.releaseDetail}</p><div class="packages">${release.artifacts.map((asset) => `<article><h3>macOS · ${asset.arch === "aarch64" ? "Apple silicon" : "Intel"}</h3><p>${t.minimum} ${escape(asset.minimum_os)} · ${(asset.size / 1048576).toFixed(1)} MiB</p><a class="button" href="${escape(asset.url)}">${t.downloadLabel} ${escape(release.version)} ↗</a><p class="verified">${t.verified}</p><details><summary>SHA256</summary><code>${escape(asset.sha256)}</code></details></article>`).join("")}</div>` : `<p class="pending"><span class="dot"></span>${t.pending}</p><p>${t.pendingDetail}</p>`;
  return `<!doctype html><html lang="${t.lang}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src 'self'; style-src 'self'; base-uri 'none'; form-action 'none'"><title>Nexum — ${t.title}</title><meta name="description" content="${t.intro}"><link rel="alternate" hreflang="zh-CN" href="${sitePath("zh/")}"><link rel="alternate" hreflang="en" href="${sitePath("en/")}"><link rel="icon" type="image/png" href="${sitePath("assets/nexum.png")}"><link rel="apple-touch-icon" sizes="180x180" href="${sitePath("assets/apple-touch-icon.png")}"><link rel="stylesheet" href="${sitePath("assets/style.css")}"></head>
<body><a class="skip" href="#main">${t.features}</a><header><a class="brand" href="${sitePath(`${locale}/`)}" aria-label="Nexum"><img class="mark" src="${sitePath("assets/nexum.png")}" alt="" width="32" height="32">Nexum</a><nav aria-label="${t.project}"><a href="#features">${t.features}</a><a href="#download">${t.downloads}</a><a href="https://github.com/liveait/Nexum/blob/main/docs/DEVELOPMENT.md">${t.docs}</a><a href="${sitePath(`${t.other}/`)}" lang="${t.other === "zh" ? "zh-CN" : "en"}">${t.language}</a></nav></header>
<main id="main"><section class="hero"><div class="hero-copy"><p class="eyebrow"><span class="dot"></span>${t.eyebrow}</p><h1>${t.title}</h1><p class="intro">${t.intro}</p><div class="actions"><a class="button" href="#download">${t.download} <span aria-hidden="true">↓</span></a><a class="text-link" href="https://github.com/liveait/Nexum">${t.source} ↗</a></div><span class="hero-note">HTTP / HTTPS · macOS · Open source</span></div><figure><img src="${sitePath("assets/downloads-native.png")}" alt="Nexum ${locale === "zh" ? "原生下载界面" : "native download interface"}" width="2400" height="1602"><figcaption>${t.caption}</figcaption></figure></section>
<section class="features" id="features"><p class="eyebrow">${t.number}</p><div class="section-heading"><h2>${t.heading}</h2><p>${t.detail}</p></div><div class="feature-grid">${cards}</div></section>
<section class="release" id="download"><p class="eyebrow">02 / NEXUM FOR MAC</p><h2>${release.available ? t.readyTitle : t.releaseTitle}</h2>${packages}<p class="limits">${t.limits}</p></section>
<section class="project"><div><p class="eyebrow">03 / BUILT IN THE OPEN</p><h2>${t.project}</h2><p>${t.projectDetail}</p></div><div class="project-links"><a href="https://github.com/liveait/Nexum/blob/main/docs/DEVELOPMENT_PLAN.md">${t.plan}<span>↗</span></a><a href="https://github.com/liveait/Nexum/releases">${t.releaseNotes}<span>↗</span></a><a href="https://github.com/liveait/Nexum/blob/main/CHANGELOG.md">${t.changelog}<span>↗</span></a><a href="https://github.com/liveait/Nexum/issues">${t.feedback}<span>↗</span></a></div></section></main><footer><span class="brand"><img class="mark" src="${sitePath("assets/nexum.png")}" alt="" width="32" height="32">Nexum</span><p>${t.footer}</p><a href="https://github.com/liveait/Nexum">GitHub ↗</a></footer></body></html>`;
}

await rm(output, { recursive: true, force: true });
await mkdir(new URL("assets/", output), { recursive: true });
for (const locale of Object.keys(messages)) {
  await mkdir(new URL(`${locale}/`, output), { recursive: true });
  await writeFile(new URL(`${locale}/index.html`, output), page(locale));
}
await writeFile(new URL("index.html", output), `<!doctype html><html lang="zh-CN"><meta charset="utf-8"><meta http-equiv="refresh" content="0;url=${sitePath("zh/")}"><title>Nexum</title><a href="${sitePath("zh/")}">中文</a> · <a href="${sitePath("en/")}">English</a></html>`);
await cp(new URL("../desktop/src-tauri/icons/icon.png", base), new URL("assets/nexum.png", output));
await cp(new URL("../desktop/src-tauri/icons/apple-touch-icon.png", base), new URL("assets/apple-touch-icon.png", output));
await writeFile(new URL(".nojekyll", output), "");
await writeFile(new URL("release.json", output), `${JSON.stringify(release, null, 2)}\n`);
await cp(new URL("style.css", base), new URL("assets/style.css", output));
await cp(new URL("assets/downloads-native.png", base), new URL("assets/downloads-native.png", output));
console.log(`Built ${fileURLToPath(output)} (${settings.basePath}; ${release.available ? "accepted release" : "release pending"})`);
