//! Layer 1 of the VOID security bridge.
//!
//! LocalAuthentication proves a person is present before a website is given a
//! folder grant. `LAPolicyDeviceOwnerAuthentication` (2) accepts Touch ID and
//! falls back to the macOS account password, so the prompt still works on Macs
//! without a fingerprint sensor.

use std::{
    ffi::{c_char, c_void, CStr, CString},
    mem, ptr,
    sync::{mpsc, Mutex, OnceLock},
    time::Duration,
};

type Object = *mut c_void;
type Selector = *mut c_void;

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Object;
    fn sel_registerName(name: *const c_char) -> Selector;
    fn objc_msgSend();
}

#[link(name = "System")]
unsafe extern "C" {
    fn dlopen(filename: *const c_char, mode: i32) -> *mut c_void;
    static _NSConcreteGlobalBlock: c_void;
}

const RTLD_NOW: i32 = 2;
const RTLD_GLOBAL: i32 = 8;
const LAPOLICY_DEVICE_OWNER_AUTHENTICATION: isize = 2;
const AUTH_TIMEOUT: Duration = Duration::from_secs(120);

// Blocks runtime layout. A global block needs no captured state, so the reply
// can hand its result back through a static slot.
#[repr(C)]
struct BlockDescriptor {
    reserved: usize,
    size: usize,
    copy: *const c_void,
    dispose: *const c_void,
    signature: *const c_char,
}

#[repr(C)]
struct Block {
    isa: *const c_void,
    flags: i32,
    reserved: i32,
    invoke: *const c_void,
    descriptor: *const BlockDescriptor,
}

const BLOCK_IS_GLOBAL: i32 = 1 << 28;
const BLOCK_HAS_SIGNATURE: i32 = 1 << 30;

static AUTH_GATE: Mutex<()> = Mutex::new(());
static AUTH_SLOT: Mutex<Option<mpsc::Sender<Result<(), String>>>> = Mutex::new(None);
static LA_FRAMEWORK: OnceLock<bool> = OnceLock::new();
// The prompt is asynchronous, so the reason string has to outlive the call that
// starts it. Reasons are a small fixed set, so one immortal copy of each is
// cheaper and safer than trying to free it when the reply block runs.
static AUTH_REASONS: Mutex<Vec<(String, usize)>> = Mutex::new(Vec::new());

pub(super) fn authenticate(reason: &str) -> Result<(), String> {
    // Overlapping prompts would race for the single reply slot.
    let _gate = AUTH_GATE
        .lock()
        .map_err(|_| "The macOS authentication bridge is unavailable.".to_string())?;

    if !load_framework() {
        return Err("Could not load the macOS LocalAuthentication framework.".into());
    }

    unsafe {
        let class = objc_getClass(c"LAContext".as_ptr());
        if class.is_null() {
            return Err("LocalAuthentication is unavailable on this Mac.".into());
        }
        let context = send_object(send_object(class, selector(c"alloc")), selector(c"init"));
        if context.is_null() {
            return Err("Could not create a macOS authentication context.".into());
        }

        let mut error: Object = ptr::null_mut();
        let can_evaluate: unsafe extern "C" fn(Object, Selector, isize, *mut Object) -> i8 =
            mem::transmute(objc_msgSend as *const ());
        let available = can_evaluate(
            context,
            selector(c"canEvaluatePolicy:error:"),
            LAPOLICY_DEVICE_OWNER_AUTHENTICATION,
            &mut error,
        );
        if available == 0 {
            let detail = describe_error(error);
            send_void(context, selector(c"release"));
            return Err(if detail.is_empty() {
                "This Mac cannot confirm your identity right now.".to_string()
            } else {
                detail
            });
        }

        let (sender, receiver) = mpsc::channel();
        *AUTH_SLOT
            .lock()
            .map_err(|_| "The macOS authentication bridge is unavailable.".to_string())? =
            Some(sender);

        let reason_string = retained_reason(reason);
        let evaluate: unsafe extern "C" fn(Object, Selector, isize, Object, Object) =
            mem::transmute(objc_msgSend as *const ());
        evaluate(
            context,
            selector(c"evaluatePolicy:localizedReason:reply:"),
            LAPOLICY_DEVICE_OWNER_AUTHENTICATION,
            reason_string,
            auth_block() as Object,
        );

        let outcome = receiver.recv_timeout(AUTH_TIMEOUT);
        if let Ok(mut slot) = AUTH_SLOT.lock() {
            *slot = None;
        }

        let result = match outcome {
            Ok(Ok(())) => Ok(()),
            Ok(Err(message)) if message.is_empty() => Err("Authentication failed.".to_string()),
            Ok(Err(message)) => Err(message),
            Err(_) => {
                send_void(context, selector(c"invalidate"));
                Err("Timed out waiting for macOS authentication.".to_string())
            }
        };
        send_void(context, selector(c"release"));
        result
    }
}

