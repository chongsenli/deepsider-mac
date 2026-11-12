//! JavaScript injected into the remote DeepSider page at **document start**
//! (via `WebviewBuilder::initialization_script`).
//!
//! Everything here exists because the remote SPA runs inside a WKWebView and
//! behaves badly in three specific ways:
//!
//! 1. **ENTER_FIX** — the Quill composer binds a bare `Enter` to "send". One
//!    stray keypress therefore fires a message the user never meant to send
//!    (very easy to do with Chinese IME candidate windows). We intercept the
//!    key in the capture phase and turn a bare `Enter` into a newline, keeping
//!    `Shift+Enter` as the site's own newline and making `Cmd/Ctrl+Enter` the
//!    only way to send. IME composition (`isComposing`) is left
//!    completely alone so Chinese input still works, and real `<input>` /
//!    `<textarea>` fields keep their normal Enter-to-submit behaviour.
//! 2. **FIND_BAR** — WKWebView has no native find UI, and the app does not
//!    ship one either, so there is no way to search a long conversation. We
//!    add a small floating find bar driven by `window.find`, wired to ⌘F/Ctrl+F.
//! 3. **DIALOG_GUARD** — WKWebView silently drops `window.alert`, so error
//!    messages from the app would vanish without a trace. We replace `alert`
//!    with an in-page toast. `confirm` / `prompt` are deliberately *not*
//!    replaced: a JS replacement cannot return a synchronous answer, so faking
//!    them would be a lie (see the comment block in `DIALOG_GUARD`).
//! 4. **THEME_SYNC** — the site implements dark mode with a `dark` class on
//!    `<html>` (its stylesheet is full of `html.app.dark` / `.dark` rules) and
//!    never consults `prefers-color-scheme`. WKWebView only reports the system
//!    appearance, so we bridge the two: the Rust side pushes the appearance
//!    (`window.__dsSetTheme`) and this part also re-asserts it if the page
//!    strips the class.
//!
//! Conventions: plain ES5-ish JavaScript, every DOM access wrapped in
//! try/catch, no `eval`, no network access, no storage writes, no external
//! crates on the Rust side.

/// JavaScript injected into the remote DeepSider page at document start.
///
/// Returns one self-contained IIFE guarded by `window.__dsInjected`, so the
/// script is idempotent even if a document is re-initialised.
pub fn script() -> String {
    let parts: [&str; 4] = [ENTER_FIX, FIND_BAR, DIALOG_GUARD, THEME_SYNC];
    let mut out = String::from(
        "(function(){\nif (window.__dsInjected) return;\nwindow.__dsInjected = true;\n",
    );
    for p in parts {
        out.push_str(p);
        out.push('\n');
    }
    out.push_str("})();");
    out
}

/// Enter must not send a message. Bare `Enter` inserts a newline,
/// `Shift+Enter` is left to the site, `Cmd/Ctrl+Enter` clicks the send button.
const ENTER_FIX: &str = r#"
/* ---------- Part 1: Enter fix (ported from the battle-tested Swift build) ---------- */

function isComposer(node) {
  if (!node) return false;
  var el = node.nodeType === 3 ? node.parentElement : node;
  if (!el || !el.closest) return false;
  return !!el.closest('.ql-editor, .ql-container, [contenteditable="true"]');
}

function insertNewline() {
  // execCommand keeps the Quill model in sync; a raw DOM insert does not.
  var ok = false;
  try { ok = document.execCommand('insertLineBreak', false, null); } catch (e) { ok = false; }
  if (!ok) { try { ok = document.execCommand('insertParagraph', false, null); } catch (e) { ok = false; } }
  return ok;
}

function clickSend() {
  try {
    var ed = document.querySelector('.ql-editor');
    if (!ed) return;
    var wrap = ed.closest('[class*=rounded]') || ed.parentElement;
    var btn = null;
    for (var i = 0; i < 6 && wrap && !btn; i++) {
      btn = wrap.querySelector('[class*=send_chat], [class*=sendChat], [class*=iconv3-send]');
      if (!btn) wrap = wrap.parentElement;
    }
    if (btn) btn.click();
  } catch (e) {}
}

