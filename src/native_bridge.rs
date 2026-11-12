use serde::Deserialize;
use tao::event_loop::EventLoopProxy;
use wry::{http::Request, WebViewBuilder};

#[cfg(all(feature = "bluetooth", not(target_os = "android")))]
mod desktop_bluetooth;

#[derive(Debug)]
pub(crate) enum UserEvent {
    Ipc {
        uri: String,
        body: String,
    },
    #[cfg(all(feature = "bluetooth", not(target_os = "android")))]
    Script {
        origin: String,
        source: String,
    },
    #[cfg(all(feature = "bluetooth", not(target_os = "android")))]
    Prompt {
        title: String,
        description: String,
        buttons: [String; 3],
        response: std::sync::mpsc::Sender<String>,
    },
}

#[derive(Deserialize)]
struct IpcCall {
    id: String,
    method: String,
    #[cfg(all(feature = "bluetooth", not(target_os = "android")))]
    #[serde(default)]
    args: serde_json::Value,
}

#[cfg(all(target_os = "macos", feature = "geolocation"))]
const GEOLOCATION_SHIM: &str = r#"(() => {
  if (!window.ipc || typeof window.ipc.postMessage !== 'function') return;
  let nextId = 1;
  const pending = new Map();
  const watches = new Map();
  let nextWatch = 1;
  function call(method) {
    return new Promise((resolve, reject) => {
      const id = String(nextId++);
      pending.set(id, { resolve, reject });
      window.ipc.postMessage(JSON.stringify({ id, method }));
    });
  }
  window.__voidNativeReply = (message) => {
    const task = pending.get(String(message.id));
    if (!task) return;
    pending.delete(String(message.id));
    if (message.error) task.reject(message.error);
    else task.resolve(message.value);
  };
  const error = (reason, callback) => {
    if (typeof callback === 'function') callback({ code: 1, message: reason || 'Location is unavailable.' });
  };
  const geo = {
    getCurrentPosition(success, failure) {
      call('geolocation.current').then((position) => {
        if (typeof success === 'function') success(position);
      }).catch((reason) => error(reason, failure));
    },
    watchPosition(success, failure, options = {}) {
      const id = nextWatch++;
      let active = true;
      const read = () => {
        if (!active) return;
        call('geolocation.current').then((position) => {
          if (active && typeof success === 'function') success(position);
        }).catch((reason) => error(reason, failure));
      };
      read();
      const timer = setInterval(read, Math.max(10000, Number(options.maximumAge) || 30000));
      watches.set(id, () => { active = false; clearInterval(timer); });
      return id;
    },
    clearWatch(id) {
      const stop = watches.get(id);
      if (stop) stop();
      watches.delete(id);
    }
  };
  try {
    Object.defineProperty(navigator, 'geolocation', { configurable: true, value: geo });
  } catch { navigator.geolocation = geo; }
})();"#;

pub(crate) fn configure(
    builder: WebViewBuilder,
    allowed_notification_host: Option<String>,
    proxy: EventLoopProxy<UserEvent>,
) -> WebViewBuilder {
    #[cfg(target_os = "macos")]
    let builder = crate::native_notifications::install(builder, allowed_notification_host.clone());
    #[cfg(not(target_os = "macos"))]
    let builder = builder;
    #[cfg(all(target_os = "macos", feature = "geolocation"))]
    let builder = builder.with_initialization_script(GEOLOCATION_SHIM);
    #[cfg(not(all(target_os = "macos", feature = "geolocation")))]
    let builder = builder;
    #[cfg(all(feature = "bluetooth", not(target_os = "android")))]
    let builder = builder.with_initialization_script(include_str!("android/bluetooth-shim.js"));
    #[cfg(not(all(feature = "bluetooth", not(target_os = "android"))))]
    let builder = builder;

    builder.with_ipc_handler(move |request: Request<String>| {
        #[cfg(target_os = "macos")]
        if crate::native_notifications::handle(&request, allowed_notification_host.as_deref()) {
            return;
        }
        #[cfg(not(target_os = "macos"))]
        let _ = &allowed_notification_host;
        let _ = proxy.send_event(UserEvent::Ipc {
            uri: request.uri().to_string(),
            body: request.body().clone(),
        });
    })
}

pub(crate) fn dispatch(uri: &str, body: &str, _proxy: EventLoopProxy<UserEvent>) -> Option<String> {
    #[cfg(all(feature = "bluetooth", not(target_os = "android")))]
    {
        let call = serde_json::from_str::<IpcCall>(body).ok()?;
        let origin = secure_origin(uri);

        if call.method.starts_with("bluetooth.") {
            let Some(origin) = origin else {
                return Some(reply_script(
                    "__voidBluetoothReply",
                    &call.id,
                    Err((
                        "Bluetooth is available only to secure website origins.".into(),
                        "SecurityError",
                    )),
                ));
            };
            let id = call.id;
            let method = call.method;
            std::thread::Builder::new()
                .name("void-bluetooth-ipc".into())
                .spawn(move || {
                    let result =
                        desktop_bluetooth::handle(&origin, &method, &call.args, _proxy.clone());
                    let script = reply_script("__voidBluetoothReply", &id, result);
                    let _ = _proxy.send_event(UserEvent::Script {
                        origin,
                        source: script,
                    });
                })
                .ok()?;
            return None;
        }
    }

    process_call(uri, body)
}