fn load_framework() -> bool {
    *LA_FRAMEWORK.get_or_init(|| unsafe {
        !dlopen(
            c"/System/Library/Frameworks/LocalAuthentication.framework/LocalAuthentication"
                .as_ptr(),
            RTLD_NOW | RTLD_GLOBAL,
        )
        .is_null()
    })
}

fn auth_block() -> *const c_void {
    static BLOCK: OnceLock<usize> = OnceLock::new();
    let address = *BLOCK.get_or_init(|| {
        let descriptor: &'static mut BlockDescriptor = Box::leak(Box::new(BlockDescriptor {
            reserved: 0,
            size: mem::size_of::<Block>(),
            copy: ptr::null(),
            dispose: ptr::null(),
            // void (^)(BOOL, NSError *)
            signature: c"v24@?0B8@16".as_ptr(),
        }));
        let block: &'static mut Block = Box::leak(Box::new(Block {
            isa: ptr::addr_of!(_NSConcreteGlobalBlock) as *const c_void,
            flags: BLOCK_IS_GLOBAL | BLOCK_HAS_SIGNATURE,
            reserved: 0,
            invoke: auth_reply as *const () as *const c_void,
            descriptor: descriptor as *mut BlockDescriptor as *const BlockDescriptor,
        }));
        block as *mut Block as usize
    });
    address as *const c_void
}

unsafe extern "C" fn auth_reply(_block: *const c_void, success: u8, error: Object) {
    let result = if success != 0 {
        Ok(())
    } else {
        Err(describe_error(error))
    };
    if let Ok(slot) = AUTH_SLOT.lock() {
        if let Some(sender) = slot.as_ref() {
            let _ = sender.send(result);
        }
    }
}

unsafe fn describe_error(error: Object) -> String {
    if error.is_null() {
        return String::new();
    }
    let description = send_object(error, selector(c"localizedDescription"));
    let text = nsstring_to_string(description);
    if !text.is_empty() {
        return text;
    }

    match send_isize(error, selector(c"code")) {
        -1 => "Authentication failed.".to_string(),
        -2 => "Authentication was cancelled.".to_string(),
        -3 => "The password fallback was requested.".to_string(),
        -4 => "The system cancelled authentication.".to_string(),
        -5 => "This Mac has no password set, so identity cannot be confirmed.".to_string(),
        -6 => "Biometric authentication is not available on this Mac.".to_string(),
        -7 => "No fingerprint is enrolled on this Mac.".to_string(),
        -8 => "Touch ID is locked. Unlock the Mac and try again.".to_string(),
        -9 => "The app cancelled authentication.".to_string(),
        -1004 => "Authentication needs a visible app window.".to_string(),
        _ => "macOS could not confirm your identity.".to_string(),
    }
}

unsafe fn retained_reason(reason: &str) -> Object {
    if let Ok(mut cache) = AUTH_REASONS.lock() {
        if let Some((_, pointer)) = cache.iter().find(|(text, _)| text == reason) {
            return *pointer as Object;
        }
        let string = nsstring(reason);
        if string.is_null() {
            return string;
        }
        cache.push((reason.to_string(), string as usize));
        return string;
    }

    // A poisoned cache must not block authentication; the string simply leaks.
    nsstring(reason)
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
