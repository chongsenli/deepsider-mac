# DeepSider for macOS

A native macOS wrapper around the [DeepSider](https://web.deepsider.app) web app, written in
**Rust + Tauri 2**. It hosts the site in a single `WKWebView` inside a real `NSWindow` and adds
the desktop furniture the web page lacks: a native toolbar, a Chinese menu bar with the usual
shortcuts, downloads, in-app OAuth, and a theme bridge.

<p align="center">
  <img src="docs/screenshot-dark.png" width="49%" alt="深色模式">
  <img src="docs/screenshot-light.png" width="49%" alt="浅色模式">
</p>

## Features

| | |
|---|---|
| **Native chrome** | Real `NSToolbar` with back / forward / reload / hard reload / stop / home. No browser-style address bar. |
| **Chinese menu bar** | 编辑 / 显示 / 导览 / 窗口 / 帮助 with the standard accelerators (⌘R, ⇧⌘R, ⌘., ⌘0, ⌘=, ⌘-, ⌘F, ⌘[, ⌘], ⇧⌘H, ⇧⌘O …). |
| **Dark & light** | Follows the system appearance, including the site itself — see [Theme bridge](#theme-bridge). |
| **Enter behaviour** | Bare <kbd>Enter</kbd> inserts a newline, <kbd>Shift</kbd>+<kbd>Enter</kbd> is left to the site, <kbd>⌘</kbd>/<kbd>⌃</kbd>+<kbd>Enter</kbd> sends. |
| **In-app OAuth** | `window.open` popups (Google sign-in) are hosted by the app with a shared web view configuration, so `window.opener` survives and the session flows back. |
| **Links & downloads** | External links open in the default browser; downloads land in `~/Downloads` with collision-safe names. |
| **Window memory** | Frame and page zoom are persisted to the app config dir; off-screen frames are pulled back onto a monitor. |
| **Small extras** | In-page find bar (<kbd>⌘F</kbd>), `alert()` surfaced as a toast, camera/microphone permission prompts granted, Chinese offline error page, clear-website-data menu item. |

## Requirements

- macOS 13 or newer
- Rust toolchain (1.77+), and optionally `@tauri-apps/cli` for bundling

## Build

```bash
cd src-tauri
cargo build --release          # debug binary: cargo build
```

Package a `.dmg` and `.zip` into `dist/`:

```bash
npm install --no-save @tauri-apps/cli@2
./scripts/package.sh
```

`scripts/package.sh` runs `tauri build --bundles app`, stages the `.app` with an
`/Applications` symlink, and creates the disk image with `diskutil image create from`
(`hdiutil` is unreliable inside sandboxed shells).

## Layout

```
src-tauri/src/
  lib.rs        window + webview wiring, menu, theme sync, downloads, persistence
  inject.rs     initialization script: Enter fix, find bar, alert toast, theme bridge
  native.rs     small objc2/msg_send helpers over the raw WKWebView / NSWindow pointers
  toolbar.rs    NSToolbar and its delegate, built with objc2's define_class!
  main.rs       binary entry point
ui/index.html   placeholder index required by Tauri's frontendDist (unused at runtime)
scripts/        packaging helpers
```

## Theme bridge

DeepSider does **not** use `prefers-color-scheme` — the string appears zero times in its
stylesheets. It switches themes by putting a `dark` class on `<html>`, and the light palette is
hard-coded in an inline `<style>` element, while the dark palette is applied at runtime by the
app's own JavaScript.

Adding only the `dark` class therefore produces white text on a white background. The injected
script closes the gap:

1. `lib.rs` reads the effective appearance (`Window::theme()`, falling back to
   `NSAppearance` via `native::is_dark_appearance`) and calls
   `window.__dsSetTheme(dark)` after every navigation and on `ThemeChanged`.
2. `inject.rs` toggles the `dark` class, keeps `color-scheme` in sync, and injects a
   `<style id="__dsThemeVars">` block carrying the site's own dark palette — scoped to
   `html.dark .deepsider-root` so it outranks the hard-coded light variables.
3. A `MutationObserver` re-applies the class if an in-app route change drops it, and the
   script applies the theme once at document start to avoid a white flash.

## Notes

- The build is **arm64** only by default; add `aarch64-apple-darwin` / `x86_64-apple-darwin`
  targets and `--target universal-apple-darwin` for a universal binary.
- The bundle is ad-hoc signed. Distributing it outside your own machine needs a Developer ID
  certificate and notarisation.
- The app is an unofficial client for a third-party service; it ships no site code, only the
  shell around it.
