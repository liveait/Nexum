# 品牌图标与生成 / Brand icons and generation

## 已选原稿 / Selected source artwork

2026-10-31 选定的石墨底 N 图标保存在 [`assets/brand/icon-graphite-v1.png`](../assets/brand/icon-graphite-v1.png)。它是原始 **1254 × 1254 RGBA PNG** 的逐字节副本；保留透明区域和已选设计，不裁切、重画或改色。旧图稿和旧派生资源仍可从 Git 历史追溯。

The graphite-backed N selected on 2026-10-31 is stored at [`assets/brand/icon-graphite-v1.png`](../assets/brand/icon-graphite-v1.png). It is a byte-for-byte copy of the original **1254 × 1254 RGBA PNG**, preserving transparency and the selected design without cropping, repainting or recoloring. Earlier artwork and derivatives remain in Git history.

同日选定的透明 N 标记原图保存在 [`assets/brand/nexum-mark.png`](../assets/brand/nexum-mark.png)，也是 **1254 × 1254 RGBA PNG** 的逐字节副本。它供侧栏与网站页头使用；Dock／Finder、favicon、Apple touch 和扩展继续使用完整石墨底 App 图标。生成脚本只缩放这两份已选原稿，不再修改设计。

The transparent N mark selected the same day is stored at [`assets/brand/nexum-mark.png`](../assets/brand/nexum-mark.png), also a byte-for-byte copy of a **1254 × 1254 RGBA PNG**. It serves the sidebar and website header; Dock/Finder, favicon, Apple touch and the extension retain the complete graphite-backed App icon. The generation script only resizes these two selected sources without further design edits.

原稿 SHA-256 / Source SHA-256:

```text
6dd6608d8b9bd67fbca249470e235dcee1b495344e69f8828508f12300f07e4f
```

透明标记原稿 SHA-256 / Transparent mark source SHA-256:

```text
9678c621f4a7a232db7875ccf5be87be0c48413e40e5ad6d9591582455e95c1e
```

## 生成命令与依赖 / Generation command and dependencies

在仓库根目录执行。Node.js 和 pnpm 要求见[开发指南](DEVELOPMENT.md)；Desktop 锁定 pnpm 11.25.0 与 Tauri CLI 2.11.5。脚本只使用 Node 内置模块和已安装的官方 Tauri CLI，不调用图像生成服务，不在生成过程中安装依赖。

Run from the repository root. See the [Development Guide](DEVELOPMENT.md) for Node.js and pnpm requirements; Desktop pins pnpm 11.25.0 and Tauri CLI 2.11.5. The script uses only Node built-ins and the installed official Tauri CLI, without an image-generation service or dependency installation during generation.

```bash
pnpm --dir apps/desktop install --frozen-lockfile
node scripts/generate-brand-icons.mjs
node scripts/verify-icon-container.mjs
```

脚本调用安装包的 `tauri.js` 入口执行下面的标准命令；`TEMP` 表示脚本私有临时目录。所有缩放都直接从各自的 1254px 原稿进行，不从 128px 派生图放大。默认命令产生的移动平台和商店专用资源不进入仓库。

The script invokes the installed `tauri.js` entry for the standard commands below; `TEMP` represents its private temporary directory. Every resize starts directly from its respective 1254px source, never an upscaled 128px derivative. Mobile/store-specific outputs from the default command are not added to the repository.

```bash
node apps/desktop/node_modules/@tauri-apps/cli/tauri.js icon assets/brand/icon-graphite-v1.png --output TEMP/standard
node apps/desktop/node_modules/@tauri-apps/cli/tauri.js icon assets/brand/icon-graphite-v1.png --output TEMP/png --png 16 --png 32 --png 48 --png 128 --png 180 --png 1024
node apps/desktop/node_modules/@tauri-apps/cli/tauri.js icon assets/brand/nexum-mark.png --output TEMP/mark-png --png 128
```

生成后更新 [`assets/brand/icon-manifest.json`](../assets/brand/icon-manifest.json)，记录原稿尺寸／哈希、CLI 版本、容器尺寸和每个交付资源的 SHA-256。同版本 CLI 在本机连续运行两次后，清单与全部交付资源逐字节一致。

Generation updates [`assets/brand/icon-manifest.json`](../assets/brand/icon-manifest.json) with source dimensions/hash, CLI version, container sizes and each delivered asset's SHA-256. Two consecutive local runs with the same CLI produce byte-identical manifests and delivered assets.

## 资源与封装取舍 / Assets and container choices

