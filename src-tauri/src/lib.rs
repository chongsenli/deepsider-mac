//! DeepSider for macOS — Tauri 2 / Rust shell (1.3.2).
//!
//! Layout: one native window ("main") hosting a single webview ("remote") that
//! shows the DeepSider SPA. Navigation chrome is a real AppKit `NSToolbar`
//! (see `toolbar.rs`), mirroring the Swift 1.2.0 build: six icon buttons at the
//! leading edge, no address bar, no title label.
//!
//! Everything WKWebView owns but Tauri does not expose (back/forward/stop/
//! progress) goes through `native.rs` + `msg_send!`.

mod inject;
mod native;
mod toolbar;

use std::ffi::c_void;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::window::WindowBuilder;
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, Position, Rect, Size, Theme, Url,
    WebviewUrl, Window, WindowEvent, Wry,
};

/// The SPA entry point (same URL the Swift 1.2.0 build used).
const HOME: &str = "https://web.deepsider.app/chat/?extension_id=dfbnddndcmilnhdfmmaolepiaefacnpo";
const DEFAULT_ZOOM: f64 = 1.0;
const MIN_ZOOM: f64 = 0.5;
const MAX_ZOOM: f64 = 2.0;
const DEFAULT_W: f64 = 1200.0;
const DEFAULT_H: f64 = 820.0;

/// Safari-like UA. The SPA is an extension page and only enables its full UI
/// for browsers it recognises, so we keep the exact shape the Swift build used.
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";

/// Hosts that must stay inside the app (login / OAuth must not escape to the
/// system browser, otherwise the session cookie never comes back).
const INTERNAL_HOSTS: &[&str] = &[
    "deepsider.app",
    "deepsider.net",
    "deepsider.me",
    "deepsider.ai",
    "deepsider.online",
    "chatgot.io",
    "chatgot.tech",
    "google.com",
    "googleusercontent.com",
    "gstatic.com",
    "googleapis.com",
    "apple.com",
    "microsoftonline.com",
    "live.com",
    "github.com",
];

fn is_internal(host: &str) -> bool {
    if host == "localhost" || host == "127.0.0.1" {
        return true;
    }
    INTERNAL_HOSTS
        .iter()
        .any(|h| host == *h || host.ends_with(&format!(".{h}")))
}

/// Global handle so AppKit callbacks (which carry no Tauri context) can drive
/// navigation without threading a handle through every objc selector.
static APP: OnceLock<AppHandle<Wry>> = OnceLock::new();

/* ------------------------------------------------------------------ */
/* persisted state                                                     */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct SavedFrame {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    window: Option<SavedFrame>,
    #[serde(default = "default_zoom")]
    zoom: f64,
}

// NOTE: never derive Default here — `f64::default()` is 0.0, which clamps to
// MIN_ZOOM and makes the first launch open at 50%.
impl Default for Saved {
    fn default() -> Self {
        Self {
            window: None,
            zoom: DEFAULT_ZOOM,
        }
    }
}

fn default_zoom() -> f64 {
    DEFAULT_ZOOM
}

struct Shared {
    notice: Mutex<String>,
    zoom: Mutex<f64>,
    title: Mutex<String>,
    frame: Mutex<Option<SavedFrame>>,
    file: PathBuf,
}

impl Shared {
    fn save(&self) {
        let frame = *self.frame.lock().unwrap();
        let zoom = *self.zoom.lock().unwrap();
        let data = Saved { window: frame, zoom };
        if let Ok(json) = serde_json::to_vec_pretty(&data) {
            if let Some(dir) = self.file.parent() {
                let _ = fs::create_dir_all(dir);
            }
            let _ = fs::write(&self.file, json);
        }
    }

    fn set_notice(&self, text: impl Into<String>) {
        if let Ok(mut n) = self.notice.lock() {
            *n = text.into();
        }
    }
}

fn shared(app: &AppHandle) -> tauri::State<'_, Shared> {
    app.state::<Shared>()
}

/* ------------------------------------------------------------------ */
/* WKWebView bridge                                                    */
/* ------------------------------------------------------------------ */

