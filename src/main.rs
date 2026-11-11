use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    env, fs,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
};
#[cfg(not(target_os = "android"))]
use tao::dpi::LogicalSize;
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::{Fullscreen, WindowBuilder},
};
use wry::{
    http::{header::CONTENT_TYPE, Response, StatusCode},
    PermissionResponse, WebViewBuilder,
};

#[cfg(target_os = "macos")]
mod native_notifications;
mod user_agent;

const BUNDLED_INDEX_HTML: &str = include_str!("../dist/index.html");
const BUNDLED_VOID_JSON: &str = include_str!("../void.json");
const VOID_GLYPHS: [[&str; 5]; 4] = [
    ["##   ##", "##   ##", " ## ## ", "  ###  ", "   #   "],
    [" ##### ", "##   ##", "##   ##", "##   ##", " ##### "],
    [" ##### ", "   #   ", "   #   ", "   #   ", " ##### "],
    ["#####  ", "##   ##", "##   ##", "##   ##", "#####  "],
];

#[derive(Debug, Clone, Copy)]
enum WindowAction {
    Close,
    Minimize,
    ToggleMaximize,
    Drag,
}

const WINDOW_CONTROLS_SCRIPT: &str = r#"(() => {
  if (window !== window.top || window.__voidControlsInstalled) return;
  window.__voidControlsInstalled = true;
  const appTitle = __VOID_APP_TITLE__;
  const post = (message) => window.ipc && window.ipc.postMessage(message);
  const mount = () => {
    if (!document.documentElement || document.getElementById('__void_window_controls')) return;
    const style = document.createElement('style');
    style.textContent = `#\u005f\u005fvoid_window_controls{position:fixed;z-index:2147483647;left:0;right:0;top:0;height:36px;display:flex;align-items:center;justify-content:space-between;padding:0 12px;background:rgba(25,25,27,.94);color:#f3f3f3;font:12px -apple-system,BlinkMacSystemFont,sans-serif;user-select:none;-webkit-user-select:none}#\u005f\u005fvoid_window_controls .void-title{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;max-width:65vw;pointer-events:none}#\u005f\u005fvoid_window_controls .void-buttons{display:flex;gap:8px}#\u005f\u005fvoid_window_controls button{width:13px;height:13px;border:0;border-radius:50%;padding:0;cursor:pointer;background:#777}#\u005f\u005fvoid_window_controls button[data-action=close]{background:#ff5f57}#\u005f\u005fvoid_window_controls button[data-action=minimize]{background:#febc2e}#\u005f\u005fvoid_window_controls button[data-action=maximize]{background:#28c840}#\u005f\u005fvoid_window_controls button:focus-visible{outline:2px solid white;outline-offset:2px}`;
    (document.head || document.documentElement).appendChild(style);
    const bar = document.createElement('div');
    bar.id = '__void_window_controls';
    bar.setAttribute('role', 'toolbar');
    bar.setAttribute('aria-label', 'Window controls');
    const title = document.createElement('span');
    title.className = 'void-title';
    title.textContent = appTitle;
    const buttons = document.createElement('span');
    buttons.className = 'void-buttons';
    [['close', 'Close'], ['minimize', 'Minimize'], ['maximize', 'Maximize']].forEach(([action, label]) => {
      const button = document.createElement('button');
      button.type = 'button';
      button.dataset.action = action;
      button.setAttribute('aria-label', label);
      button.addEventListener('click', (event) => {
        event.preventDefault(); event.stopPropagation();
        post(action === 'maximize' ? 'toggle-maximize' : action);
      });
      buttons.appendChild(button);
    });
    bar.append(title, buttons);
    bar.addEventListener('pointerdown', (event) => {
      if (event.target.closest('button') || event.button !== 0) return;
      event.preventDefault(); event.stopPropagation();
      post(event.detail > 1 ? 'toggle-maximize' : 'drag');
    });
    document.documentElement.appendChild(bar);
  };
  if (document.documentElement) mount();
  else document.addEventListener('DOMContentLoaded', mount, {once:true});
})();"#;

#[cfg(not(target_os = "android"))]
fn with_window_controls<'a>(
    builder: WebViewBuilder<'a>,
    proxy: EventLoopProxy<WindowAction>,
    title: &str,
) -> WebViewBuilder<'a> {
    let title_json = serde_json::to_string(title).unwrap_or_else(|_| "\"VOID\"".to_string());
    let script = WINDOW_CONTROLS_SCRIPT.replace("__VOID_APP_TITLE__", &title_json);

    builder
        .with_initialization_script(script)
        .with_ipc_handler(move |request| {
            let action = match request.body().as_str() {
                "close" => WindowAction::Close,
                "minimize" => WindowAction::Minimize,
                "toggle-maximize" => WindowAction::ToggleMaximize,
                "drag" => WindowAction::Drag,
                _ => return,
            };
            let _ = proxy.send_event(action);
        })
}