fn process_call(uri: &str, body: &str) -> Option<String> {
    let call = serde_json::from_str::<IpcCall>(body).ok()?;
    let parsed_uri = uri.parse::<wry::http::Uri>().ok();
    let secure_origin = parsed_uri
        .as_ref()
        .is_some_and(|parsed| secure_origin_from_uri(parsed).is_some());
    let result: Result<serde_json::Value, String> = if !secure_origin {
        Err("Native features are available only to secure website origins.".to_string())
    } else {
        match call.method.as_str() {
            #[cfg(all(target_os = "macos", feature = "geolocation"))]
            "geolocation.current" => {
                macos_location::current_location().map(|(latitude, longitude, accuracy)| {
                    serde_json::json!({
                        "coords": {
                            "latitude": latitude,
                            "longitude": longitude,
                            "accuracy": accuracy,
                            "altitude": null,
                            "altitudeAccuracy": null,
                            "heading": null,
                            "speed": null
                        },
                        "timestamp": now_millis()
                    })
                })
            }
            _ => Err("This native request is not supported by this build.".to_string()),
        }
    };
    let message = match result {
        Ok(value) => serde_json::json!({ "id": call.id, "value": value }),
        Err(error) => serde_json::json!({ "id": call.id, "error": error }),
    };
    let encoded = serde_json::to_string(&message).ok()?;
    Some(format!(
        "if (typeof window.__voidNativeReply === 'function') window.__voidNativeReply({encoded});"
    ))
}

#[cfg(all(feature = "bluetooth", not(target_os = "android")))]
pub(crate) fn secure_origin(uri: &str) -> Option<String> {
    uri.parse::<wry::http::Uri>()
        .ok()
        .and_then(|parsed| secure_origin_from_uri(&parsed))
}

fn secure_origin_from_uri(uri: &wry::http::Uri) -> Option<String> {
    let scheme = uri.scheme_str()?;
    let host = uri.host()?;
    let allowed = match (scheme, host) {
        ("https", _) => true,
        ("http", "localhost" | "127.0.0.1" | "[::1]") => true,
        ("void", "localhost") => true,
        _ => false,
    };
    if !allowed {
        return None;
    }
    let default_port = if scheme == "https" { 443 } else { 80 };
    let port = uri
        .port_u16()
        .filter(|port| *port != default_port)
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    Some(format!(
        "{}://{}{}",
        scheme.to_ascii_lowercase(),
        host.to_ascii_lowercase(),
        port
    ))
}

#[cfg(all(feature = "bluetooth", not(target_os = "android")))]
fn reply_script(
    callback: &str,
    id: &str,
    result: Result<serde_json::Value, (String, &'static str)>,
) -> String {
    let message = match result {
        Ok(value) => serde_json::json!({ "id": id, "value": value }),
        Err((error, name)) => serde_json::json!({ "id": id, "error": error, "name": name }),
    };
    let encoded = serde_json::to_string(&message).unwrap_or_else(|_| "{}".to_string());
    format!("if (typeof window.{callback} === 'function') window.{callback}({encoded});")
}

#[cfg(all(target_os = "macos", feature = "geolocation"))]
fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(all(target_os = "macos", feature = "geolocation"))]
mod macos_location {
    use std::{
        ffi::{c_char, c_void},
        mem,
        sync::{mpsc, Mutex, OnceLock},
        time::{Duration, Instant},
    };

    #[repr(C)]
    struct Coordinate {
        latitude: f64,
        longitude: f64,
    }

