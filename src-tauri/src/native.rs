//! Thin, `msg_send!`-only access to the handful of `WKWebView` members that Tauri does
//! not expose (history, loading state, progress, hard reload).
//!
//! Everything here is called from inside `Webview::with_webview`, i.e. on the main
//! thread with a live `WKWebView` pointer. Only primitive return types are used
//! (`bool`/`f64`/`void`/object pointers we immediately drop), so no Objective-C
//! ownership rules are violated and no retain/release juggling is needed.

use core::ffi::c_void;
use objc2::msg_send;
use objc2::runtime::AnyObject;

#[inline]
fn obj(wk: *mut c_void) -> *mut AnyObject {
    wk as *mut AnyObject
}

/// `-[WKWebView canGoBack]`
pub fn can_go_back(wk: *mut c_void) -> bool {
    if wk.is_null() {
        return false;
    }
    unsafe { msg_send![obj(wk), canGoBack] }
}

/// `-[WKWebView canGoForward]`
pub fn can_go_forward(wk: *mut c_void) -> bool {
    if wk.is_null() {
        return false;
    }
    unsafe { msg_send![obj(wk), canGoForward] }
}

/// `-[WKWebView goBack]`
pub fn go_back(wk: *mut c_void) {
    if wk.is_null() {
        return;
    }
    let _: *mut AnyObject = unsafe { msg_send![obj(wk), goBack] };
}

/// `-[WKWebView goForward]`
pub fn go_forward(wk: *mut c_void) {
    if wk.is_null() {
        return;
    }
    let _: *mut AnyObject = unsafe { msg_send![obj(wk), goForward] };
}

/// `-[WKWebView isLoading]`
pub fn is_loading(wk: *mut c_void) -> bool {
    if wk.is_null() {
        return false;
    }
    unsafe { msg_send![obj(wk), isLoading] }
}

/// `-[WKWebView estimatedProgress]`, clamped to `0.0..=1.0`.
#[allow(dead_code)]
pub fn estimated_progress(wk: *mut c_void) -> f64 {
    if wk.is_null() {
        return 0.0;
    }
    let p: f64 = unsafe { msg_send![obj(wk), estimatedProgress] };
    if p.is_finite() {
        p.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// `-[WKWebView stopLoading]`
pub fn stop_loading(wk: *mut c_void) {
    if wk.is_null() {
        return;
    }
    let _: () = unsafe { msg_send![obj(wk), stopLoading] };
}

/// `-[WKWebView reloadFromOrigin]` — a real "force reload", bypassing the cache.
pub fn reload_from_origin(wk: *mut c_void) {
    if wk.is_null() {
        return;
    }
    let _: *mut AnyObject = unsafe { msg_send![obj(wk), reloadFromOrigin] };
}

/// Geometry of the window that hosts this webview, in logical points.
///
/// On macOS tao installs a full-size content view, so the *visible* content
/// area is smaller than the content view itself: the title bar overlaps its
/// top edge. Returns `(content_view_height, visible_content_height)`; the
/// difference is the title bar inset that a top-anchored child webview has to
/// clear.
pub fn content_metrics(wk: *mut c_void) -> Option<(f64, f64)> {
    if wk.is_null() {
        return None;
    }
    unsafe {
        use objc2_foundation::NSRect;
        let w: *mut AnyObject = msg_send![obj(wk), window];
        if w.is_null() {
            return None;
        }
        let cv: *mut AnyObject = msg_send![w, contentView];
        if cv.is_null() {
            return None;
        }
        let cv_frame: NSRect = msg_send![cv, frame];
        let layout: NSRect = msg_send![w, contentLayoutRect];
        let full = cv_frame.size.height;
        if !full.is_finite() || full <= 0.0 {
            return None;
        }
        let visible = if layout.size.height.is_finite() && layout.size.height > 0.0 {
            layout.size.height
        } else {
            full
        };
        Some((full, visible))
    }
}

/// Whether the effective appearance (system-wide, since Tauri sets it on
/// `NSApplication`) currently resolves to dark.
///
/// Compares `bestMatchFromAppearancesWithNecessaryAppearances:` against
/// `NSAppearanceNameDarkAqua`, which is how AppKit itself answers "is this
/// view dark right now" without caring which concrete appearance name is set.
pub fn is_dark_appearance(wk: *mut c_void) -> bool {
    if wk.is_null() {
        return false;
    }
    unsafe {
        let obj = |p: *mut c_void| p as *mut AnyObject;
        let view = obj(wk);

        // Prefer the webview's own effective appearance, fall back to the app's.
        let mut appearance: *mut AnyObject = msg_send![view, effectiveAppearance];
        if appearance.is_null() {
            let app: *mut AnyObject = msg_send![objc2::class!(NSApplication), sharedApplication];
            if app.is_null() {
                return false;
            }
            appearance = msg_send![app, effectiveAppearance];
        }
        if appearance.is_null() {
            return false;
        }

        let dark_name: *mut AnyObject = msg_send![
            objc2::class!(NSString),
            stringWithUTF8String: b"NSAppearanceNameDarkAqua\0".as_ptr()
        ];
        if dark_name.is_null() {
            return false;
        }

        // -[NSAppearance bestMatchFromAppearancesWithNecessaryAppearances:]
        // returns the entry of the candidate list it best matches.
        let candidates: *mut AnyObject = msg_send![
            objc2::class!(NSArray),
            arrayWithObject: dark_name
        ];
        if candidates.is_null() {
            return false;
        }
        let best: *mut AnyObject =
            msg_send![appearance, bestMatchFromAppearancesWithNecessaryAppearances: candidates];
        if best.is_null() {
            return false;
        }
        let equal: bool = msg_send![best, isEqual: dark_name];
        equal
    }
}