window.addEventListener('keydown', function (e) {
  try {
    if (!e || e.key !== 'Enter') return;
    // Never interfere with IME composition (critical for Chinese input).
    // NOTE: macOS WKWebView reports `keyCode === 229` for *every* trusted key
    // event, so `keyCode` must not be used as an IME signal here — only
    // `isComposing` (and a non-Enter `key`, which is already filtered above).
    if (e.isComposing) return;

    var target = e.target;
    var el = target && target.nodeType === 3 ? target.parentElement : target;
    // Real text inputs (search box, rename field, ...) keep Enter-to-submit.
    if (el && el.closest && el.closest('input, textarea')) return;

    if (!isComposer(target)) return;

    if (e.metaKey || e.ctrlKey) {
      e.preventDefault();
      e.stopImmediatePropagation();
      clickSend();
      return;
    }
    if (e.shiftKey) return; // let the site insert its own soft newline
    e.preventDefault();
    e.stopImmediatePropagation();
    insertNewline();
  } catch (err) {}
}, true);

"#;

/// An in-page find bar (`window.__dsFindBar`), because neither the app nor
/// WKWebView provides a find UI.
const FIND_BAR: &str = r#"
/* ---------- Part 2: in-page find bar ---------- */

var __dsFindCss = [
  '#__dsFindBar{position:fixed;top:8px;right:12px;z-index:2147483600;width:300px;box-sizing:border-box;display:none;align-items:center;gap:6px;padding:7px 8px;border-radius:8px;border:1px solid rgba(0,0,0,0.12);background:#ffffff;color:#1d1d1f;box-shadow:0 8px 24px rgba(0,0,0,0.18);font:12.5px -apple-system,BlinkMacSystemFont,Arial,sans-serif;-webkit-font-smoothing:antialiased;}',
  '#__dsFindBar .__ds-find-input{flex:1 1 auto;min-width:0;margin:0;padding:3px 6px;border:1px solid rgba(0,0,0,0.14);border-radius:5px;background:#f6f6f7;color:#1d1d1f;font:inherit;outline:none;box-sizing:border-box;}',
  '#__dsFindBar .__ds-find-count{flex:0 0 auto;font-size:11.5px;opacity:0.7;white-space:nowrap;display:none;}',
  '#__dsFindBar .__ds-find-btn{flex:0 0 auto;margin:0;padding:2px 5px;border:0;border-radius:5px;background:transparent;color:inherit;font:inherit;line-height:1.2;cursor:pointer;-webkit-appearance:none;appearance:none;}',
  '#__dsFindBar .__ds-find-btn:hover{background:rgba(0,0,0,0.08);}',
  '@media (prefers-color-scheme: dark){#__dsFindBar{background:#2b2b2e;color:#e8e8ea;border-color:rgba(255,255,255,0.14);box-shadow:0 8px 24px rgba(0,0,0,0.5);}#__dsFindBar .__ds-find-input{background:#3a3a3e;color:#e8e8ea;border-color:rgba(255,255,255,0.16);}#__dsFindBar .__ds-find-btn:hover{background:rgba(255,255,255,0.12);}}'
].join('');

var __dsFindBarEl = null;
var __dsFindInput = null;
var __dsFindCounter = null;
var __dsFindVisitedBody = false;

function __dsFindEnsureStyle() {
  try {
    if (document.getElementById('__dsFindStyle')) return;
    var s = document.createElement('style');
    s.id = '__dsFindStyle';
    s.appendChild(document.createTextNode(__dsFindCss));
    (document.head || document.documentElement).appendChild(s);
  } catch (e) {}
}

// window.find(aString, aCaseSensitive, aBackwards, aWrapAround, aWholeWord,
//             aSearchInFrames, aShowDialog) - WebKit only, which is all we need.
function __dsFindRun(txt, backwards) {
  try {
    return window.find(txt, false, !!backwards, true, false, false, false);
  } catch (e) {
    return false;
  }
}

function __dsFindSetState(found, hasText) {
  try {
    if (!__dsFindCounter) return;
    if (!hasText || found) {
      __dsFindCounter.textContent = '';
      __dsFindCounter.style.display = 'none';
    } else {
      __dsFindCounter.textContent = '无结果';
      __dsFindCounter.style.display = 'inline';
    }
  } catch (e) {}
}

