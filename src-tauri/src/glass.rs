//! Native Liquid Glass chrome (macOS 26+).
//!
//! The webview is transparent; behind it we place
//!   1. a window-background material (NSVisualEffectView) filling the window, and
//!   2. a floating Liquid Glass panel (NSGlassEffectView) where the sidebar sits.
//! The CSS keeps the sidebar area transparent (`html.native-glass`) so the real glass shows.
//! On systems without NSGlassEffectView nothing is installed and the CSS draws its own glass.

use objc2::{rc::Retained, runtime::AnyClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSGlassEffectView, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
    NSVisualEffectState, NSVisualEffectView, NSWindow, NSWindowOrderingMode,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use std::sync::atomic::{AtomicBool, Ordering};

/// Must match the sidebar column width and margin in style.css.
const SIDEBAR_W: f64 = 240.0;
const INSET: f64 = 8.0;
const RADIUS: f64 = 18.0;

static ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

pub fn install(window: &tauri::WebviewWindow) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    if AnyClass::get(c"NSGlassEffectView").is_none() {
        return;
    }
    let (Ok(win_ptr), Ok(view_ptr)) = (window.ns_window(), window.ns_view()) else { return };
    // SAFETY: Tauri hands out the live NSWindow / WKWebView pointers for this window, on the main thread.
    let ns_window: &NSWindow = unsafe { &*(win_ptr as *const NSWindow) };
    let webview: &NSView = unsafe { &*(view_ptr as *const NSView) };
    let fill = NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable;

    // The view that hosts the webview. If the webview *is* the content view, wrap it.
    let host: Retained<NSView> = match unsafe { webview.superview() } {
        Some(parent) => parent,
        None => {
            let Some(content) = ns_window.contentView() else { return };
            let container = NSView::initWithFrame(NSView::alloc(mtm), content.frame());
            ns_window.setContentView(Some(&container));
            webview.setFrame(container.bounds());
            webview.setAutoresizingMask(fill);
            container.addSubview(webview);
            container
        }
    };
    let bounds = host.bounds();

    let bg = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), bounds);
    bg.setMaterial(NSVisualEffectMaterial::UnderWindowBackground);
    bg.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    bg.setState(NSVisualEffectState::FollowsWindowActiveState);
    bg.setAutoresizingMask(fill);
    host.addSubview_positioned_relativeTo(&bg, NSWindowOrderingMode::Below, Some(webview));

    // Symmetric inset, so it does not matter whether `host` is flipped.
    let frame = NSRect::new(
        NSPoint::new(INSET, INSET),
        NSSize::new(SIDEBAR_W - 2.0 * INSET, bounds.size.height - 2.0 * INSET),
    );
    let glass = NSGlassEffectView::initWithFrame(NSGlassEffectView::alloc(mtm), frame);
    glass.setCornerRadius(RADIUS);
    glass.setAutoresizingMask(NSAutoresizingMaskOptions::ViewHeightSizable | NSAutoresizingMaskOptions::ViewMaxXMargin);
    host.addSubview_positioned_relativeTo(&glass, NSWindowOrderingMode::Below, Some(webview));

    ACTIVE.store(true, Ordering::Relaxed);
}