    type Object = *mut c_void;
    type Selector = *mut c_void;
    type Imp = unsafe extern "C" fn();

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Object;
        fn objc_allocateClassPair(
            superclass: Object,
            name: *const c_char,
            extra_bytes: usize,
        ) -> Object;
        fn objc_registerClassPair(class: Object);
        fn class_addMethod(
            class: Object,
            selector: Selector,
            implementation: Imp,
            types: *const c_char,
        ) -> bool;
        fn sel_registerName(name: *const c_char) -> Selector;
        fn objc_msgSend();
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFRunLoopDefaultMode: Object;
        fn CFRunLoopRunInMode(mode: Object, seconds: f64, return_after_source_handled: bool)
            -> i32;
    }

    static DELEGATE_CLASS: OnceLock<usize> = OnceLock::new();
    static RESULT_SENDER: Mutex<Option<mpsc::Sender<Result<(f64, f64, f64), String>>>> =
        Mutex::new(None);

    pub(super) fn current_location() -> Result<(f64, f64, f64), String> {
        let (sender, receiver) = mpsc::channel();
        {
            let mut slot = RESULT_SENDER
                .lock()
                .map_err(|_| "Location bridge is unavailable".to_string())?;
            *slot = Some(sender);
        }

        unsafe {
            let manager_class = objc_getClass(c"CLLocationManager".as_ptr());
            if manager_class.is_null() {
                clear_sender();
                return Err("CoreLocation is unavailable on this Mac.".into());
            }
            let alloc = selector(c"alloc");
            let init = selector(c"init");
            let manager = send_object(send_object(manager_class, alloc), init);
            if manager.is_null() {
                clear_sender();
                return Err("Could not create a CoreLocation manager.".into());
            }
            let delegate_class = delegate_class()?;
            let delegate = send_object(send_object(delegate_class, alloc), init);
            if delegate.is_null() {
                clear_sender();
                return Err("Could not create the CoreLocation delegate.".into());
            }
            send_object_arg(manager, selector(c"setDelegate:"), delegate);
            send_void(manager, selector(c"requestWhenInUseAuthorization"));
            send_void(manager, selector(c"requestLocation"));

            let deadline = Instant::now() + Duration::from_secs(35);
            let mut answer = None;
            while Instant::now() < deadline {
                if let Ok(value) = receiver.try_recv() {
                    answer = Some(value);
                    break;
                }
                CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.1, true);
            }
            clear_sender();
            answer.unwrap_or_else(|| {
                Err("Timed out waiting for macOS location permission or a location fix.".into())
            })
        }
    }

    fn clear_sender() {
        if let Ok(mut slot) = RESULT_SENDER.lock() {
            *slot = None;
        }
    }

    unsafe fn delegate_class() -> Result<Object, String> {
        let address = *DELEGATE_CLASS.get_or_init(|| unsafe {
            let class_name = c"VoidEngineLocationDelegate";
            let class =
                objc_allocateClassPair(objc_getClass(c"NSObject".as_ptr()), class_name.as_ptr(), 0);
            if class.is_null() {
                return 0;
            }
            let update = selector(c"locationManager:didUpdateLocations:");
            let failure = selector(c"locationManager:didFailWithError:");
            let authorization = selector(c"locationManagerDidChangeAuthorization:");
            let update_types = c"v@:@@";
            let object_types = c"v@:@";
            let update_imp: Imp = mem::transmute(location_updated as *const ());
            let failure_imp: Imp = mem::transmute(location_failed as *const ());
            let authorization_imp: Imp = mem::transmute(authorization_changed as *const ());
            if !class_addMethod(class, update, update_imp, update_types.as_ptr())
                || !class_addMethod(class, failure, failure_imp, object_types.as_ptr())
                || !class_addMethod(
                    class,
                    authorization,
                    authorization_imp,
                    object_types.as_ptr(),
                )
            {
                return 0;
            }
            objc_registerClassPair(class);
            class as usize
        });
        if address == 0 {
            Err("Could not register the CoreLocation delegate.".into())
        } else {
            Ok(address as Object)
        }
    }

    unsafe extern "C" fn authorization_changed(_this: Object, _cmd: Selector, manager: Object) {
        let status = send_isize(manager, selector(c"authorizationStatus"));
        match status {
            3 | 4 => send_void(manager, selector(c"requestLocation")),
            1 | 2 => send_error("Location access was denied in macOS Privacy & Security settings."),
            _ => {}
        }
    }

    unsafe extern "C" fn location_updated(
        _this: Object,
        _cmd: Selector,
        _manager: Object,
        locations: Object,
    ) {
        let last = send_object(locations, selector(c"lastObject"));
        if last.is_null() {
            send_error("macOS did not return a location fix.");
            return;
        }
        let get_coordinate: unsafe extern "C" fn(Object, Selector) -> Coordinate =
            mem::transmute(objc_msgSend as *const ());
        let coordinate = get_coordinate(last, selector(c"coordinate"));
        let accuracy = send_f64(last, selector(c"horizontalAccuracy"));
        if coordinate.latitude.is_finite() && coordinate.longitude.is_finite() && accuracy >= 0.0 {
            send_result(Ok((coordinate.latitude, coordinate.longitude, accuracy)));
        } else {
            send_error("macOS returned an invalid location fix.");
        }
    }

    unsafe extern "C" fn location_failed(
        _this: Object,
        _cmd: Selector,
        _manager: Object,
        _error: Object,
    ) {
        send_error("CoreLocation could not determine the Mac's location.");
    }

    fn send_error(error: &str) {
        send_result(Err(error.to_string()));
    }

    fn send_result(result: Result<(f64, f64, f64), String>) {
        if let Ok(slot) = RESULT_SENDER.lock() {
            if let Some(sender) = slot.as_ref() {
                let _ = sender.send(result);
            }
        }
    }

    unsafe fn selector(name: &std::ffi::CStr) -> Selector {
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

    unsafe fn send_f64(receiver: Object, selector: Selector) -> f64 {
        let function: unsafe extern "C" fn(Object, Selector) -> f64 =
            mem::transmute(objc_msgSend as *const ());
        function(receiver, selector)
    }
}
