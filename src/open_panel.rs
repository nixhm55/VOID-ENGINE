//! Powerbox folder selection for the sandboxed macOS build.
//!
//! Under App Sandbox a process can only see the filesystem through the system
//! Powerbox. `NSOpenPanel` is that door: the panel runs out of process, the
//! person picks a folder, and macOS hands this process a sandbox extension for
//! exactly that folder. Nothing else becomes readable.

use std::{
    ffi::{c_char, c_void, CStr, CString},
    mem, ptr,
};

type Object = *mut c_void;
type Selector = *mut c_void;

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Object;
    fn sel_registerName(name: *const c_char) -> Selector;
    fn objc_msgSend();
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

const NS_MODAL_RESPONSE_OK: isize = 1;

pub(crate) fn pick_folder(message: &str) -> Option<String> {
    unsafe {
        let class = objc_getClass(c"NSOpenPanel".as_ptr());
        if class.is_null() {
            return None;
        }
        let panel = send_object(class, selector(c"openPanel"));
        if panel.is_null() {
            return None;
        }

        // A folder picker, single selection, and the system may offer the
        // "New Folder" button. Files are never returned by this panel.
        send_bool_arg(panel, selector(c"setCanChooseFiles:"), 0);
        send_bool_arg(panel, selector(c"setCanChooseDirectories:"), 1);
        send_bool_arg(panel, selector(c"setAllowsMultipleSelection:"), 0);
        send_bool_arg(panel, selector(c"setCanCreateDirectories:"), 1);
        send_bool_arg(panel, selector(c"setResolvesAliases:"), 1);
        send_bool_arg(panel, selector(c"setShowsHiddenFiles:"), 0);

        let prompt = nsstring("Share");
        if !prompt.is_null() {
            send_object_arg(panel, selector(c"setPrompt:"), prompt);
            send_void(prompt, selector(c"release"));
        }
        let text = nsstring(message);
        if !text.is_null() {
            send_object_arg(panel, selector(c"setMessage:"), text);
            send_void(text, selector(c"release"));
        }

        if send_isize(panel, selector(c"runModal")) != NS_MODAL_RESPONSE_OK {
            return None;
        }

        let url = send_object(panel, selector(c"URL"));
        if url.is_null() {
            return None;
        }

        // Keep the Powerbox extension active for the rest of the session. On a
        // non-sandboxed development build this simply returns NO.
        send_bool_arg(url, selector(c"startAccessingSecurityScopedResource"), 1);

        let path = send_object(url, selector(c"path"));
        let value = nsstring_to_string(path);
        if value.is_empty() {
            None
        } else {
            Some(value)
        }
    }
}

unsafe fn nsstring(value: &str) -> Object {
    let class = objc_getClass(c"NSString".as_ptr());
    if class.is_null() {
        return ptr::null_mut();
    }
    let Ok(text) = CString::new(value) else {
        return ptr::null_mut();
    };
    let allocated = send_object(class, selector(c"alloc"));
    if allocated.is_null() {
        return ptr::null_mut();
    }
    send_object_arg(
        allocated,
        selector(c"initWithUTF8String:"),
        text.as_ptr() as Object,
    );
    allocated
}

unsafe fn nsstring_to_string(value: Object) -> String {
    if value.is_null() {
        return String::new();
    }
    let utf8 = send_object(value, selector(c"UTF8String"));
    if utf8.is_null() {
        return String::new();
    }
    CStr::from_ptr(utf8 as *const c_char)
        .to_string_lossy()
        .into_owned()
}

unsafe fn selector(name: &CStr) -> Selector {
    sel_registerName(name.as_ptr())
}

unsafe fn send_object(receiver: Object, selector: Selector) -> Object {
    let function: unsafe extern "C" fn(Object, Selector) -> Object =
        mem::transmute(objc_msgSend as *const ());
    function(receiver, selector)
}

unsafe fn send_object_arg(receiver: Object, selector: Selector, value: Object) {
    let function: unsafe extern "C" fn(Object, Selector, Object) =
        mem::transmute(objc_msgSend as *const ());
    function(receiver, selector, value);
}

unsafe fn send_bool_arg(receiver: Object, selector: Selector, value: i8) {
    let function: unsafe extern "C" fn(Object, Selector, i8) =
        mem::transmute(objc_msgSend as *const ());
    function(receiver, selector, value);
}

unsafe fn send_void(receiver: Object, selector: Selector) {
    let function: unsafe extern "C" fn(Object, Selector) =
        mem::transmute(objc_msgSend as *const ());
    function(receiver, selector);
}

unsafe fn send_isize(receiver: Object, selector: Selector) -> isize {
    let function: unsafe extern "C" fn(Object, Selector) -> isize =
        mem::transmute(objc_msgSend as *const ());
    function(receiver, selector)
}