/// Run `f` against the raw `WKWebView` pointer and wait for its result.
///
/// `with_webview` executes inline when we are on the main thread and posts to
/// the event loop otherwise; in both cases the channel receives exactly once,
/// so a bounded `recv_timeout` cannot deadlock the UI.
fn with_wk<R: Send + 'static, F>(webview: &tauri::Webview, f: F) -> Option<R>
where
    F: FnOnce(*mut c_void) -> R + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    webview
        .with_webview(move |pw| {
            let _ = tx.send(f(pw.inner()));
        })
        .ok()?;
    rx.recv_timeout(Duration::from_millis(1500)).ok()
}

fn remote(app: &AppHandle) -> Option<tauri::Webview> {
    app.get_webview("remote")
}

/// Returns the appearance the app should be painted in right now.
///
/// `WindowBuilder::theme(None)` makes the window follow the system, so the
/// window's reported `theme()` is already the effective appearance; the WKWebView
/// probe is the fallback when the callback comes back with an error.
fn effective_dark(app: &AppHandle) -> bool {
    if let Some(win) = app.get_window("main") {
        if let Ok(theme) = win.theme() {
            return theme == Theme::Dark;
        }
    }
    if let Some(wv) = remote(app) {
        if let Some(dark) = with_wk(&wv, |wk| native::is_dark_appearance(wk)) {
            return dark;
        }
    }
    false
}

/// Mirrors the system appearance onto the remote page.
///
/// The DeepSider stylesheet keys its dark theme off a `dark` class on `<html>`
/// and ignores `prefers-color-scheme` entirely, so a plain WKWebView would stay
/// light forever. `inject.rs` exposes `window.__dsSetTheme(bool)` for exactly
/// this; we push it on launch, after every finished page load, and whenever the
/// system appearance changes.
fn sync_theme(app: &AppHandle) {
    push_theme(app, effective_dark(app));
}

fn push_theme(app: &AppHandle, dark: bool) {
    if let Some(wv) = remote(app) {
        let js = format!(
            "try{{window.__dsSetTheme&&window.__dsSetTheme({})}}catch(e){{}}",
            if dark { "true" } else { "false" }
        );
        let _ = wv.eval(&js);
    }
}

fn frame_of(app: &AppHandle) -> Option<SavedFrame> {
    let win = app.get_window("main")?;
    let pos = win.outer_position().ok()?;
    let size = win.inner_size().ok()?;
    let scale = win.scale_factor().unwrap_or(1.0);
    Some(SavedFrame {
        x: pos.x as f64 / scale,
        y: pos.y as f64 / scale,
        w: size.width as f64 / scale,
        h: size.height as f64 / scale,
    })
}

/* ------------------------------------------------------------------ */
/* layout                                                              */
/* ------------------------------------------------------------------ */

/// Height of title bar + toolbar that overlays the content view, in points.
///
/// tao installs a full-size content view, so `Window::inner_size()` reports the
/// whole content view — including the strip the title bar and toolbar cover.
/// Child webviews are positioned from the content view's top edge, so a y of 0
/// would hide them underneath.
fn chrome_inset(win: &Window) -> f64 {
    let Some(wv) = win.get_webview("remote") else {
        return 0.0;
    };
    match with_wk(&wv, |wk| native::content_metrics(wk)) {
        Some(Some((full, visible))) if full > visible && visible > 1.0 => {
            let inset = full - visible;
            if inset.is_finite() && inset >= 0.0 && inset < 200.0 {
                inset
            } else {
                0.0
            }
        }
        _ => 0.0,
    }
}

/// Keep the single webview clear of the title bar / toolbar.
fn layout_children(win: &Window) {
    let Ok(size) = win.inner_size() else { return };
    let scale = win.scale_factor().unwrap_or(1.0);
    if scale <= 0.0 {
        return;
    }
    let w = size.width as f64 / scale;
    let h = size.height as f64 / scale;
    if w < 1.0 || h < 1.0 {
        return;
    }
    let inset = chrome_inset(win);
    let top = inset.min((h - 40.0).max(0.0));
    let body = (h - top).max(1.0);

    if let Some(webview) = win.get_webview("remote") {
        let _ = webview.set_bounds(Rect {
            position: Position::Logical(LogicalPosition::new(0.0, top)),
            size: Size::Logical(LogicalSize::new(w, body)),
        });
    }
}