// Android owns the activity surface and has no desktop window-manager controls.
#[cfg(target_os = "android")]
fn with_window_controls<'a>(
    builder: WebViewBuilder<'a>,
    _proxy: EventLoopProxy<WindowAction>,
    _title: &str,
) -> WebViewBuilder<'a> {
    builder
}

#[derive(Parser, Debug)]
#[command(
    name = "void",
    version,
    override_usage = "void [URL] [OPTIONS]\n       void init",
    after_help = "Examples:\n  void https://example.com --name MyApp\n  void https://example.com --width 1280 --height 800 --fullscreen\n  void init"
)]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(value_name = "URL", help = "Open a website in a VOID window")]
    url: Option<String>,

    #[arg(
        long = "url",
        value_name = "URL",
        conflicts_with = "url",
        help = "Open a website in a VOID window"
    )]
    url_flag: Option<String>,

    #[arg(
        long,
        alias = "title",
        value_name = "NAME",
        help = "Set the window title"
    )]
    name: Option<String>,

    #[arg(long, value_name = "PIXELS", help = "Set the window width")]
    width: Option<f64>,

    #[arg(long, value_name = "PIXELS", help = "Set the window height")]
    height: Option<f64>,

    #[arg(long, help = "Start in fullscreen mode")]
    fullscreen: bool,

    #[arg(long, help = "Hide the title bar where supported")]
    hide_title_bar: bool,

    #[arg(
        long,
        value_name = "PATH",
        conflicts_with_all = ["url", "url_flag"],
        help = "Load a local file or assets directory"
    )]
    assets: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Create or update void.json with a guided setup
    Init,
}

#[derive(Deserialize, Serialize, Default, Debug)]
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

fn notification_host(url: &str) -> Option<String> {
    let uri = url.parse::<wry::http::Uri>().ok()?;
    (uri.scheme_str() == Some("https"))
        .then(|| uri.host().map(str::to_ascii_lowercase))
        .flatten()
}

fn with_native_notification_bridge(
    builder: WebViewBuilder,
    allowed_host: Option<String>,
) -> WebViewBuilder {
    #[cfg(target_os = "macos")]
    {
        native_notifications::install(builder, allowed_host)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = allowed_host;
        builder
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
    if let Err(err) = start_app(load_config()) {
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

fn start_app(config: VoidConfig) -> wry::Result<()> {
    let title = non_empty(config.title).unwrap_or_else(|| "VOID".to_string());

    let event_loop = EventLoopBuilder::<WindowAction>::with_user_event().build();
    #[allow(unused_mut)]
    let mut window_builder = WindowBuilder::new()
        .with_title(title.clone())
        .with_decorations(false);

    #[cfg(not(target_os = "android"))]
    if config.width.is_some() || config.height.is_some() {
        let width = config.width.unwrap_or(1200.0);
        let height = config.height.unwrap_or(780.0);
        window_builder = window_builder.with_inner_size(LogicalSize::new(width, height));
    }

    if config.fullscreen {
        window_builder = window_builder.with_fullscreen(Some(Fullscreen::Borderless(None)));
    }

    let window = window_builder
        .build(&event_loop)
        .expect("Failed to create window");

    let resolved_url = non_empty(config.url);

    let _webview = if let Some(url) = resolved_url {
        let target_url = normalize_url(&url);
        with_native_notification_bridge(
            with_window_controls(
                WebViewBuilder::new()
                    .with_user_agent(user_agent::current())
                    // Keep camera/microphone requests on the platform's normal
                    // permission path instead of implicitly granting them.
                    .with_permission_handler(|_| PermissionResponse::Default),
                event_loop.create_proxy(),
                &title,
            ),
            notification_host(&target_url),
        )
        .with_url(&target_url)
        .build(&window)?
    } else {
        let asset_target =
            non_empty(config.assets).unwrap_or_else(|| "dist/index.html".to_string());
        let (base_dir, entry_file) = resolve_local_asset(&asset_target);
        let entry_url = format!("void://localhost/{entry_file}");

        let builder = with_native_notification_bridge(
            with_window_controls(
                WebViewBuilder::new()
                    .with_user_agent(user_agent::current())
                    .with_permission_handler(|_| PermissionResponse::Default),
                event_loop.create_proxy(),
                &title,
            ),
            notification_host(&entry_url),
        );

        builder
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

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            }
            | Event::UserEvent(WindowAction::Close) => *control_flow = ControlFlow::Exit,
            Event::UserEvent(WindowAction::Minimize) => window.set_minimized(true),
            Event::UserEvent(WindowAction::ToggleMaximize) => {
                window.set_maximized(!window.is_maximized())
            }
            Event::UserEvent(WindowAction::Drag) => {
                let _ = window.drag_window();
            }
            _ => {}
        }
    });
}