function __dsFindUpdate() {
  try {
    var txt = __dsFindInput ? __dsFindInput.value : '';
    if (!txt) { __dsFindSetState(false, false); return; }
    __dsFindSetState(__dsFindRun(txt, false), true);
  } catch (e) {}
}

function __dsFindStep(backwards) {
  try {
    var txt = __dsFindInput ? __dsFindInput.value : '';
    if (!txt) return;
    __dsFindSetState(__dsFindRun(txt, backwards), true);
  } catch (e) {}
}

function __dsFindMakeBtn(glyph, label, handler) {
  var b = document.createElement('button');
  b.type = 'button';
  b.className = '__ds-find-btn';
  b.textContent = glyph;
  b.setAttribute('aria-label', label);
  b.setAttribute('title', label);
  b.addEventListener('click', function (ev) {
    try { ev.preventDefault(); ev.stopPropagation(); } catch (e) {}
    handler();
  });
  return b;
}

function __dsFindBuild() {
  if (__dsFindBarEl) return true;
  try {
    if (!document.body) return false;
    __dsFindEnsureStyle();

    var bar = document.createElement('div');
    bar.id = '__dsFindBar';
    bar.setAttribute('role', 'search');

    var input = document.createElement('input');
    input.type = 'text';
    input.className = '__ds-find-input';
    input.setAttribute('placeholder', '在页面中查找');
    input.setAttribute('aria-label', '在页面中查找');
    input.setAttribute('spellcheck', 'false');
    input.setAttribute('autocomplete', 'off');
    input.addEventListener('input', function () { __dsFindUpdate(); });
    input.addEventListener('keydown', function (ev) {
      try {
        if (!ev) return;
        if (ev.key === 'Escape') {
          ev.preventDefault();
          ev.stopImmediatePropagation();
          __dsFindClose();
        } else if (ev.key === 'Enter') {
          ev.preventDefault();
          ev.stopImmediatePropagation();
          __dsFindStep(!!ev.shiftKey);
        }
      } catch (e) {}
    });

    var counter = document.createElement('span');
    counter.className = '__ds-find-count';
    counter.setAttribute('aria-live', 'polite');

    bar.appendChild(input);
    bar.appendChild(counter);
    bar.appendChild(__dsFindMakeBtn('▲', '上一个', function () { __dsFindStep(true); }));
    bar.appendChild(__dsFindMakeBtn('▼', '下一个', function () { __dsFindStep(false); }));
    bar.appendChild(__dsFindMakeBtn('✕', '关闭', function () { __dsFindClose(); }));

    // Keep the page from stealing focus / closing us on mousedown.
    bar.addEventListener('mousedown', function (ev) {
      try { ev.stopPropagation(); } catch (e) {}
    });

    document.body.appendChild(bar);
    __dsFindBarEl = bar;
    __dsFindInput = input;
    __dsFindCounter = counter;
    return true;
  } catch (e) {
    return false;
  }
}

function __dsFindOpen() {
  try {
    if (!__dsFindBuild()) {
      // document.body not there yet (very early call): retry after parse.
      if (!__dsFindVisitedBody) {
        __dsFindVisitedBody = true;
        document.addEventListener('DOMContentLoaded', function () {
          try { __dsFindOpen(); } catch (e) {}
        });
      }
      return;
    }
    __dsFindBarEl.style.display = 'flex';
    try { __dsFindInput.focus(); } catch (e) {}
    try { __dsFindInput.select(); } catch (e) {}
    __dsFindUpdate();
  } catch (e) {}
}

function __dsFindClose() {
  try {
    if (__dsFindBarEl) __dsFindBarEl.style.display = 'none';
    var sel = window.getSelection ? window.getSelection() : null;
    if (sel) sel.removeAllRanges();
    __dsFindSetState(false, false);
  } catch (e) {}
}

window.__dsFindBar = {
  open: function () { __dsFindOpen(); },
  close: function () { __dsFindClose(); }
};

// ⌘F / Ctrl+F anywhere opens the find bar; Escape closes it.
document.addEventListener('keydown', function (e) {
  try {
    if (!e) return;
    var key = e.key;
    if ((e.metaKey || e.ctrlKey) && !e.altKey && (key === 'f' || key === 'F')) {
      e.preventDefault();
      e.stopImmediatePropagation();
      __dsFindOpen();
      return;
    }
    if (key === 'Escape' && __dsFindBarEl && __dsFindBarEl.style.display === 'flex') {
      e.preventDefault();
      e.stopImmediatePropagation();
      __dsFindClose();
    }
  } catch (err) {}
}, true);
"#;

