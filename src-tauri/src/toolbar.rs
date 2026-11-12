//! Native AppKit toolbar, mirroring the Swift 1.2.0 build.
//!
//! 1.3.0 hosted a 48 pt HTML strip above the page. That read as browser
//! chrome, so 1.3.1 drops it and installs a real `NSToolbar` with the six
//! icons the Swift build had: back / forward / reload / hard reload / stop /
//! home, followed by a trailing flexible space.
//!
//! Everything runs on the main thread and is driven through raw AppKit
//! pointers, because tauri does not model toolbars.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSImage, NSToolbar, NSToolbarDelegate, NSToolbarDisplayMode, NSToolbarItem,
    NSToolbarItemIdentifier, NSWindow, NSWindowToolbarStyle,
};
use objc2_foundation::{NSArray, NSString};

/// Selector tag per button; -1 for unknown items.
const BACK: isize = 0;
const FORWARD: isize = 1;
const RELOAD: isize = 2;
const HARD_RELOAD: isize = 3;
const STOP: isize = 4;
const HOME: isize = 5;

struct Spec {
    id: &'static str,
    label: &'static str,
    symbol: &'static str,
    tooltip: &'static str,
    tag: isize,
}

/// Mirrors `WebWindowController.ItemID` from the Swift build.
const SPECS: [Spec; 6] = [
    Spec {
        id: "ds.back",
        label: "后退",
        symbol: "chevron.backward",
        tooltip: "后退（⌘[）",
        tag: BACK,
    },
    Spec {
        id: "ds.forward",
        label: "前进",
        symbol: "chevron.forward",
        tooltip: "前进（⌘]）",
        tag: FORWARD,
    },
    Spec {
        id: "ds.reload",
        label: "重新载入",
        symbol: "arrow.clockwise",
        tooltip: "重新载入（⌘R）",
        tag: RELOAD,
    },
    Spec {
        id: "ds.hard_reload",
        label: "强制重新载入",
        symbol: "arrow.triangle.2.circlepath",
        tooltip: "忽略缓存重新载入（⇧⌘R）",
        tag: HARD_RELOAD,
    },
    Spec {
        id: "ds.stop",
        label: "停止载入",
        symbol: "xmark",
        tooltip: "停止载入（⌘.）",
        tag: STOP,
    },
    Spec {
        id: "ds.home",
        label: "主页",
        symbol: "house",
        tooltip: "回到主页（⇧⌘H）",
        tag: HOME,
    },
];

/// Kept in sync by `crate::nav_poll` so the toolbar can grey itself out.
static CAN_BACK: AtomicBool = AtomicBool::new(false);
static CAN_FORWARD: AtomicBool = AtomicBool::new(false);
static LOADING: AtomicBool = AtomicBool::new(false);
/// Raw `NSToolbar` pointer (0 until `install` runs). The window owns the
/// toolbar, so it stays valid for as long as the window does.
static TOOLBAR: AtomicUsize = AtomicUsize::new(0);

fn enabled_for(tag: isize) -> bool {
    match tag {
        BACK => CAN_BACK.load(Ordering::Relaxed),
        FORWARD => CAN_FORWARD.load(Ordering::Relaxed),
        STOP => LOADING.load(Ordering::Relaxed),
        _ => true,
    }
}

fn action_for(tag: isize) -> Option<&'static str> {
    match tag {
        BACK => Some("back"),
        FORWARD => Some("forward"),
        RELOAD => Some("reload"),
        HARD_RELOAD => Some("hard_reload"),
        STOP => Some("stop"),
        HOME => Some("home"),
        _ => None,
    }
}

fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

/// Ordered identifiers the toolbar shows by default: the six buttons plus a
/// trailing flexible space.
fn default_identifiers() -> Retained<NSArray<NSToolbarItemIdentifier>> {
    let ids: Vec<Retained<NSString>> = SPECS.iter().map(|s| ns(s.id)).collect();
    let refs: Vec<&NSString> = ids.iter().map(|s| &**s).collect();
    NSArray::from_slice(&refs)
}

/// Build one toolbar item, or `None` for an identifier we do not own.
fn build_item(
    identifier: &NSToolbarItemIdentifier,
    target: &AnyObject,
) -> Option<Retained<NSToolbarItem>> {
    let id = identifier.to_string();
    let spec = SPECS.iter().find(|s| s.id == id)?;
    let mtm = MainThreadMarker::new()?;
    let allocated = NSToolbarItem::alloc(mtm);
    let item = NSToolbarItem::initWithItemIdentifier(allocated, identifier);
    item.setLabel(&ns(spec.label));
    item.setPaletteLabel(&ns(spec.label));
    item.setToolTip(Some(&ns(spec.tooltip)));
    if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &ns(spec.symbol),
        Some(&ns(spec.label)),
    ) {
        item.setImage(Some(&image));
    }
    item.setTag(spec.tag);
    item.setEnabled(enabled_for(spec.tag));
    // Mark them as navigational so AppKit lays them out at the leading edge,
    // the way the 1.2 build's toolbar looked.
    item.setNavigational(true);
    unsafe {
        item.setTarget(Some(target));
        item.setAction(Some(sel!(dsNavAction:)));
    }
    Some(item)
}

