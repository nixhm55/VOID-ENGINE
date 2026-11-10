use std::{
    borrow::Cow,
    env,
    fs,
    path::{Path, PathBuf},
};
use clap::Parser;
use serde::Deserialize;
#[cfg(not(target_os = "android"))]
use tao::dpi::LogicalSize;
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::{Fullscreen, WindowBuilder},
};
use wry::{
    http::{header::CONTENT_TYPE, Response, StatusCode},
    WebViewBuilder,
};

const BUNDLED_INDEX_HTML: &str = include_str!("../dist/index.html");
const BUNDLED_VOID_JSON: &str = include_str!("../void.json");

#[derive(Parser, Debug)]
#[command(name = "void", version, about = "Void Engine - Lightweight cross-platform web wrapper")]
#[allow(dead_code)]
struct Args {
    #[arg(long)]
    url: Option<String>,
}

#[derive(Deserialize, Default, Debug)]
struct VoidConfig {
    url: Option<String>,
    #[serde(alias = "dist", alias = "local", alias = "entry")]
    assets: Option<String>,
    title: Option<String>,
    #[allow(dead_code)]
    width: Option<f64>,
    #[allow(dead_code)]
    height: Option<f64>,
    #[serde(default)]
    fullscreen: bool,
    #[allow(dead_code)]
    #[serde(default)]
    hide_title_bar: bool,
    #[allow(dead_code)]
    #[serde(default)]
    new_window: bool,
}

fn non_empty(opt: Option<String>) -> Option<String> {
    opt.and_then(|s| {
        let trimmed = s.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn load_config() -> VoidConfig {
    let mut candidate_paths = vec![PathBuf::from("void.json")];

    if let Ok(exe_path) = env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            candidate_paths.push(exe_dir.join("void.json"));
            candidate_paths.push(exe_dir.join("../Resources/void.json"));
        }
    }

    for path in candidate_paths {
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str::<VoidConfig>(&content) {
                return cfg;
            }
        }
    }

    serde_json::from_str::<VoidConfig>(BUNDLED_VOID_JSON).unwrap_or_default()
}

fn is_remote_url(target: &str) -> bool {
    target.starts_with("http://") || target.starts_with("https://")
}

fn normalize_url(raw_url: &str) -> String {
    let trimmed = raw_url.trim();
    if is_remote_url(trimmed) {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    }
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
#[no_mangle]
pub extern "C" fn init_void_android() {
    tao::android_binding!(
        com_void,
        void_engine,
        WryActivity,
        wry::android_setup,
        _start_app
    );
    wry::android_binding!(com_void, void_engine);
}

fn start_app(cli_url: Option<String>) -> wry::Result<()> {
    let config = load_config();

    let title = non_empty(config.title).unwrap_or_else(|| "VOID".to_string());

    let event_loop = EventLoop::new();
    #[allow(unused_mut)]
    let mut window_builder = WindowBuilder::new().with_title(title);

    #[cfg(not(target_os = "android"))]
    if config.width.is_some() || config.height.is_some() {
        let width = config.width.unwrap_or(1200.0);
        let height = config.height.unwrap_or(780.0);
        window_builder = window_builder.with_inner_size(LogicalSize::new(width, height));
    }

    if config.fullscreen {
        window_builder = window_builder.with_fullscreen(Some(Fullscreen::Borderless(None)));
    }

    #[cfg(target_os = "macos")]
    if config.hide_title_bar {
        window_builder = window_builder.with_decorations(false);
    }

    let window = window_builder
        .build(&event_loop)
        .expect("Failed to create window");

    let resolved_url = non_empty(cli_url).or_else(|| non_empty(config.url));

    let _webview = if let Some(url) = resolved_url {
        let target_url = normalize_url(&url);
        WebViewBuilder::new()
            .with_url(&target_url)
            .build(&window)?
    } else {
        let asset_target = non_empty(config.assets).unwrap_or_else(|| "dist/index.html".to_string());
        let (base_dir, entry_file) = resolve_local_asset(&asset_target);
        let entry_url = format!("void://localhost/{entry_file}");

        WebViewBuilder::new()
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

#[allow(dead_code)]
fn main() -> wry::Result<()> {
    #[cfg(target_os = "android")]
    {
        _start_app();
        Ok(())
    }

    #[cfg(not(target_os = "android"))]
    {
        let args = Args::parse();
        start_app(args.url)
    }
}