/* ------------------------------------------------------------------ */
/* navigation                                                          */
/* ------------------------------------------------------------------ */

fn apply_zoom(app: &AppHandle, zoom: f64) {
    let z = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    if let Some(wv) = remote(app) {
        let _ = wv.set_zoom(z);
    }
    if let Ok(mut cur) = shared(app).zoom.lock() {
        *cur = z;
    }
    shared(app).save();
}

fn goto_home(app: &AppHandle) {
    if let Some(wv) = remote(app) {
        if let Ok(url) = Url::parse(HOME) {
            let _ = wv.navigate(url);
        }
    }
}

fn open_in_browser(app: &AppHandle) {
    let target = remote(app)
        .and_then(|wv| wv.url().ok())
        .map(|u| u.to_string())
        .unwrap_or_else(|| HOME.to_string());
    let _ = std::process::Command::new("open").arg(target).spawn();
}

/// Perform a navigation action on the remote webview.
fn perform_nav(app: &AppHandle, action: &str) {
    let Some(wv) = remote(app) else { return };
    match action {
        "back" => {
            with_wk(&wv, |wk| native::go_back(wk));
        }
        "forward" => {
            with_wk(&wv, |wk| native::go_forward(wk));
        }
        "reload" => {
            let _ = wv.reload();
        }
        "hard_reload" => {
            with_wk(&wv, |wk| native::reload_from_origin(wk));
        }
        "stop" => {
            with_wk(&wv, |wk| native::stop_loading(wk));
        }
        "home" => goto_home(app),
        "open_in_browser" => open_in_browser(app),
        other => eprintln!("deepsider: unknown nav action {other}"),
    }
}

/// Entry point for the native toolbar (called from an objc action selector).
pub(crate) fn dispatch_nav(action: &str) {
    let Some(app) = APP.get() else { return };
    perform_nav(app, action);
}

/// Push the current back/forward/loading state into the toolbar's buttons.
fn refresh_toolbar(app: &AppHandle) {
    let Some(wv) = remote(app) else { return };
    let state = with_wk(&wv, |wk| {
        (
            native::can_go_back(wk),
            native::can_go_forward(wk),
            native::is_loading(wk),
        )
    });
    if let Some((back, forward, loading)) = state {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            let _ = handle;
            toolbar::refresh_state(back, forward, loading);
        });
    }
}

/// Poll the WKWebView because Tauri emits no navigation-state events.
fn spawn_toolbar_poller(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_dark: Option<bool> = None;
        loop {
            if app.get_window("main").is_none() {
                break;
            }
            refresh_toolbar(&app);
            // Fallback for `WindowEvent::ThemeChanged`: that event can be missed
            // (e.g. when the appearance changes while the window is hidden), so
            // the appearance is also sampled here and pushed only on a change.
            let dark = effective_dark(&app);
            if last_dark != Some(dark) {
                last_dark = Some(dark);
                push_theme(&app, dark);
            }
            std::thread::sleep(Duration::from_millis(350));
        }
    });
}

