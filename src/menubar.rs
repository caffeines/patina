//! Patina's status item in the macOS menu bar.
//!
//! A left click opens the popover. egui draws the popover in its own
//! borderless window (see `ui::popover`); this module only reports where it
//! should hang. A right click (or control-click) shows a small native menu
//! with Open Patina and Quit Patina.
//!
//! AppKit calls back on the main thread. Clicks are queued in [`EVENTS`] and
//! the app drains them each frame with [`take_events`].

use std::sync::{Mutex, OnceLock};

/// What the status item asked for.
#[derive(Clone, Copy, Debug)]
pub enum Event {
    /// Left click: open or close the popover.
    Toggle(Anchor),
    /// "Open Patina" in the right-click menu.
    Open,
    /// "Quit Patina" in the right-click menu.
    Quit,
    /// The app became active (for example, it was opened again from Finder).
    Activated,
}

/// Where the popover goes, for a popover of [`POPOVER_SIZE`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Anchor {
    /// Top-left corner in egui points (top-left origin, main screen).
    pub pos: [f32; 2],
    /// Top-left corner in AppKit screen coordinates (bottom-left origin).
    pub native: [f64; 2],
}

/// The popover's size, from the design.
pub const POPOVER_SIZE: [f32; 2] = [380.0, 540.0];
/// The popover window's title (it has no title bar; this finds it).
pub const POPOVER_TITLE: &str = "Patina menu";
/// Space between the menu bar and the popover.
const GAP: f64 = 6.0;

static EVENTS: Mutex<Vec<Event>> = Mutex::new(Vec::new());

/// Print what the status item and popover do when PATINA_DEBUG is set.
pub fn trace(what: &str) {
    if std::env::var_os("PATINA_DEBUG").is_some() {
        eprintln!("[patina] menu bar: {what}");
    }
}
static CTX: OnceLock<egui::Context> = OnceLock::new();

fn push(ev: Event) {
    EVENTS.lock().unwrap_or_else(|e| e.into_inner()).push(ev);
    if let Some(ctx) = CTX.get() {
        ctx.request_repaint_of(egui::ViewportId::ROOT);
    }
}

/// Clicks since the last call.
pub fn take_events() -> Vec<Event> {
    std::mem::take(&mut *EVENTS.lock().unwrap_or_else(|e| e.into_inner()))
}

