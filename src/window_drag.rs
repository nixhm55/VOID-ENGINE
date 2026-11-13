//! Frameless window dragging.
//!
//! A frameless macOS window keeps its traffic lights and its content view, but
//! the webview covers the only area `AppKit` would otherwise treat as a title
//! bar, so a press inside the page never becomes a window drag. The preload
//! marks the page header as a drag region and reports mouse-down over it; this
//! module turns that report into a real window drag.
//!
//! `wry`/`tao` cannot do it on their own here: `Window::drag_window` on macOS
//! forwards `NSApp.currentEvent` to `performWindowDragWithEvent:`, and that
//! event is gone by the time an asynchronous IPC message is handled. The drag
//! is therefore driven from the event loop with `AppKit` mouse queries, which
//! keeps it smooth and independent of the IPC round trip.

#[cfg(target_os = "macos")]
use std::time::Duration;
#[cfg(target_os = "macos")]
use tao::{platform::macos::WindowExtMacOS, window::Window};
use wry::WebViewBuilder;

/// How often the window is repositioned while a drag is in flight (~120 Hz).
#[cfg(target_os = "macos")]
pub(crate) const TICK: Duration = Duration::from_millis(8);

#[cfg(target_os = "macos")]
const PRELOAD: &str = include_str!("window_drag.js");

/// Installs the drag-region preload. Only the macOS build needs it: Chromium
/// webviews already honour the injected `app-region` CSS on their own.
pub(crate) fn apply(builder: WebViewBuilder<'_>) -> WebViewBuilder<'_> {
    #[cfg(target_os = "macos")]
    {
        builder.with_initialization_script(PRELOAD)
    }
    #[cfg(not(target_os = "macos"))]
    {
        builder
    }
}

/// Lets `AppKit` move the window. `tao` 0.37 exposes no `Window::set_movable`,
/// so the `NSWindow` property is applied directly at startup.
#[cfg(target_os = "macos")]
pub(crate) fn allow_movement(window: &Window) {
    let ns_window = window.ns_window();
    if !ns_window.is_null() {
        appkit::set_movable(ns_window);
    }
}

/// One in-flight drag, anchored where the press started.
#[cfg(target_os = "macos")]
pub(crate) struct DragSession {
    ns_window: appkit::Object,
    start_mouse: appkit::NSPoint,
    start_origin: appkit::NSPoint,
    /// Last origin pushed to `AppKit`, so a still pointer costs no window calls.
    applied: appkit::NSPoint,
}

#[cfg(target_os = "macos")]
impl DragSession {
    /// Starts a drag only while the primary button is actually held, so a page
    /// cannot move the window without a real press.
    pub(crate) fn begin(window: &Window) -> Option<Self> {
        if !appkit::primary_button_down() {
            return None;
        }
        let ns_window = window.ns_window();
        if ns_window.is_null() {
            return None;
        }
        let start_origin = appkit::frame_origin(ns_window);
        Some(Self {
            ns_window,
            start_mouse: appkit::mouse_location(),
            start_origin,
            applied: start_origin,
        })
    }

    /// Moves the window by the mouse delta since the press. Both the pointer
    /// and the window frame are in screen points, so no scaling is involved.
    pub(crate) fn update(&mut self) {
        let mouse = appkit::mouse_location();
        let origin = appkit::NSPoint {
            x: self.start_origin.x + (mouse.x - self.start_mouse.x),
            y: self.start_origin.y + (mouse.y - self.start_mouse.y),
        };
        // Exact comparison is intentional: the origin is recomputed from the
        // same mouse reading, so an untouched pointer yields bit-identical values.
        if origin.x.to_bits() == self.applied.x.to_bits()
            && origin.y.to_bits() == self.applied.y.to_bits()
        {
            return;
        }
        appkit::set_frame_origin(self.ns_window, origin);
        self.applied = origin;
    }
}

/// Reports whether the primary button that started a drag has been released.
#[cfg(target_os = "macos")]
pub(crate) fn primary_button_released() -> bool {
    !appkit::primary_button_down()
}

/// Minimal `AppKit` access. `tao` only exposes setters for some window
/// properties, so the missing ones go through `objc_msgSend` directly, the
/// same way the location bridge in `native_bridge.rs` calls `CoreLocation`.
#[cfg(target_os = "macos")]
mod appkit {
    use std::{
        ffi::{c_char, c_void, CStr},
        mem,
    };

    pub(super) type Object = *mut c_void;
    type Selector = *mut c_void;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub(super) struct NSPoint {
        pub x: f64,
        pub y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    #[allow(dead_code)]
    struct NSSize {
        width: f64,
        height: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    #[allow(dead_code)]
    struct NSRect {
        origin: NSPoint,
        size: NSSize,
    }

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Object;
        fn sel_registerName(name: *const c_char) -> Selector;
        fn objc_msgSend();
    }

    // Only Intel macOS returns the 32-byte NSRect through a struct-return
    // helper; arm64 returns it in registers like any other call.
    #[cfg(target_arch = "x86_64")]
    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_msgSend_stret(rect: *mut NSRect, receiver: Object, selector: Selector);
    }

    fn selector(name: &CStr) -> Selector {
        unsafe { sel_registerName(name.as_ptr()) }
    }

    fn event_class() -> Object {
        unsafe { objc_getClass(c"NSEvent".as_ptr()) }
    }

    /// `+[NSEvent mouseLocation]`: screen coordinates in points, origin at the
    /// bottom-left of the main display.
    pub(super) fn mouse_location() -> NSPoint {
        unsafe {
            let class = event_class();
            if class.is_null() {
                return NSPoint::default();
            }
            let send: unsafe extern "C" fn(Object, Selector) -> NSPoint =
                mem::transmute(objc_msgSend as *const ());
            let point = send(class, selector(c"mouseLocation"));
            if point.x.is_finite() && point.y.is_finite() {
                point
            } else {
                NSPoint::default()
            }
        }
    }

    /// `+[NSEvent pressedMouseButtons]` bit 0 is the primary button.
    pub(super) fn primary_button_down() -> bool {
        unsafe {
            let class = event_class();
            if class.is_null() {
                return false;
            }
            let send: unsafe extern "C" fn(Object, Selector) -> usize =
                mem::transmute(objc_msgSend as *const ());
            send(class, selector(c"pressedMouseButtons")) & 1 == 1
        }
    }

    pub(super) fn frame_origin(ns_window: Object) -> NSPoint {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            let mut frame = NSRect::default();
            objc_msgSend_stret(&mut frame, ns_window, selector(c"frame"));
            frame.origin
        }
        #[cfg(not(target_arch = "x86_64"))]
        unsafe {
            let send: unsafe extern "C" fn(Object, Selector) -> NSRect =
                mem::transmute(objc_msgSend as *const ());
            send(ns_window, selector(c"frame")).origin
        }
    }

    pub(super) fn set_frame_origin(ns_window: Object, origin: NSPoint) {
        unsafe {
            let send: unsafe extern "C" fn(Object, Selector, NSPoint) =
                mem::transmute(objc_msgSend as *const ());
            send(ns_window, selector(c"setFrameOrigin:"), origin);
        }
    }

    pub(super) fn set_movable(ns_window: Object) {
        unsafe {
            let send: unsafe extern "C" fn(Object, Selector, bool) =
                mem::transmute(objc_msgSend as *const ());
            send(ns_window, selector(c"setMovable:"), true);
        }
    }
}