/* ------------------------------------------------------------------ */
/* menu                                                                */
/* ------------------------------------------------------------------ */

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let about = PredefinedMenuItem::about(
        app,
        Some("关于 DeepSider"),
        Some(AboutMetadata {
            name: Some("DeepSider".into()),
            version: Some("1.3.2".into()),
            comments: Some("DeepSider 网页版的 macOS 原生外壳（Tauri + Rust）".into()),
            ..Default::default()
        }),
    )?;
    let sep = || PredefinedMenuItem::separator(app);

    let app_menu = Submenu::with_items(
        app,
        "DeepSider",
        true,
        &[
            &about,
            &sep()?,
            &PredefinedMenuItem::services(app, Some("服务"))?,
            &sep()?,
            &PredefinedMenuItem::hide(app, Some("隐藏 DeepSider"))?,
            &PredefinedMenuItem::hide_others(app, Some("隐藏其他"))?,
            &PredefinedMenuItem::show_all(app, Some("显示全部"))?,
            &sep()?,
            &PredefinedMenuItem::quit(app, Some("退出 DeepSider"))?,
        ],
    )?;

    let edit_menu = Submenu::with_items(
        app,
        "编辑",
        true,
        &[
            &PredefinedMenuItem::undo(app, Some("撤销"))?,
            &PredefinedMenuItem::redo(app, Some("重做"))?,
            &sep()?,
            &PredefinedMenuItem::cut(app, Some("剪切"))?,
            &PredefinedMenuItem::copy(app, Some("拷贝"))?,
            &PredefinedMenuItem::paste(app, Some("粘贴"))?,
            &sep()?,
            &PredefinedMenuItem::select_all(app, Some("全选"))?,
        ],
    )?;

    let reload = MenuItem::with_id(app, "reload", "重新载入", true, Some("CmdOrCtrl+R"))?;
    let hard_reload =
        MenuItem::with_id(app, "hard_reload", "强制重新载入", true, Some("CmdOrCtrl+Shift+R"))?;
    let stop = MenuItem::with_id(app, "stop", "停止载入", true, Some("CmdOrCtrl+."))?;
    let zoom_reset = MenuItem::with_id(app, "zoom_reset", "实际大小", true, Some("CmdOrCtrl+0"))?;
    let zoom_in = MenuItem::with_id(app, "zoom_in", "放大", true, Some("CmdOrCtrl+="))?;
    let zoom_out = MenuItem::with_id(app, "zoom_out", "缩小", true, Some("CmdOrCtrl+-"))?;
    let fullscreen = PredefinedMenuItem::fullscreen(app, Some("进入全屏幕"))?;

    let view_menu = Submenu::with_items(
        app,
        "显示",
        true,
        &[
            &reload,
            &hard_reload,
            &stop,
            &sep()?,
            &zoom_reset,
            &zoom_in,
            &zoom_out,
            &sep()?,
            &fullscreen,
        ],
    )?;

    let back = MenuItem::with_id(app, "back", "后退", true, Some("CmdOrCtrl+["))?;
    let forward = MenuItem::with_id(app, "forward", "前进", true, Some("CmdOrCtrl+]"))?;
    let home = MenuItem::with_id(app, "home", "主页", true, Some("CmdOrCtrl+Shift+H"))?;
    let find = MenuItem::with_id(app, "find", "查找…", true, Some("CmdOrCtrl+F"))?;
    let external =
        MenuItem::with_id(app, "external", "在默认浏览器中打开", true, Some("CmdOrCtrl+Shift+O"))?;
    let clear = MenuItem::with_id(app, "clear_data", "清除网站数据", true, None::<&str>)?;

    let go_menu = Submenu::with_items(
        app,
        "导览",
        true,
        &[
            &back,
            &forward,
            &home,
            &sep()?,
            &find,
            &external,
            &sep()?,
            &clear,
        ],
    )?;

    let window_menu = Submenu::with_items(
        app,
        "窗口",
        true,
        &[
            &PredefinedMenuItem::minimize(app, Some("最小化"))?,
            &PredefinedMenuItem::maximize(app, Some("缩放"))?,
            &sep()?,
            &PredefinedMenuItem::bring_all_to_front(app, Some("前置全部窗口"))?,
        ],
    )?;

    let help_guide = MenuItem::with_id(app, "help_guide", "使用说明", true, None::<&str>)?;
    let help_site = MenuItem::with_id(app, "help_site", "访问 DeepSider 官网", true, None::<&str>)?;
    let help_menu = Submenu::with_items(app, "帮助", true, &[&help_guide, &help_site])?;

    Menu::with_items(
        app,
        &[
            &app_menu,
            &edit_menu,
            &view_menu,
            &go_menu,
            &window_menu,
            &help_menu,
        ],
    )
}

