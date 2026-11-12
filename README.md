# DeepSider for macOS

把 [DeepSider](https://web.deepsider.app) 网页版封装成 macOS 原生应用的桌面外壳，使用
**Rust + Tauri 2** 编写。整个应用只有**一个原生窗口 + 一个 `WKWebView`**，外面补上网页版缺少的
桌面能力：原生工具栏、中文菜单栏与快捷键、下载、应用内登录弹窗、以及与系统一致的深浅色主题。

> 非官方客户端，只提供外壳，不包含站点任何代码。

## 功能特性

- **原生工具栏**：真正的 AppKit `NSToolbar`，左侧 6 个图标按钮——后退 / 前进 / 重新载入 /
  强制重新载入 / 停止载入 / 主页。没有地址栏，也没有网页里那种模拟工具栏。
- **中文菜单栏**：DeepSider / 编辑 / 显示 / 导览 / 窗口 / 帮助，快捷键与 Safari 一致。
- **跟随系统深浅色**：工具栏、窗口与页面正文同步切换，见 [主题桥接](#主题桥接)。
- **回车键行为修正**：网页原本 <kbd>Enter</kbd> 就发送。现在裸 <kbd>Enter</kbd> 换行，
  <kbd>Shift</kbd>+<kbd>Enter</kbd> 交给站点，<kbd>⌘</kbd>/<kbd>⌃</kbd>+<kbd>Enter</kbd> 才发送。
- **应用内 OAuth**：`window.open` 弹窗（Google 登录等）由应用自己承载，并共享同一份
  WKWebView 配置，`window.opener` 得以保留，登录态能正常回传到主窗口。
- **外链与下载**：站外链接交给默认浏览器打开；下载落到 `~/Downloads`，重名自动加序号。
- **窗口记忆**：窗口位置、尺寸与页面缩放持久化；显示器拔掉后越界的窗口会被拉回可见区域。
- **其他小功能**：页面内查找栏（<kbd>⌘F</kbd>）、`alert()` 转成页面内 toast、摄像头/麦克风/
  通知权限自动允许、清除网站数据菜单项。

## 系统要求

| | |
|---|---|
| 系统 | macOS 13.0 或更新（对应 `LSMinimumSystemVersion`） |
| 芯片 | 当前发布产物为 **arm64**（Apple Silicon）；Intel 需自行加 target 编译，见[已知限制](#已知限制) |
| 编译 | Rust 1.77+、`@tauri-apps/cli` 2.x（仅打包时需要） |

## 安装

```bash
# 从发行产物安装（自己构建出来的 dmg）
open dist/DeepSider-1.3.2.dmg
# 把 DeepSider.app 拖进 /Applications
```

应用是 ad-hoc 签名，首次打开若被 Gatekeeper 拦下：

```bash
xattr -dr com.apple.quarantine /Applications/DeepSider.app
```

## 构建

```bash
cd src-tauri
cargo build              # 调试版：src-tauri/target/debug/deepsider
cargo build --release    # 发布版
cargo test               # 单元测试（注入脚本断言 + 内网域名匹配）
```

打包 `.dmg` 与 `.zip` 到 `dist/`：

```bash
npm install --no-save @tauri-apps/cli@2
./scripts/package.sh
```

`scripts/package.sh` 做三件事：`tauri build --bundles app` 产出 `.app` → 复制到临时目录并
加 `/Applications` 软链 → `diskutil image create from` 生成 UDZO 格式 dmg，最后 `ditto`
打 zip。这里刻意不用 `hdiutil`——在某些受限 shell 里它会报「操作不被允许」。

> 注意：`tauri build` 自带的 dmg 步骤在部分机器上会因 `hdiutil: couldn't unmount` 失败，
> 所以打包脚本改用 `--bundles app` 自己组盘。

## 项目结构

```
src-tauri/
  Cargo.toml           包名 deepsider / lib 名 deepsider_lib，version 1.3.2
  tauri.conf.json      identifier app.deepsider.mac，窗口在 Rust 侧创建（windows: []）
  src/
    lib.rs             窗口与 webview 装配、菜单、主题同步、下载、状态持久化
    inject.rs          启动注入脚本：回车修正 / 查找栏 / alert toast / 主题桥接
    native.rs          objc2 + msg_send! 对 WKWebView / NSWindow 裸指针的小封装
    toolbar.rs         NSToolbar 及其 delegate（objc2 define_class!）
    main.rs            可执行入口
ui/index.html          占位页，仅因 Tauri 要求 frontendDist 指向的目录存在
scripts/package.sh     打包脚本
```

`lib.rs` 用 Tauri 的 `unstable` feature（`WindowBuilder::build()` + `Window::add_child()`）
先建一个不带 webview 的窗口，再往里挂一个子 webview。

## 菜单与快捷键

| 菜单 | 项目 | 快捷键 |
|---|---|---|
| 显示 | 重新载入 | ⌘R |
| 显示 | 强制重新载入 | ⇧⌘R |
| 显示 | 停止载入 | ⌘. |
| 显示 | 实际大小 / 放大 / 缩小 | ⌘0 / ⌘= / ⌘- |
| 显示 | 进入全屏幕 | ⌃⌘F |
| 导览 | 后退 / 前进 | ⌘[ / ⌘] |
| 导览 | 主页 | ⇧⌘H |
| 导览 | 查找… | ⌘F |
| 导览 | 在默认浏览器中打开 | ⇧⌘O |
| 导览 | 清除网站数据 | — |
| 编辑 | 撤销 / 重做 / 剪切 / 拷贝 / 粘贴 / 全选 | 系统标准 |
| 窗口 | 最小化 / 缩放 / 前置全部窗口 | 系统标准 |

工具栏按钮的可用状态（后退、前进、停止载入）由 350ms 轮询从 WKWebView 读取后回写，
通过 `validateToolbarItem:` 刷新。

## 主题桥接

这是本项目最不直观的一块。

DeepSider **不使用** `prefers-color-scheme`——这个字符串在它的样式表里出现 0 次。它靠给
`<html>` 加 `dark` 类切换主题，而且浅色调色板被写死在一段内联 `<style>` 里，深色调色板
则是由站点自己的 JS 在运行时注入。

因此只加 `dark` 类会得到**白底白字**：`html.app.dark` 把文字变成 `#fff`，但背景变量
`--cg-color-b1` 等仍然解析成浅色值。注入脚本补上了这一段：

1. `lib.rs` 读取当前有效外观（`Window::theme()`，失败时退回 `native::is_dark_appearance`
   读 `NSAppearance`），在每次导航完成、以及收到 `ThemeChanged` 事件时调用
   `window.__dsSetTheme(dark)`；工具栏轮询器另外做一次兜底，因为窗口隐藏时可能收不到事件。
2. `inject.rs` 切换 `dark` 类、同步 `color-scheme`，并注入 `<style id="__dsThemeVars">`，
   把站点自己的深色调色板写进去，选择器用 `html.dark .deepsider-root` 以压过站点那条
   内联的浅色变量（多一级特异性）。
3. 一个 `MutationObserver` 监听 `<html>` 的 `class`，站点在 SPA 路由切换时把 `dark` 抹掉
   就会立刻补回；脚本在文档开始时先按 `matchMedia` 应用一次，避免冷启动白闪。

## 运行数据

| 内容 | 位置 |
|---|---|
| 窗口尺寸位置、页面缩放 | `~/Library/Application Support/app.deepsider.mac/state.json` |
| 登录态、Cookie、IndexedDB | 应用容器内的 WebKit 数据目录（由 WKWebsiteDataStore 管理） |
| 下载文件 | `~/Downloads` |

登录 token 存在站点的 IndexedDB（`chatstore`）里，而不是 `document.cookie`，所以清除数据时
会一并丢失登录态。

## 已知限制

- **仅 arm64**：本机只装了 `aarch64-apple-darwin`。要做 universal 需要
  `rustup target add x86_64-apple-darwin`，再
  `tauri build --target universal-apple-darwin`。
- **ad-hoc 签名**：`codesign -dv` 显示 `Signature=adhoc`。对外分发需要 Developer ID
  证书 + 公证（notarisation）。
- **主题是应用级的**：macOS 上 Tauri 通过 `NSApplication.appearance` 生效，无法做到
  单个窗口单独指定深浅色。
- **原生工具栏依赖 `unstable` feature**：升级 Tauri 大版本时 `add_child` /
  `WebviewBuilder` 的导出路径可能变动。
- **`ui/index.html` 是占位文件**：早期 1.3.0 用 HTML 工具栏，1.3.1 换成原生工具栏后它已经
  不被加载，只是 Tauri 要求 `frontendDist` 指向的目录必须存在。

## 版本历史

| 版本 | 说明 |
|---|---|
| 1.0.0 | Swift + WKWebView 首版，原生窗口、工具栏、中文菜单 |
| 1.1.0 | 回车键行为修正（Enter 只换行，⌘Enter 发送） |
| 1.2.0 | 工具栏补主页 / 重新载入 / 强制重新载入 / 停止载入 |
| 1.3.0 | 整体重写为 **Tauri 2 + Rust**，主题改为跟随系统 |
| 1.3.1 | 去掉模拟浏览器的 HTML 工具栏，换回原生 `NSToolbar` |
| 1.3.2 | 修好深色模式（深色调色板注入），页面与工具栏同步跟随系统深浅色 |

## 许可

未指定许可证。这是个人项目，仅供学习与自用；DeepSider 服务本身的使用条款以官方为准。