define_class!(
    /// Target + delegate for the DeepSider toolbar. One instance lives for the
    /// whole process (leaked on purpose) because AppKit keeps the delegate and
    /// item targets as weak references.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ()]
    #[name = "DeepSiderToolbarController"]
    struct ToolbarController;

    impl ToolbarController {
        /// One action for every button; the sender's `tag` picks the command.
        #[unsafe(method(dsNavAction:))]
        fn nav_action(&self, sender: &NSToolbarItem) {
            if let Some(action) = action_for(sender.tag()) {
                crate::dispatch_nav(action);
            }
        }

        /// AppKit asks this on every validation pass; answering here keeps the
        /// buttons honest (`setEnabled` alone gets overwritten by autovalidation).
        #[unsafe(method(validateToolbarItem:))]
        fn validate_item(&self, item: &NSToolbarItem) -> bool {
            enabled_for(item.tag())
        }
    }

    unsafe impl NSObjectProtocol for ToolbarController {}

    unsafe impl NSToolbarDelegate for ToolbarController {
        #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
        fn default_item_identifiers(
            &self,
            _toolbar: &NSToolbar,
        ) -> Retained<NSArray<NSToolbarItemIdentifier>> {
            default_identifiers()
        }

        #[unsafe(method_id(toolbarAllowedItemIdentifiers:))]
        fn allowed_item_identifiers(
            &self,
            _toolbar: &NSToolbar,
        ) -> Retained<NSArray<NSToolbarItemIdentifier>> {
            default_identifiers()
        }

        #[unsafe(method_id(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn item_for_identifier(
            &self,
            _toolbar: &NSToolbar,
            identifier: &NSToolbarItemIdentifier,
            _flag: bool,
        ) -> Option<Retained<NSToolbarItem>> {
            let target: &AnyObject = unsafe { &*(self as *const Self as *const AnyObject) };
            build_item(identifier, target)
        }
    }
);

fn make_controller() -> Retained<ToolbarController> {
    let mtm = MainThreadMarker::new().expect("toolbar controller needs the main thread");
    unsafe {
        let this = ToolbarController::alloc(mtm).set_ivars(());
        msg_send![super(this), init]
    }
}

/// Install the toolbar on the `NSWindow` behind `ns_window`.
pub fn install(ns_window: *mut c_void) {
    if ns_window.is_null() {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else { return };
    let window: &NSWindow = unsafe { &*(ns_window as *const NSWindow) };

    let toolbar = {
        let allocated = NSToolbar::alloc(mtm);
        NSToolbar::initWithIdentifier(allocated, &ns("DeepSiderToolbar"))
    };
    toolbar.setDisplayMode(NSToolbarDisplayMode::IconOnly);
    toolbar.setAllowsUserCustomization(false);
    toolbar.setAutosavesConfiguration(false);

    // Leaked on purpose — AppKit holds only weak references to the delegate
    // and to each item's target. Exactly one allocation per process.
    let ctrl: &'static ToolbarController = Box::leak(Box::new(make_controller()));
    let proto: &ProtocolObject<dyn NSToolbarDelegate> = ProtocolObject::from_ref(ctrl);
    toolbar.setDelegate(Some(proto));

    window.setToolbar(Some(&toolbar));
    window.setToolbarStyle(NSWindowToolbarStyle::Unified);
    window.setTitlebarAppearsTransparent(false);

    TOOLBAR.store(&*toolbar as *const NSToolbar as usize, Ordering::Release);
}

/// Refresh the enabled state of the six buttons. Must run on the main thread.
pub fn refresh_state(back: bool, forward: bool, loading: bool) {
    CAN_BACK.store(back, Ordering::Relaxed);
    CAN_FORWARD.store(forward, Ordering::Relaxed);
    LOADING.store(loading, Ordering::Relaxed);

    let raw = TOOLBAR.load(Ordering::Acquire);
    if raw == 0 {
        return;
    }
    let Some(_mtm) = MainThreadMarker::new() else { return };
    let toolbar: &NSToolbar = unsafe { &*(raw as *const NSToolbar) };
    toolbar.validateVisibleItems();
}
