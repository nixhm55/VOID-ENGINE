use serde::Deserialize;
use std::{
    borrow::Cow,
    fs,
    path::{Path, PathBuf},
};
#[cfg(not(target_os = "android"))]
use tao::dpi::LogicalSize;
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use wry::{
    http::{header::CONTENT_TYPE, Response, StatusCode},
    PermissionResponse, WebViewBuilder,
};

mod user_agent;

const BUNDLED_INDEX_HTML: &str = include_str!("../dist/index.html");
const BUNDLED_VOID_JSON: &str = include_str!("../void.json");

#[derive(Deserialize, Default, Debug)]
pub struct VoidConfig {
    pub url: Option<String>,
    #[serde(alias = "dist", alias = "local", alias = "entry")]
    pub assets: Option<String>,
    pub title: Option<String>,
    #[allow(dead_code)]
    pub width: Option<f64>,
    #[allow(dead_code)]
    pub height: Option<f64>,
}

fn load_config() -> VoidConfig {
    if let Ok(content) = fs::read_to_string("void.json") {
        if let Ok(cfg) = serde_json::from_str::<VoidConfig>(&content) {
            return cfg;
        }
    }
    serde_json::from_str::<VoidConfig>(BUNDLED_VOID_JSON).unwrap_or_default()
}

fn is_remote_url(target: &str) -> bool {
    target.starts_with("http://") || target.starts_with("https://")
}

fn mime_type_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") | Some("mjs") => "application/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}

fn resolve_local_asset(asset_spec: &str) -> (PathBuf, String) {
    let path = Path::new(asset_spec);
    if path.is_file() || asset_spec.ends_with(".html") || asset_spec.ends_with(".htm") {
        let base_dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let entry_file = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("index.html")
            .to_string();
        (base_dir, entry_file)
    } else {
        (path.to_path_buf(), "index.html".to_string())
    }
}

#[cfg(target_os = "android")]
fn _start_app() {
    if let Err(err) = start_app(None) {
        eprintln!("Void Engine Android runtime error: {err}");
    }
}

#[cfg(target_os = "android")]
fn init_android_bindings() {
    tao::android_binding!(
        com_void,
        void_engine,
        WryActivity,
        wry::android_setup,
        _start_app
    );
    wry::android_binding!(com_void, void_engine);
}

#[cfg(target_os = "android")]
#[repr(C)]
pub struct ANativeActivity {
    pub callbacks: *mut std::ffi::c_void,
    pub vm: *mut std::ffi::c_void,
    pub env: *mut std::ffi::c_void,
    pub clazz: *mut std::ffi::c_void,
    pub internal_data_path: *const std::os::raw::c_char,
    pub external_data_path: *const std::os::raw::c_char,
    pub sdk_version: i32,
    pub instance: *mut std::ffi::c_void,
    pub asset_manager: *mut std::ffi::c_void,
    pub obb_path: *const std::os::raw::c_char,
}

#[cfg(target_os = "android")]
unsafe fn setup_native_activity_webview(
    env: &mut jni::JNIEnv,
    activity_obj: &jni::objects::JObject,
) -> jni::errors::Result<()> {
    use jni::objects::{JObject, JValue};

    let null_obj = JObject::null();

    // Release NativeActivity's raw surface & input queue takeover so Android's
    // View hierarchy renders the WebView and dispatches touch events without ANR.
    if let Ok(window_val) =
        env.call_method(activity_obj, "getWindow", "()Landroid/view/Window;", &[])
    {
        if let Ok(window_obj) = window_val.l() {
            if !window_obj.is_null() {
                let _ = env.call_method(
                    &window_obj,
                    "takeSurface",
                    "(Landroid/view/SurfaceHolder$Callback2;)V",
                    &[JValue::Object(&null_obj)],
                );
                let _ = env.call_method(
                    &window_obj,
                    "takeInputQueue",
                    "(Landroid/view/InputQueue$Callback;)V",
                    &[JValue::Object(&null_obj)],
                );
            }
        }
    }

    let webview = env.new_object(
        "android/webkit/WebView",
        "(Landroid/content/Context;)V",
        &[JValue::Object(activity_obj)],
    )?;

    if let Ok(settings_val) = env.call_method(
        &webview,
        "getSettings",
        "()Landroid/webkit/WebSettings;",
        &[],
    ) {
        if let Ok(settings) = settings_val.l() {
            if let Ok(user_agent) = env.new_string(user_agent::current()) {
                let user_agent_obj = JObject::from(user_agent);
                let _ = env.call_method(
                    &settings,
                    "setUserAgentString",
                    "(Ljava/lang/String;)V",
                    &[JValue::Object(&user_agent_obj)],
                );
            }
            let _ = env.call_method(
                &settings,
                "setJavaScriptEnabled",
                "(Z)V",
                &[JValue::Bool(1)],
            );
            let _ = env.call_method(
                &settings,
                "setDomStorageEnabled",
                "(Z)V",
                &[JValue::Bool(1)],
            );
            let _ = env.call_method(&settings, "setAllowFileAccess", "(Z)V", &[JValue::Bool(1)]);
        }
    }

    if let Ok(client) = env.new_object("android/webkit/WebViewClient", "()V", &[]) {
        let _ = env.call_method(
            &webview,
            "setWebViewClient",
            "(Landroid/webkit/WebViewClient;)V",
            &[JValue::Object(&client)],
        );
    }

    env.call_method(
        activity_obj,
        "setContentView",
        "(Landroid/view/View;)V",
        &[JValue::Object(&webview)],
    )?;

    let config = load_config();
    let target = config
        .url
        .or(config.assets)
        .unwrap_or_else(|| "dist/index.html".to_string());

    if is_remote_url(&target) {
        let j_url = env.new_string(&target)?;
        let j_url_obj = JObject::from(j_url);
        env.call_method(
            &webview,
            "loadUrl",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&j_url_obj)],
        )?;
    } else {
        let base_url = env.new_string("file:///android_asset/")?;
        let html_data = env.new_string(BUNDLED_INDEX_HTML)?;
        let mime_type = env.new_string("text/html")?;
        let encoding = env.new_string("UTF-8")?;
        let base_obj = JObject::from(base_url);
        let html_obj = JObject::from(html_data);
        let mime_obj = JObject::from(mime_type);
        let enc_obj = JObject::from(encoding);

        env.call_method(
            &webview,
            "loadDataWithBaseURL",
            "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V",
            &[
                JValue::Object(&base_obj),
                JValue::Object(&html_obj),
                JValue::Object(&mime_obj),
                JValue::Object(&enc_obj),
                JValue::Object(&null_obj),
            ],
        )?;
    }

    Ok(())
}