fn on_menu(app: &AppHandle, id: &str) {
    match id {
        "reload" | "hard_reload" | "stop" | "back" | "forward" | "home" | "external" => {
            perform_nav(app, id)
        }
        "zoom_in" => {
            let cur = *shared(app).zoom.lock().unwrap_or_else(|e| e.into_inner());
            apply_zoom(app, (cur + 0.1 * 1.0001).min(MAX_ZOOM));
        }
        "zoom_out" => {
            let cur = *shared(app).zoom.lock().unwrap_or_else(|e| e.into_inner());
            apply_zoom(app, (cur - 0.1).max(MIN_ZOOM));
        }
        "zoom_reset" => apply_zoom(app, DEFAULT_ZOOM),
        "find" => {
            if let Some(wv) = remote(app) {
                let _ = wv.eval("try{window.__dsFindBar&&window.__dsFindBar.open()}catch(e){}");
            }
        }
        "clear_data" => {
            if let Some(wv) = remote(app) {
                let _ = wv.clear_all_browsing_data();
                let _ = wv.reload();
            }
            shared(app).set_notice("已清除网站数据");
        }
        "help_guide" => {
            shared(app).set_notice("Enter 换行 · ⌘Enter 发送 · ⌘F 查找");
        }
        "help_site" => {
            let _ = std::process::Command::new("open")
                .arg("https://web.deepsider.app/")
                .spawn();
        }
        _ => {}
    }
}

/* ------------------------------------------------------------------ */
/* window setup                                                        */
/* ------------------------------------------------------------------ */

/// Keep a saved window rect only if it still lands on a connected display.
fn restore_frame(app: &AppHandle, saved: Option<SavedFrame>) -> Option<SavedFrame> {
    let f = saved?;
    if f.w < 400.0 || f.h < 300.0 {
        return None;
    }
    let monitors = app.available_monitors().unwrap_or_default();
    if monitors.is_empty() {
        return Some(f);
    }
    let cx = f.x + f.w / 2.0;
    let cy = f.y + f.h / 2.0;
    for m in &monitors {
        let scale = m.scale_factor();
        let wa = m.work_area();
        let x0 = wa.position.x as f64 / scale;
        let y0 = wa.position.y as f64 / scale;
        let x1 = x0 + wa.size.width as f64 / scale;
        let y1 = y0 + wa.size.height as f64 / scale;
        if cx >= x0 && cx <= x1 && cy >= y0 && cy <= y1 {
            return Some(f);
        }
    }
    None
}