/// WKWebView drops native JS dialogs: `alert` becomes a toast. `confirm` and
/// `prompt` are left untouched on purpose.
const DIALOG_GUARD: &str = r#"
/* ---------- Part 3: native dialog guard ---------- */

var __dsToastCss = [
  '#__dsToast{position:fixed;left:50%;bottom:28px;transform:translateX(-50%);z-index:2147483600;max-width:70vw;box-sizing:border-box;padding:10px 16px;border-radius:8px;border:1px solid rgba(0,0,0,0.12);background:#ffffff;color:#1d1d1f;box-shadow:0 10px 30px rgba(0,0,0,0.2);font:12.5px -apple-system,BlinkMacSystemFont,Arial,sans-serif;white-space:pre-wrap;word-break:break-word;display:none;opacity:0;transition:opacity 0.2s ease;}',
  '@media (prefers-color-scheme: dark){#__dsToast{background:#2b2b2e;color:#e8e8ea;border-color:rgba(255,255,255,0.14);box-shadow:0 10px 30px rgba(0,0,0,0.55);}}'
].join('');

var __dsToastEl = null;
var __dsToastTimer = 0;

function __dsToastEnsureStyle() {
  try {
    if (document.getElementById('__dsToastStyle')) return;
    var s = document.createElement('style');
    s.id = '__dsToastStyle';
    s.appendChild(document.createTextNode(__dsToastCss));
    (document.head || document.documentElement).appendChild(s);
  } catch (e) {}
}

function __dsToastShow(message) {
  try {
    if (!document.body) return;
    __dsToastEnsureStyle();
    var el = __dsToastEl;
    if (!el) {
      el = document.createElement('div');
      el.id = '__dsToast';
      el.setAttribute('role', 'status');
      el.setAttribute('aria-live', 'polite');
      document.body.appendChild(el);
      __dsToastEl = el;
    }
    el.textContent = (message === undefined || message === null) ? '' : String(message);
    el.style.display = 'block';
    el.style.opacity = '1';
    if (__dsToastTimer) {
      try { clearTimeout(__dsToastTimer); } catch (e) {}
      __dsToastTimer = 0;
    }
    __dsToastTimer = setTimeout(function () {
      try {
        el.style.opacity = '0';
        setTimeout(function () {
          try { if (el.style.opacity === '0') el.style.display = 'none'; } catch (e) {}
        }, 220);
      } catch (e) {}
    }, 4000);
  } catch (e) {}
}

// WKWebView silently drops alert(), so replace it with a toast.
// Returns undefined, exactly like the native alert.
try {
  window.alert = function (message) {
    __dsToastShow(message);
    return undefined;
  };
} catch (e) {}

/*
 * window.confirm and window.prompt are deliberately NOT replaced.
 *
 * Both must return an answer synchronously, within the same microtask as the
 * call, and no DOM-based replacement can do that: a fake always has to pick
 * some default answer up front, which either silently confirms destructive
 * actions the user never approved or silently cancels the ones they did. That
 * is worse than WKWebView's current behaviour, which simply returns the
 * "dismissed" value. Leave them alone so their semantics stay truthful.
 *
 * window.print is likewise left alone: the page itself triggers printing via
 * the app's own UI, and intercepting it would break that flow.
 */
"#;

const THEME_SYNC: &str = r#"
/* ---------- Part 4: theme sync (system appearance -> site `dark` class) ---------- */