#[cfg(target_os = "android")]
#[no_mangle]
pub unsafe extern "C" fn ANativeActivity_onCreate(
    activity: *mut ANativeActivity,
    _saved_state: *mut std::ffi::c_void,
    _saved_state_size: usize,
) {
    init_android_bindings();

    static NDK_INIT: std::sync::Once = std::sync::Once::new();
    if let Some(act) = activity.as_ref() {
        NDK_INIT.call_once(|| {
            ndk_context::initialize_android_context(act.vm, act.clazz);
        });

        if let Ok(mut env) = jni::JNIEnv::from_raw(act.env.cast()) {
            let activity_obj = jni::objects::JObject::from_raw(act.clazz.cast());
            if setup_native_activity_webview(&mut env, &activity_obj).is_err()
                && env.exception_check().unwrap_or(false)
            {
                let _ = env.exception_describe();
                let _ = env.exception_clear();
            }
        }
    }
}

pub fn start_app(cli_url: Option<String>) -> wry::Result<()> {
    let config = load_config();
    let title = config.title.unwrap_or_else(|| "VOID".to_string());

    let event_loop = EventLoop::new();
    #[allow(unused_mut)]
    let mut window_builder = WindowBuilder::new().with_title(title);

    #[cfg(not(target_os = "android"))]
    if config.width.is_some() || config.height.is_some() {
        let width = config.width.unwrap_or(800.0);
        let height = config.height.unwrap_or(600.0);
        window_builder = window_builder.with_inner_size(LogicalSize::new(width, height));
    }

    let window = window_builder
        .build(&event_loop)
        .expect("Failed to create window");

    // Priority: CLI --url > void.json assets > void.json url > bundled local dist/index.html
    let target = cli_url
        .or(config.assets)
        .or(config.url)
        .unwrap_or_else(|| "dist/index.html".to_string());

    let _webview = if is_remote_url(&target) {
        WebViewBuilder::new()
            .with_user_agent(user_agent::current())
            // Keep camera/microphone requests on the platform's normal
            // permission path instead of implicitly granting them.
            .with_permission_handler(|_| PermissionResponse::Default)
            .with_url(&target)
            .build(&window)?
    } else {
        let (base_dir, entry_file) = resolve_local_asset(&target);
        let entry_url = format!("void://localhost/{entry_file}");

        WebViewBuilder::new()
            .with_user_agent(user_agent::current())
            .with_permission_handler(|_| PermissionResponse::Default)
            .with_custom_protocol("void".into(), move |_webview_id, request| {
                let raw_path = request.uri().path().trim_start_matches('/');
                let rel_path = if raw_path.is_empty() {
                    entry_file.as_str()
                } else {
                    raw_path
                };

                let file_path = base_dir.join(rel_path);
                if let Ok(bytes) = fs::read(&file_path) {
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(CONTENT_TYPE, mime_type_for_path(&file_path))
                        .body(Cow::Owned(bytes))
                        .unwrap()
                } else if rel_path == "index.html" {
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(CONTENT_TYPE, "text/html; charset=utf-8")
                        .body(Cow::Borrowed(BUNDLED_INDEX_HTML.as_bytes()))
                        .unwrap()
                } else {
                    Response::builder()
                        .status(StatusCode::NOT_FOUND)
                        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
                        .body(Cow::Borrowed(b"404 Not Found" as &'static [u8]))
                        .unwrap()
                }
            })
            .with_url(&entry_url)
            .build(&window)?
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *control_flow = ControlFlow::Exit;
        }
    });
}