fn prompt(label: &str, default: &str) -> io::Result<String> {
    if default.is_empty() {
        print!("{label}: ");
    } else {
        print!("{label} [{default}]: ");
    }
    io::stdout().flush()?;

    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    let answer = answer.trim();
    Ok(if answer.is_empty() {
        default.to_string()
    } else {
        answer.to_string()
    })
}

fn prompt_bool(label: &str, default: bool) -> io::Result<bool> {
    let hint = if default { "Y/n" } else { "y/N" };
    loop {
        print!("{label} ({hint}): ");
        io::stdout().flush()?;

        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        match answer.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            "" => return Ok(default),
            _ => println!("Enter y or n, or press Enter to keep the default."),
        }
    }
}

fn prompt_dimension(label: &str, default: f64) -> io::Result<f64> {
    loop {
        let answer = prompt(label, &default.to_string())?;
        match answer.parse::<f64>() {
            Ok(value) if value.is_finite() && value > 0.0 => return Ok(value),
            _ => println!("Enter a number greater than zero."),
        }
    }
}

fn initialize_config() -> io::Result<()> {
    if !io::stdin().is_terminal() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "`void init` needs an interactive terminal; run it in Terminal and answer the prompts.",
        ));
    }

    let mut config = fs::read_to_string("void.json")
        .ok()
        .and_then(|contents| serde_json::from_str::<VoidConfig>(&contents).ok())
        .unwrap_or_default();

    println!("{}\n", terminal_banner());
    println!("Setup — press Enter to keep each default.\n");
    config.title = Some(prompt(
        "App name",
        config.title.as_deref().unwrap_or("VOID"),
    )?);
    let url = prompt(
        "Website URL (type 'local' to use local files)",
        config.url.as_deref().unwrap_or(""),
    )?;
    config.url = if url.trim().is_empty() || url.eq_ignore_ascii_case("local") {
        None
    } else {
        Some(url)
    };

    if config.url.is_some() {
        config.assets = None;
    } else {
        config.assets = Some(prompt(
            "Local HTML file or assets directory",
            config.assets.as_deref().unwrap_or("dist/index.html"),
        )?);
    }

    config.width = Some(prompt_dimension(
        "Window width",
        config.width.unwrap_or(1200.0),
    )?);
    config.height = Some(prompt_dimension(
        "Window height",
        config.height.unwrap_or(780.0),
    )?);
    config.fullscreen = prompt_bool("Start fullscreen", config.fullscreen)?;
    config.hide_title_bar = prompt_bool("Hide title bar (macOS)", config.hide_title_bar)?;

    let json = serde_json::to_string_pretty(&config)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    fs::write("void.json", format!("{json}\n"))?;
    println!("\nSaved settings to void.json.");
    Ok(())
}

fn terminal_banner() -> String {
    let repo = "https://github.com/nixhm55/VOID-ENGINE";
    let tagline = "Turn websites into desktop apps with Rust.";
    let color = io::stdout().is_terminal() && env::var_os("NO_COLOR").is_none();
    let prompt = if color {
        "\x1b[38;5;208m~ ⚔️ void\x1b[0m\n"
    } else {
        "~ ⚔️ void\n"
    };
    let mut banner = prompt.to_string();

    for row in 0..5 {
        let logo_line = VOID_GLYPHS
            .iter()
            .map(|glyph| glyph[row])
            .collect::<Vec<_>>()
            .join("   ");
        let logo = if color {
            format!("\x1b[38;5;208m{logo_line}\x1b[0m")
        } else {
            logo_line
        };
        banner.push_str(&logo);

        if row == 1 {
            banner.push_str("   ");
            if color {
                banner.push_str(&format!("\x1b[38;5;208m{repo}\x1b[0m"));
            } else {
                banner.push_str(repo);
            }
        } else if row == 2 {
            banner.push_str("   ");
            if color {
                banner.push_str(&format!("\x1b[92m{tagline}\x1b[0m"));
            } else {
                banner.push_str(tagline);
            }
        }
        banner.push('\n');
    }

    banner.pop();
    banner
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
        let matches = Args::command().before_help(terminal_banner()).get_matches();
        let args = Args::from_arg_matches(&matches).unwrap_or_else(|err| err.exit());
        if matches!(args.command, Some(Commands::Init)) {
            if let Err(err) = initialize_config() {
                eprintln!("Could not configure VOID: {err}");
                std::process::exit(2);
            }
            return Ok(());
        }

        let mut config = load_config();
        if let Some(url) = args.url.or(args.url_flag) {
            config.url = Some(url);
        }
        if let Some(name) = args.name {
            config.title = Some(name);
        }
        if let Some(width) = args.width {
            config.width = Some(width);
        }
        if let Some(height) = args.height {
            config.height = Some(height);
        }
        if args.fullscreen {
            config.fullscreen = true;
        }
        if args.hide_title_bar {
            config.hide_title_bar = true;
        }
        if let Some(assets) = args.assets {
            config.assets = Some(assets);
            config.url = None;
        }

        start_app(config)
    }
}