fn build_main_window(app: &AppHandle, saved: Saved) -> tauri::Result<()> {
    let frame = restore_frame(app, saved.window);

    let mut builder = WindowBuilder::new(app, "main")
        .title("DeepSider")
        .inner_size(
            frame.map(|f| f.w).unwrap_or(DEFAULT_W),
            frame.map(|f| f.h).unwrap_or(DEFAULT_H),
        )
        .min_inner_size(720.0, 480.0)
        .resizable(true)
        .theme(None) // follow the system appearance
        .visible(false);

    if let Some(f) = frame {
        builder = builder.position(f.x, f.y);
    } else {
        builder = builder.center();
    }

    let win = builder.build()?;

    let size = win.inner_size()?;
    let scale = win.scale_factor().unwrap_or(1.0);
    let logical_w = size.width as f64 / scale;
    let logical_h = (size.height as f64 / scale).max(1.0);

    let remote_builder =
        WebviewBuilder::new("remote", WebviewUrl::External(Url::parse(HOME).unwrap()))
            .user_agent(USER_AGENT)
            .initialization_script(inject::script())
            .zoom_hotkeys_enabled(false)
            .on_navigation(|url| {
                let host = url.host_str().unwrap_or("");
                if is_internal(host) {
                    return true;
                }
                // Anything else belongs to the system browser.
                let _ = std::process::Command::new("open").arg(url.as_str()).spawn();
                false
            })
            .on_new_window(|_url, _features| NewWindowResponse::Allow)
            .on_document_title_changed(|webview, title| {
                let app = webview.app_handle();
                if let Ok(mut t) = shared(&app).title.lock() {
                    *t = title.clone();
                }
                if let Some(win) = app.get_window("main") {
                    let _ = win.set_title(&title);
                }
            })
            .on_page_load(|webview, payload| {
                if matches!(payload.event(), PageLoadEvent::Finished) {
                    let app = webview.app_handle();
                    let zoom = *shared(&app).zoom.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(wv) = app.get_webview("remote") {
                        let _ = wv.set_zoom(zoom);
                    }
                    // The site keys its dark theme off a class, so it has to be
                    // re-applied after every navigation.
                    sync_theme(&app);
                }
            })
            .on_permission_request(|_webview, kind| match kind {
                tauri::webview::PermissionKind::Microphone
                | tauri::webview::PermissionKind::Camera
                | tauri::webview::PermissionKind::Notifications => {
                    tauri::webview::PermissionResponse::Allow
                }
                _ => tauri::webview::PermissionResponse::Default,
            })
            .on_download(|webview, event| match event {
                DownloadEvent::Requested { url, destination } => {
                    let app = webview.app_handle();
                    let dir = app
                        .path()
                        .download_dir()
                        .unwrap_or_else(|_| std::env::temp_dir());
                    let name = url
                        .path_segments()
                        .and_then(|mut s| s.next_back())
                        .filter(|s| !s.is_empty())
                        .unwrap_or("deepsider-download")
                        .to_string();
                    let mut target = dir.join(&name);
                    let mut n = 1;
                    while target.exists() {
                        target = dir.join(format!("{n}-{name}"));
                        n += 1;
                    }
                    *destination = target;
                    shared(&app).set_notice("开始下载…");
                    true
                }
                DownloadEvent::Finished { success, .. } => {
                    let app = webview.app_handle();
                    shared(&app).set_notice(if success { "下载完成" } else { "下载失败" });
                    true
                }
                _ => true,
            });

    win.add_child(
        remote_builder,
        LogicalPosition::new(0.0, 0.0),
        LogicalSize::new(logical_w, logical_h),
    )?;

    // Native toolbar, then geometry (its height feeds `chrome_inset`).
    match win.ns_window() {
        Ok(ptr) => toolbar::install(ptr),
        Err(err) => eprintln!("deepsider: no NSWindow for toolbar: {err}"),
    }
    layout_children(&win);

    // Remember geometry for the next launch.
    let handle = app.clone();
    win.on_window_event(move |event| match event {
        WindowEvent::Resized(_) => {
            if let Some(w) = handle.get_window("main") {
                layout_children(&w);
            }
            if let Some(f) = frame_of(&handle) {
                if let Ok(mut slot) = shared(&handle).frame.lock() {
                    *slot = Some(f);
                }
            }
        }
        WindowEvent::Moved(_) => {
            if let Some(f) = frame_of(&handle) {
                if let Ok(mut slot) = shared(&handle).frame.lock() {
                    *slot = Some(f);
                }
            }
        }
        WindowEvent::ThemeChanged(_) => {
            // macOS flips NSApplication.appearance app-wide; the remote page has
            // to be told, because it does not watch prefers-color-scheme.
            sync_theme(&handle);
        }
        WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed => {
            shared(&handle).save();
        }
        _ => {}
    });

    // Apply the remembered zoom once the remote page is up.
    let zoom = saved.zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    if let Ok(mut z) = shared(app).zoom.lock() {
        *z = zoom;
    }

    win.show()?;
    win.set_focus()?;

    spawn_toolbar_poller(app.clone());
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .menu(|app| build_menu(app))
        .on_menu_event(|app, event| {
            on_menu(app, event.id().as_ref());
        })
        .setup(|app| {
            let handle = app.handle().clone();
            let _ = APP.set(handle.clone());

            let file = handle
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| std::env::temp_dir())
                .join("state.json");
            let saved: Saved = fs::read(&file)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default();
            let zoom = if saved.zoom >= MIN_ZOOM && saved.zoom <= MAX_ZOOM {
                saved.zoom
            } else {
                DEFAULT_ZOOM
            };

            app.manage(Shared {
                notice: Mutex::new(String::new()),
                zoom: Mutex::new(zoom),
                title: Mutex::new("DeepSider".to_string()),
                frame: Mutex::new(saved.window),
                file,
            });

            build_main_window(&handle, saved)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running DeepSider");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_host_matching() {
        assert!(is_internal("web.deepsider.app"));
        assert!(is_internal("deepsider.app"));
        assert!(is_internal("api2.deepsider.me"));
        assert!(is_internal("accounts.google.com"));
        assert!(is_internal("localhost"));
        assert!(!is_internal("example.com"));
        assert!(!is_internal("notdeepsider.app.evil.com"));
    }
}
