//! Frameless window dragging, Liquid Glass titlebar vibrancy, and keyboard shortcut preload.
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

#[cfg(not(target_os = "android"))]
const PRELOAD: &str = include_str!("window_drag.js");

/// Installs the drag-region and cross-platform keyboard shortcut preload.
pub(crate) fn apply(builder: WebViewBuilder<'_>) -> WebViewBuilder<'_> {
    #[cfg(not(target_os = "android"))]
    {
        builder.with_initialization_script(PRELOAD)
    }
    #[cfg(target_os = "android")]
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

/// Installs a translucent Liquid Glass (`NSVisualEffectView`) backdrop directly
/// inside the macOS `NSTitlebarView` and `contentView` so the title bar blurs
/// behind the window instead of showing a solid flat grey bar.
#[cfg(target_os = "macos")]
pub(crate) fn apply_liquid_glass(window: &Window) {
    let ns_window = window.ns_window();
    if !ns_window.is_null() {
        appkit::apply_liquid_glass(ns_window);
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

    unsafe fn view_bounds(view: Object) -> NSRect {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            let mut rect = NSRect::default();
            objc_msgSend_stret(&mut rect, view, selector(c"bounds"));
            rect
        }
        #[cfg(not(target_arch = "x86_64"))]
        unsafe {
            let get_rect: unsafe extern "C" fn(Object, Selector) -> NSRect =
                mem::transmute(objc_msgSend as *const ());
            get_rect(view, selector(c"bounds"))
        }
    }

    unsafe fn create_effect_view(bounds: NSRect, material: isize) -> Object {
        let send_obj: unsafe extern "C" fn(Object, Selector) -> Object =
            mem::transmute(objc_msgSend as *const ());
        let send_bool: unsafe extern "C" fn(Object, Selector, bool) =
            mem::transmute(objc_msgSend as *const ());
        let send_isize: unsafe extern "C" fn(Object, Selector, isize) =
            mem::transmute(objc_msgSend as *const ());
        let send_usize: unsafe extern "C" fn(Object, Selector, usize) =
            mem::transmute(objc_msgSend as *const ());

        let effect_class = objc_getClass(c"NSVisualEffectView".as_ptr());
        if effect_class.is_null() {
            return std::ptr::null_mut();
        }
        let alloc = send_obj(effect_class, selector(c"alloc"));
        if alloc.is_null() {
            return std::ptr::null_mut();
        }
        let init_frame: unsafe extern "C" fn(Object, Selector, NSRect) -> Object =
            mem::transmute(objc_msgSend as *const ());
        let effect_view = init_frame(alloc, selector(c"initWithFrame:"), bounds);
        if effect_view.is_null() {
            return std::ptr::null_mut();
        }

        // Material 13 (NSVisualEffectMaterialHUDWindow) / 10 (HeaderView) with
        // BehindWindow (0) and Active (1) gives a true frosted Liquid Glass blur.
        send_bool(effect_view, selector(c"setWantsLayer:"), true);
        send_isize(effect_view, selector(c"setMaterial:"), material);
        send_isize(effect_view, selector(c"setBlendingMode:"), 0);
        send_isize(effect_view, selector(c"setState:"), 1);
        // NSViewWidthSizable (2) | NSViewHeightSizable (16) = 18
        send_usize(effect_view, selector(c"setAutoresizingMask:"), 18);
        effect_view
    }

    pub(super) fn apply_liquid_glass(ns_window: Object) {
        unsafe {
            let send_bool: unsafe extern "C" fn(Object, Selector, bool) =
                mem::transmute(objc_msgSend as *const ());
            let send_obj: unsafe extern "C" fn(Object, Selector) -> Object =
                mem::transmute(objc_msgSend as *const ());
            let send_obj_arg: unsafe extern "C" fn(Object, Selector, Object) =
                mem::transmute(objc_msgSend as *const ());
            let send_usize_obj: unsafe extern "C" fn(Object, Selector, usize) -> Object =
                mem::transmute(objc_msgSend as *const ());
            let add_subview: unsafe extern "C" fn(Object, Selector, Object, isize, Object) =
                mem::transmute(objc_msgSend as *const ());

            send_bool(ns_window, selector(c"setOpaque:"), false);
            send_bool(ns_window, selector(c"setTitlebarAppearsTransparent:"), true);

            let ns_color_class = objc_getClass(c"NSColor".as_ptr());
            if !ns_color_class.is_null() {
                let clear_color = send_obj(ns_color_class, selector(c"clearColor"));
                if !clear_color.is_null() {
                    send_obj_arg(ns_window, selector(c"setBackgroundColor:"), clear_color);
                }
            }

            // 1. Attach Liquid Glass NSVisualEffectView directly inside NSTitlebarView
            //    (located via the traffic-light close button's superview) so the
            //    28pt native titlebar area renders as translucent frosted glass
            //    instead of a flat grey strip.
            let close_button = send_usize_obj(ns_window, selector(c"standardWindowButton:"), 0);
            if !close_button.is_null() {
                let titlebar_view = send_obj(close_button, selector(c"superview"));
                if !titlebar_view.is_null() {
                    let tb_bounds = view_bounds(titlebar_view);
                    // NSVisualEffectMaterialHUDWindow (13) produces a crisp, translucent
                    // Liquid Glass blur behind the title bar and traffic lights.
                    let tb_effect = create_effect_view(tb_bounds, 13);
                    if !tb_effect.is_null() {
                        add_subview(
                            titlebar_view,
                            selector(c"addSubview:positioned:relativeTo:"),
                            tb_effect,
                            -1,
                            close_button,
                        );
                    }
                }
            }

            // 2. Also install an UnderWindowBackground (21) NSVisualEffectView at the
            //    back of contentView so overlay/fullsize mode shares the same vibrancy.
            let content_view = send_obj(ns_window, selector(c"contentView"));
            if !content_view.is_null() {
                let cv_bounds = view_bounds(content_view);
                let cv_effect = create_effect_view(cv_bounds, 21);
                if !cv_effect.is_null() {
                    add_subview(
                        content_view,
                        selector(c"addSubview:positioned:relativeTo:"),
                        cv_effect,
                        -1,
                        std::ptr::null_mut(),
                    );
                }
            }
        }
    }
}