/// The menu bar glyph: the Patina mark as a template image (alpha only), a
/// ring with its lower-right half filled. `n` is the size in pixels.
pub fn glyph_rgba(n: usize) -> Vec<u8> {
    let mut rgba = vec![0u8; n * n * 4];
    let c = n as f32 / 2.0;
    let r_out = n as f32 * 0.43;
    let r_in = r_out - n as f32 * 0.085;
    for y in 0..n {
        for x in 0..n {
            let mut hit = 0u32;
            for sy in 0..4 {
                for sx in 0..4 {
                    let px = x as f32 + (sx as f32 + 0.5) / 4.0 - c;
                    let py = y as f32 + (sy as f32 + 0.5) / 4.0 - c;
                    let d = (px * px + py * py).sqrt();
                    if d <= r_out && (d >= r_in || px + py > 0.0) {
                        hit += 1;
                    }
                }
            }
            rgba[(y * n + x) * 4 + 3] = (hit * 255 / 16) as u8;
        }
    }
    rgba
}

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::RefCell;

    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{define_class, msg_send, sel, AnyThread, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSEventMask, NSEventModifierFlags, NSEventType, NSImage, NSMenu,
        NSApplicationDidBecomeActiveNotification, NSMenuItem, NSPopUpMenuWindowLevel, NSScreen, NSSquareStatusItemLength, NSStatusBar, NSStatusItem,
        NSWindowCollectionBehavior,
    };
    use objc2_foundation::{NSData, NSNotificationCenter, NSObject, NSPoint, NSSize, NSString};

    use super::{push, Anchor, Event, GAP, POPOVER_SIZE, POPOVER_TITLE};

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and Target has no Drop.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "PatinaStatusTarget"]
        struct Target;

        impl Target {
            #[unsafe(method(click:))]
            fn click(&self, _sender: Option<&AnyObject>) {
                on_click(self.mtm());
            }

            #[unsafe(method(open:))]
            fn open(&self, _sender: Option<&AnyObject>) {
                push(Event::Open);
            }

            #[unsafe(method(quit:))]
            fn quit(&self, _sender: Option<&AnyObject>) {
                push(Event::Quit);
            }

            #[unsafe(method(activated:))]
            fn activated(&self, _note: Option<&AnyObject>) {
                push(Event::Activated);
            }
        }
    );

    struct Installed {
        item: Retained<NSStatusItem>,
        menu: Retained<NSMenu>,
        _target: Retained<Target>,
    }

    thread_local! {
        static ITEM: RefCell<Option<Installed>> = const { RefCell::new(None) };
    }

    fn on_click(mtm: MainThreadMarker) {
        super::trace("status item clicked");
        let app = NSApplication::sharedApplication(mtm);
        let secondary = app.currentEvent().is_some_and(|e| {
            e.r#type() == NSEventType::RightMouseUp || e.modifierFlags().contains(NSEventModifierFlags::Control)
        });
        ITEM.with_borrow(|it| {
            let Some(it) = it else { return };
            if secondary {
                #[allow(deprecated)]
                it.item.popUpStatusItemMenu(&it.menu);
                return;
            }
            if let Some(anchor) = anchor(mtm, &it.item) {
                push(Event::Toggle(anchor));
            }
        });
    }

    /// Hang the popover under the status item, centered, kept on its screen.
    fn anchor(mtm: MainThreadMarker, item: &NSStatusItem) -> Option<Anchor> {
        let win = item.button(mtm)?.window()?;
        let f = win.frame();
        let main_h = NSScreen::screens(mtm).firstObject().map_or(f.origin.y + f.size.height, |s| s.frame().size.height);
        let vis = win.screen().map_or(f, |s| s.visibleFrame());
        let w = POPOVER_SIZE[0] as f64;
        let mid = f.origin.x + f.size.width / 2.0;
        let x = (mid - w / 2.0).clamp(vis.origin.x + 8.0, (vis.origin.x + vis.size.width - w - 8.0).max(vis.origin.x + 8.0));
        let top = f.origin.y - GAP;
        Some(Anchor { pos: [x as f32, (main_h - top) as f32], native: [x, top] })
    }

    fn glyph(mtm: MainThreadMarker) -> Option<Retained<NSImage>> {
        let _ = mtm;
        let n = 36;
        let rgba = super::glyph_rgba(n);
        let mut png = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png, n as u32, n as u32);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().ok()?;
            w.write_image_data(&rgba).ok()?;
        }
        let data = NSData::with_bytes(&png);
        let img = NSImage::initWithData(NSImage::alloc(), &data)?;
        img.setSize(NSSize { width: 18.0, height: 18.0 });
        img.setTemplate(true);
        Some(img)
    }

    fn menu_item(mtm: MainThreadMarker, title: &str, action: objc2::runtime::Sel, key: &str, target: &Target) -> Retained<NSMenuItem> {
        // SAFETY: the action is one of Target's methods, and the target outlives the menu.
        unsafe {
            let it = NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                Some(action),
                &NSString::from_str(key),
            );
            it.setTarget(Some(target));
            it
        }
    }

    pub fn install() -> bool {
        let Some(mtm) = MainThreadMarker::new() else { return false };
        if ITEM.with_borrow(|it| it.is_some()) {
            return true;
        }
        let target: Retained<Target> = unsafe { msg_send![Target::alloc(mtm), init] };
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSSquareStatusItemLength);
        let Some(button) = item.button(mtm) else { return false };
        if let Some(img) = glyph(mtm) {
            button.setImage(Some(&img));
        }
        button.setToolTip(Some(&NSString::from_str("Patina")));
        // SAFETY: click: is Target's method, and ITEM keeps the target alive.
        unsafe {
            button.setTarget(Some(&target));
            button.setAction(Some(sel!(click:)));
        }
        button.sendActionOn(NSEventMask::LeftMouseUp | NSEventMask::RightMouseUp);

        let menu = NSMenu::new(mtm);
        menu.addItem(&menu_item(mtm, "Open Patina", sel!(open:), "", &target));
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        menu.addItem(&menu_item(mtm, "Quit Patina", sel!(quit:), "q", &target));

        // Opening Patina again while it lives in the menu bar brings the window back.
        // SAFETY: activated: is Target's method; the observer is removed in remove().
        unsafe {
            NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                &target,
                sel!(activated:),
                Some(NSApplicationDidBecomeActiveNotification),
                None,
            );
        }

        ITEM.set(Some(Installed { item, menu, _target: target }));
        true
    }

    pub fn remove() {
        if let Some(it) = ITEM.take() {
            // SAFETY: the target is a live object we registered in install().
            unsafe { NSNotificationCenter::defaultCenter().removeObserver(&it._target) };
            NSStatusBar::systemStatusBar().removeStatusItem(&it.item);
        }
    }

    pub fn set_running(on: bool) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        ITEM.with_borrow(|it| {
            if let Some(button) = it.as_ref().and_then(|it| it.item.button(mtm)) {
                if button.appearsDisabled() == on {
                    button.setAppearsDisabled(!on);
                }
            }
        });
    }

    /// Show or hide the Dock icon. Patina lives in the menu bar while its
    /// window is closed.
    pub fn set_dock(visible: bool) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let policy = if visible { NSApplicationActivationPolicy::Regular } else { NSApplicationActivationPolicy::Accessory };
        let app = NSApplication::sharedApplication(mtm);
        if app.activationPolicy() != policy {
            app.setActivationPolicy(policy);
        }
    }

    pub fn activate() {
        let Some(mtm) = MainThreadMarker::new() else { return };
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    }

    /// Give the keyboard back to the app that had it.
    pub fn hide_app() {
        let Some(mtm) = MainThreadMarker::new() else { return };
        NSApplication::sharedApplication(mtm).hide(None);
    }

    /// Native touches egui can't express: the popover's shadow, its place on
    /// every Space (including over full-screen apps), its level above the
    /// menu bar, and its exact position. With `anchor` (the first frame) it
    /// also shows the popover, which egui made hidden: on screen first, on
    /// the current Space, and only then is Patina activated. Activating an
    /// app with no window on the current Space makes macOS switch to one
    /// that has one, which took you out of a full-screen app.
    pub fn style_popover(anchor: Option<Anchor>) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let app = NSApplication::sharedApplication(mtm);
        for w in app.windows().iter() {
            if w.title().to_string() != POPOVER_TITLE {
                continue;
            }
            w.setHasShadow(true);
            w.invalidateShadow();
            w.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::FullScreenAuxiliary
                    | NSWindowCollectionBehavior::Transient
                    | NSWindowCollectionBehavior::IgnoresCycle,
            );
            w.setLevel(NSPopUpMenuWindowLevel);
            if let Some(a) = anchor {
                super::trace(&format!("showing the popover at {:?} on screen {:?}", a.native, w.screen().map(|s| s.frame())));
                w.setFrameTopLeftPoint(NSPoint { x: a.native[0], y: a.native[1] });
                w.orderFrontRegardless();
                #[allow(deprecated)]
                app.activateIgnoringOtherApps(true);
                w.makeKeyAndOrderFront(None);
                super::trace(&format!(
                    "popover visible: {}, on active Space: {}, behavior: {:?}, level: {}, policy: {:?}",
                    w.isVisible(),
                    w.isOnActiveSpace(),
                    w.collectionBehavior(),
                    w.level(),
                    app.activationPolicy()
                ));
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod mac {
    use super::Anchor;
    pub fn install() -> bool {
        false
    }
    pub fn remove() {}
    pub fn set_running(_on: bool) {}
    pub fn set_dock(_visible: bool) {}
    pub fn activate() {}
    pub fn hide_app() {}
    pub fn style_popover(_anchor: Option<Anchor>) {}
}

pub use mac::{activate, hide_app, remove, set_dock, set_running, style_popover};

/// Put the status item in the menu bar. Returns false where there is no
/// menu bar to put it in.
pub fn install(ctx: &egui::Context) -> bool {
    let _ = CTX.set(ctx.clone());
    mac::install()
}

#[cfg(test)]
mod tests {
    #[test]
    fn glyph_is_a_ring_with_a_filled_half() {
        let n = 36;
        let px = super::glyph_rgba(n);
        let a = |x: usize, y: usize| px[(y * n + x) * 4 + 3];
        assert_eq!(a(0, 0), 0, "corners are clear");
        assert_eq!(a(13, 13), 0, "the upper-left half is open");
        assert_eq!(a(23, 23), 255, "the lower-right half is filled");
        assert_eq!(a(18, 3), 255, "the ring is drawn");
    }
}
