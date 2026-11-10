use std::{
    borrow::Cow,
    fs,
    path::{Path, PathBuf},
};
use clap::Parser;
use serde::Deserialize;
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use wry::{
    http::{header::CONTENT_TYPE, Response, StatusCode},
    WebViewBuilder,
};

const BUNDLED_INDEX_HTML: &str = include_str!("../dist/index.html");

#[derive(Parser, Debug)]
#[command(name = "void", version, about = "Void Engine - Lightweight cross-platform web wrapper")]
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
    width: Option<f64>,
    height: Option<f64>,
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

fn main() -> wry::Result<()> {
    let args = Args::parse();

    let config: VoidConfig = match fs::read_to_string("void.json") {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|err| {
            eprintln!("Warning: Failed to parse void.json ({err}), falling back to defaults.");
            VoidConfig::default()
        }),
        Err(_) => VoidConfig::default(),
    };

    let title = config.title.unwrap_or_else(|| "VOID".to_string());

    let event_loop = EventLoop::new();
    let mut window_builder = WindowBuilder::new().with_title(title);

    if config.width.is_some() || config.height.is_some() {
        let width = config.width.unwrap_or(800.0);
        let height = config.height.unwrap_or(600.0);
        window_builder = window_builder.with_inner_size(LogicalSize::new(width, height));
    }

    let window = window_builder
        .build(&event_loop)
        .expect("Failed to create window");

    // Priority: CLI --url > void.json assets > void.json url > bundled local dist/index.html
    let target = args
        .url
        .or(config.assets)
        .or(config.url)
        .unwrap_or_else(|| "dist/index.html".to_string());

    let _webview = if is_remote_url(&target) {
        WebViewBuilder::new()
            .with_url(&target)
            .build(&window)?
    } else {
        let (base_dir, entry_file) = resolve_local_asset(&target);
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