| 文件 / File | 尺寸与用途 / Dimensions and use |
| --- | --- |
| `apps/desktop/src-tauri/icons/icon.png` | 128px 规范图；与 ICO 的 128px PNG payload 及 ICNS 的 `ic07` payload 精确相同。 / 128px canonical image, exactly matching the ICO 128px PNG payload and ICNS `ic07` payload. |
| `apps/desktop/src-tauri/icons/icon-1024.png` | 从原稿直接下采样的 1024px 导出，便于高分辨率消费；它不是被放大的旧 128px 图。 / 1024px export downsampled directly from the source for high-resolution consumers, not an enlarged old 128px image. |
| `apps/desktop/src-tauri/icons/apple-touch-icon.png` | 从原稿直接生成的 180px Apple touch 资源；网站接线由网站改动负责。 / 180px Apple touch image generated directly from the source; website wiring belongs to the website change. |
| `apps/desktop/src-tauri/icons/brand-mark.png` | 从透明标记原稿直接生成的 128px RGBA；侧栏／网站页头接线由各自 UI 改动负责，不替换规范 App 图标。 / 128px RGBA generated directly from the transparent mark source; sidebar/website header wiring belongs to their UI changes and does not replace the canonical App icon. |
| `apps/desktop/src-tauri/icons/icon.icns` | 标准 Tauri ICNS，含 16／32／64／128／256／512／1024px 及 Retina 表示。 / Standard Tauri ICNS with 16/32/64/128/256/512/1024px and Retina representations. |
| `apps/desktop/src-tauri/icons/icon.ico` | 标准 Tauri 的 16／24／32／48／64／256px PNG 帧，加同源 128px 帧。 / Standard Tauri 16/24/32/48/64/256px PNG frames plus the same-source 128px frame. |
| `apps/extension/icons/nexum-{16,32,48,128}.png` | 同一原稿直接生成的扩展尺寸；128px 与 Desktop 规范图字节相同。 / Extension sizes generated directly from the same source; 128px is byte-identical to the Desktop canonical image. |

Tauri 2.11.5 的默认 ICO 不包含 128px。脚本保留 CLI 已生成的全部 PNG payload，只新增 128px 目录项并更新偏移，维持既有 canonical 绑定保护。默认 ICNS 的记录顺序在多次运行间会变化；脚本按类型排序完整记录，保留每个记录的原字节，避免哈希只因目录顺序改变。这些步骤仅处理容器，不再次缩放或编码像素。

Tauri 2.11.5's default ICO omits 128px. The script retains every CLI-generated PNG payload, adds a 128px entry and updates offsets, preserving the existing canonical-binding protection. Default ICNS record order varies between runs; the script sorts complete records by type while preserving every record's bytes, so ordering alone does not change hashes. These steps only package containers, without another pixel resize or encoding.

## 验证与边界 / Verification and limits

既有 `verify-desktop-bundle.mjs` 继续验证 Server 哈希、版本、签名和架构，并验证图标源／包一致性。ICO 检查实际解析每个资源目录、PNG CRC、zlib 与行过滤器，要求 128px payload 与规范图一致；ICNS 检查所有记录边界，解码 PNG 与 legacy RGB/RLE/alpha，要求完整尺寸覆盖至 1024px 和规范图绑定。它们不是只读文件名或尺寸常量。

The existing `verify-desktop-bundle.mjs` retains Server hash, version, signature and architecture checks, and verifies icon source/package consistency. ICO verification parses every resource entry, PNG CRC, zlib and row filters, requiring its 128px payload to match the canonical image. ICNS verification checks every record boundary, decodes PNG and legacy RGB/RLE/alpha data, and requires full size coverage through 1024px plus canonical binding. These checks do more than read filenames or dimension constants.

本轮还用 macOS ImageIO 真正解码 ICNS 的 10 个表示和 ICO 的 7 帧，并运行扩展已有 lint／build。扩展目前没有锁文件，验证安装未生成或修改锁文件；pnpm 对已知 `esbuild` 的一次性脚本批准仅用于本机验证，不作为产品配置提交。

This slice also uses macOS ImageIO to decode all 10 ICNS representations and 7 ICO frames, and runs the extension's existing lint/build. The extension currently has no lockfile; validation installation creates or changes none. A one-time pnpm script approval for the known `esbuild` dependency is used only for local validation and is not committed as product configuration.

格式解码不等于 Windows 原生图标／安装包验收；ad-hoc 签名不等于分发签名、公证或 Gatekeeper 验收。Finder／Dock、扩展加载、网站及最终原生产物的视觉检查仍应按各自候选记录，不能因资源生成成功而标记 M0/M1/M3 完成。

Format decoding does not establish Windows native icon/installer acceptance, and ad-hoc signing does not establish distribution signing, notarization or Gatekeeper acceptance. Record Finder/Dock, extension loading, website and final native-artifact visual checks against their respective candidates; successful generation alone does not complete M0/M1/M3.