/*
 * The DeepSider stylesheet never mentions `prefers-color-scheme`; dark mode is
 * a `dark` class on <html> (`html.app.dark{...}`, plus ~100 `.dark ...` rules
 * and Tailwind `dark:` variants compiled to `:is(.dark *)`). A plain WKWebView
 * therefore stays light no matter what macOS is set to.
 *
 * `__dsSetTheme` is the entry point the Rust side calls (via `eval`) on launch
 * and on every `ThemeChanged` window event. The script also installs a
 * MutationObserver, because the SPA rewrites <html> classes during route
 * changes and would otherwise drop the class again.
 *
 * Adding the class is not enough on its own: the light palette is hard-coded
 * into an inline <style> (`.deepsider-root{--cg-color-b1:255 255 255; ...}`)
 * while the dark palette lives in a JS object that the app only applies after
 * it has mounted and decided on a theme. Without the palette below the page
 * turns white-on-white-ish: `html.app.dark` flips the text colour to #fff but
 * every `bg-color-b*` surface keeps resolving to the light values. So we
 * declare the dark palette ourselves, one specificity step above the site rule
 * (`html.dark .deepsider-root` beats `.deepsider-root`).
 */
var __dsThemeDark = null;

var __DS_DARK_VARS = [
  '--cg-color-blue: 11 121 255',
  '--cg-color-green: 48 209 88',
  '--cg-color-yellow: 255 214 10',
  '--cg-color-orange: 255 159 10',
  '--cg-color-red: 255 69 58',
  '--cg-color-t1: 255 255 255',
  '--cg-color-t5: 28 28 30',
  '--cg-color-b1: 17 17 17',
  '--cg-color-b2: 28 29 36',
  '--cg-color-b3: 47 47 62',
  '--cg-color-b4: 67 67 85',
  '--cg-color-b5: 47 47 62',
  '--cg-color-e1: 255 255 255',
  '--cg-color-m1: 0 0 0',
  '--cg-color-m2: 255 255 255'
].join(';');

function __dsApplyVars(dark) {
  try {
    var host = document.head || document.documentElement;
    if (!host) return;
    var tag = document.getElementById('__dsThemeVars');
    if (!tag) {
      tag = document.createElement('style');
      tag.id = '__dsThemeVars';
      host.appendChild(tag);
    }
    // t2..t4 are defined as `var(--cg-color-t1) / 0.8`, so they follow the
    // override automatically and do not need to be restated.
    tag.textContent = dark
      ? 'html.dark .deepsider-root, html.dark .deepsider-root::before, html.dark .deepsider-root::after {' +
        __DS_DARK_VARS +
        '}'
      : '';
  } catch (e) {}
}

function __dsApplyTheme(dark) {
  try {
    __dsThemeDark = !!dark;
    var html = document.documentElement;
    if (!html) return;
    if (dark) html.classList.add('dark');
    else html.classList.remove('dark');
    // Native form controls, scrollbars and the canvas keep matching the theme.
    html.style.colorScheme = dark ? 'dark' : 'light';
    var body = document.body;
    if (body) body.style.colorScheme = dark ? 'dark' : 'light';
    __dsApplyVars(dark);
  } catch (e) {}
}

window.__dsSetTheme = function (dark) {
  __dsApplyTheme(dark);
  return true;
};

// Re-assert after the SPA touches <html> (it adds/removes `app`, `h-full`, ...).
try {
  var __dsThemeObserver = new MutationObserver(function () {
    if (__dsThemeDark === null) return;
    try {
      var html = document.documentElement;
      var has = html.classList.contains('dark');
      if (has !== __dsThemeDark) __dsApplyTheme(__dsThemeDark);
    } catch (e) {}
  });
  __dsThemeObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['class']
  });
} catch (e) {}

// Paint the right theme before the first frame whenever the system already
// reports one (the Rust side re-sends it explicitly, this only kills the
// white flash during startup).
try {
  if (window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches) {
    __dsApplyTheme(true);
  }
} catch (e) {}

"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_is_an_iife_containing_every_part() {
        let js = script();
        assert!(
            js.starts_with("(function()"),
            "script must open as an IIFE, got: {}",
            &js[..js.len().min(40)]
        );
        assert!(js.contains("__dsInjected"), "missing idempotence guard");
        assert!(js.contains("__dsFindBar"), "missing find bar");
        assert!(
            js.contains("__dsSetTheme"),
            "missing theme bridge (__dsSetTheme)"
        );
        assert!(
            js.contains("__dsThemeVars") && js.contains("--cg-color-b1: 17 17 17"),
            "missing dark palette override"
        );
        assert!(
            js.contains("insertLineBreak"),
            "missing enter-fix marker (insertLineBreak)"
        );
        assert!(js.trim_end().ends_with("})();"), "script must be closed");
    }
}
