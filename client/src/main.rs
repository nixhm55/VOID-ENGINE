use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    time::Duration,
};

use reqwest::{header, Client, Url};
use serde_json::{json, Value};

const DEFAULT_REPOSITORY: &str = "nixhm55/VOID-ENGINE";
const BUILD_PROXY_ENDPOINT: &str = "https://engineproxy.vercel.app/api";
const API_VERSION: &str = "2026-03-10";
const POLL_INTERVAL: Duration = Duration::from_secs(5);
const BUILD_TIMEOUT: Duration = Duration::from_secs(60 * 60);

struct Arguments {
    target_url: String,
    app_name: Option<String>,
    icon_url: Option<String>,
}

enum ParseResult {
    Help,
    Version,
    Run(Arguments),
}

fn usage() -> &'static str {
    "VOID Cloud Builder\n\n\
     Usage: void <URL> [--name <NAME>] [--icon-url <URL>]\n\n\
     Options:\n\
       --name <NAME>       Name shown for the generated app\n\
       --icon-url <URL>    Optional PNG/ICO icon URL\n\
       -h, --help          Show this help\n\
       -V, --version       Show the client version\n\n\
     Environment:\n\
       VOID_GITHUB_TOKEN        GitHub token with Actions: read permission for polling/downloads\n\
       VOID_GITHUB_REPOSITORY   Build repository for polling (default: nixhm55/VOID-ENGINE)\n\
     Downloaded ZIPs and extracted packages are saved in the current folder."
}

fn parse_args() -> Result<ParseResult, String> {
    let mut args = env::args().skip(1).peekable();
    let mut target_url = None;
    let mut app_name = None;
    let mut icon_url = None;

    while let Some(arg) = args.next() {
        if arg == "-h" || arg == "--help" {
            return Ok(ParseResult::Help);
        }
        if arg == "-V" || arg == "--version" {
            return Ok(ParseResult::Version);
        }

        if let Some(value) = arg.strip_prefix("--name=") {
            app_name = Some(value.to_string());
            continue;
        }
        if let Some(value) = arg.strip_prefix("--icon-url=") {
            icon_url = Some(value.to_string());
            continue;
        }
        if arg == "--name" || arg == "--icon-url" || arg == "--url" {
            let value = args
                .next()
                .ok_or_else(|| format!("{arg} requires a value"))?;
            match arg.as_str() {
                "--name" => app_name = Some(value),
                "--icon-url" => icon_url = Some(value),
                "--url" => target_url = Some(value),
                _ => unreachable!(),
            }
            continue;
        }
        if arg.starts_with('-') {
            return Err(format!("unknown option: {arg}\n\n{}", usage()));
        }
        if target_url.replace(arg).is_some() {
            return Err(format!(
                "only one target URL may be supplied\n\n{}",
                usage()
            ));
        }
    }

    let target_url =
        target_url.ok_or_else(|| format!("a target URL is required\n\n{}", usage()))?;
    let parsed = Url::parse(&target_url)
        .map_err(|_| "target URL must be a complete http:// or https:// URL".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("target URL must be a complete http:// or https:// URL".to_string());
    }

    if let Some(icon) = icon_url.as_deref() {
        let parsed_icon = Url::parse(icon)
            .map_err(|_| "icon URL must be a complete http:// or https:// URL".to_string())?;
        if !matches!(parsed_icon.scheme(), "http" | "https") || parsed_icon.host_str().is_none() {
            return Err("icon URL must be a complete http:// or https:// URL".to_string());
        }
    }

    if let Some(name) = app_name.as_deref() {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err("app name cannot be empty".to_string());
        }
        if trimmed.chars().count() > 80 || trimmed.chars().any(char::is_control) {
            return Err(
                "app name must be at most 80 characters and contain no control characters"
                    .to_string(),
            );
        }
    }

    Ok(ParseResult::Run(Arguments {
        target_url: parsed.to_string(),
        app_name,
        icon_url,
    }))
}

fn validate_repository(repository: &str) -> Result<(&str, &str), String> {
    let mut pieces = repository.split('/');
    let owner = pieces.next().unwrap_or_default();
    let name = pieces.next().unwrap_or_default();
    let valid_piece = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
            && part != "."
            && part != ".."
    };

    if pieces.next().is_some() || !valid_piece(owner) || !valid_piece(name) {
        return Err("VOID_GITHUB_REPOSITORY must have the form owner/repository".to_string());
    }
    Ok((owner, name))
}

fn title_from_url(target_url: &str) -> String {
    let host = Url::parse(target_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_else(|| "VOID".to_string());
    let label = host
        .strip_prefix("www.")
        .unwrap_or(&host)
        .split('.')
        .next()
        .unwrap_or("VOID");
    let words = label.replace(['-', '_'], " ");
    words
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn env_value(key: &str, default: &str) -> String {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn safe_component(value: &str) -> String {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let trimmed = normalized.trim_matches('-');
    if trimmed.is_empty() {
        "void".to_string()
    } else {
        trimmed.chars().take(48).collect()
    }
}

fn api_endpoint(repository: &str, path: &str) -> String {
    format!("https://api.github.com/repos/{repository}/{path}")
}

fn github_get(client: &Client, token: &str, url: &str) -> reqwest::RequestBuilder {
    client
        .get(url)
        .bearer_auth(token)
        .header(header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", API_VERSION)
}

async fn response_json(response: reqwest::Response, context: &str) -> Result<Value, String> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("could not read {context}: {error}"))?;
    if !status.is_success() {
        let detail = body.trim();
        let detail = if detail.is_empty() {
            String::new()
        } else {
            format!(": {detail}")
        };
        return Err(format!(
            "GitHub returned HTTP {status} while {context}{detail}"
        ));
    }
    serde_json::from_str(&body)
        .map_err(|error| format!("GitHub returned invalid {context}: {error}"))
}

async fn wait_for_run(
    client: &Client,
    token: &str,
    repository: &str,
    run_id: u64,
    run_url: &str,
) -> Result<(), String> {
    let endpoint = api_endpoint(repository, &format!("actions/runs/{run_id}"));
    let started = tokio::time::Instant::now();
    let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    let mut tick = 0usize;

    loop {
        let response = github_get(client, token, &endpoint)
            .send()
            .await
            .map_err(|error| format!("could not check cloud build status: {error}"))?;
        let run = response_json(response, "checking the cloud build status").await?;
        let status = run
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");

        let elapsed = started.elapsed().as_secs();
        let label = match status {
            "queued" | "requested" | "waiting" => "Waiting for a GitHub runner",
            "in_progress" => "Building your app",
            other => other,
        };
        print!(
            "\r{} {} · {:02}:{:02}  {}",
            spinner[tick % spinner.len()],
            label,
            elapsed / 60,
            elapsed % 60,
            run_url
        );
        let _ = io::stdout().flush();
        tick += 1;

        if status == "completed" {
            println!();
            let conclusion = run
                .get("conclusion")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            if conclusion != "success" {
                return Err(format!(
                    "cloud build finished with conclusion '{conclusion}': {run_url}"
                ));
            }
            println!("Build completed successfully.");
            return Ok(());
        }

        if started.elapsed() >= BUILD_TIMEOUT {
            return Err(format!(
                "cloud build did not finish within {} minutes: {run_url}",
                BUILD_TIMEOUT.as_secs() / 60
            ));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

#[cfg(target_os = "macos")]
fn extract_archive(archive: &Path, destination: &Path) -> Result<(), String> {
    let status = Command::new("ditto")
        .arg("-x")
        .arg("-k")
        .arg(archive)
        .arg(destination)
        .status()
        .map_err(|error| format!("could not start macOS ZIP extractor (ditto): {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("macOS ZIP extraction failed with {status}"))
    }
}

#[cfg(target_os = "windows")]
fn extract_archive(archive: &Path, destination: &Path) -> Result<(), String> {
    let status = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Expand-Archive -LiteralPath $env:VOID_ZIP_PATH -DestinationPath $env:VOID_DEST_PATH -Force",
        ])
        .env("VOID_ZIP_PATH", archive)
        .env("VOID_DEST_PATH", destination)
        .status()
        .map_err(|error| format!("could not start Windows ZIP extractor: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Windows ZIP extraction failed with {status}"))
    }
}

#[cfg(target_os = "linux")]
fn extract_archive(archive: &Path, destination: &Path) -> Result<(), String> {
    let mut failures = Vec::new();
    for tool in ["unzip", "bsdtar"] {
        let mut command = Command::new(tool);
        if tool == "unzip" {
            command
                .args(["-o", "-q"])
                .arg(archive)
                .arg("-d")
                .arg(destination);
        } else {
            command
                .args(["-xf"])
                .arg(archive)
                .arg("-C")
                .arg(destination);
        }
        match command.status() {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => failures.push(format!("{tool} exited with {status}")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => failures.push(format!("could not run {tool}: {error}")),
        }
    }

    for python in ["python3", "python"] {
        match Command::new(python)
            .args(["-m", "zipfile", "-e"])
            .arg(archive)
            .arg(destination)
            .status()
        {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => failures.push(format!("{python} ZIP extraction exited with {status}")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => failures.push(format!("could not run {python}: {error}")),
        }
    }

    let detail = if failures.is_empty() {
        "no system ZIP extractor was found (install unzip or Python 3)".to_string()
    } else {
        failures.join("; ")
    };
    Err(format!("could not extract {}: {detail}", archive.display()))
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn extract_archive(_archive: &Path, _destination: &Path) -> Result<(), String> {
    Err("automatic ZIP extraction is supported on macOS, Windows, and Linux".to_string())
}

async fn download_artifacts(
    client: &Client,
    token: &str,
    repository: &str,
    run_id: u64,
) -> Result<PathBuf, String> {
    let endpoint = api_endpoint(
        repository,
        &format!("actions/runs/{run_id}/artifacts?per_page=100"),
    );
    let response = github_get(client, token, &endpoint)
        .send()
        .await
        .map_err(|error| format!("could not list cloud build artifacts: {error}"))?;
    let payload = response_json(response, "listing the cloud build artifacts").await?;
    let artifacts = payload
        .get("artifacts")
        .and_then(Value::as_array)
        .ok_or_else(|| "GitHub returned no artifact list for this cloud build".to_string())?;
    let output_dir = env::current_dir()
        .map_err(|error| format!("could not read the current folder: {error}"))?;

    let mut downloaded = 0usize;
    for artifact in artifacts {
        if artifact.get("expired").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let name = artifact
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "GitHub returned an artifact without a name".to_string())?;
        let artifact_id = artifact
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("GitHub returned an invalid ID for artifact '{name}'"))?;
        let destination = output_dir.join(format!("{}-{run_id}.zip", safe_component(name)));
        let endpoint = api_endpoint(repository, &format!("actions/artifacts/{artifact_id}/zip"));
        println!("Downloading {name}...");
        let response = github_get(client, token, &endpoint)
            .send()
            .await
            .map_err(|error| format!("could not download artifact '{name}': {error}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let detail = response.text().await.unwrap_or_default();
            return Err(format!(
                "GitHub returned HTTP {status} downloading artifact '{name}': {}",
                detail.trim()
            ));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| format!("could not read artifact '{name}': {error}"))?;
        fs::write(&destination, &bytes)
            .map_err(|error| format!("could not save {}: {error}", destination.display()))?;
        println!(
            "Saved {} ({:.1} MB)",
            destination.display(),
            bytes.len() as f64 / 1_000_000.0
        );
        extract_archive(&destination, &output_dir)?;
        downloaded += 1;
        println!("Extracted {name} into {}", output_dir.display());
    }

    if downloaded == 0 {
        return Err("the cloud build succeeded but produced no downloadable artifacts".to_string());
    }
    Ok(output_dir)
}

async fn dispatch(arguments: Arguments) -> Result<(), String> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| "could not initialize the HTTPS crypto provider".to_string())?;

    let token = env::var("VOID_GITHUB_TOKEN")
        .or_else(|_| env::var("GH_TOKEN"))
        .or_else(|_| env::var("GITHUB_TOKEN"))
        .map_err(|_| {
            "set VOID_GITHUB_TOKEN to a GitHub token with Actions: read permission for build polling and artifact downloads".to_string()
        })?;
    if token.trim().is_empty() {
        return Err("VOID_GITHUB_TOKEN is empty".to_string());
    }

    let repository = env_value("VOID_GITHUB_REPOSITORY", DEFAULT_REPOSITORY);
    let (owner, repo) = validate_repository(&repository)?;
    let repository = format!("{owner}/{repo}");
    let app_name = arguments
        .app_name
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| title_from_url(&arguments.target_url));
    let icon_url = arguments.icon_url.unwrap_or_default();
    let client = Client::builder()
        .user_agent(concat!("void-cli/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("could not create the cloud build client: {error}"))?;

    let mut payload = json!({
        "target_url": arguments.target_url,
        "app_name": app_name,
    });
    if !icon_url.is_empty() {
        payload["icon_url"] = json!(icon_url);
    }
    let response = client
        .post(BUILD_PROXY_ENDPOINT)
        .header(header::ACCEPT, "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|error| format!("Vercel build proxy request failed: {error}"))?;

    let status = response.status();
    let response_text = response
        .text()
        .await
        .map_err(|error| format!("could not read the build proxy response: {error}"))?;
    if !status.is_success() {
        let detail = response_text.trim();
        let detail = if detail.is_empty() {
            String::new()
        } else {
            format!(": {detail}")
        };
        return Err(format!("build proxy returned HTTP {status}{detail}"));
    }

    let details: Value = serde_json::from_str(&response_text)
        .map_err(|error| format!("build proxy returned invalid JSON: {error}"))?;
    let workflow_run = details.get("workflow_run");
    let run_id = details
        .get("workflow_run_id")
        .or_else(|| details.get("run_id"))
        .or_else(|| workflow_run.and_then(|run| run.get("id")))
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            "build proxy did not return a workflow run ID; confirm it returns the dispatched run details"
                .to_string()
        })?;
    let run_url = details
        .get("html_url")
        .and_then(Value::as_str)
        .or_else(|| details.get("run_url").and_then(Value::as_str))
        .or_else(|| workflow_run.and_then(|run| run.get("html_url")).and_then(Value::as_str))
        .unwrap_or("https://github.com/actions");
    println!("Started cloud build for {app_name}: {run_url}");
    wait_for_run(&client, token.trim(), &repository, run_id, run_url).await?;
    println!("Cloud build succeeded. Downloading artifacts...");
    let output_dir = download_artifacts(&client, token.trim(), &repository, run_id).await?;
    println!(
        "All downloaded ZIPs and extracted files are in {}",
        output_dir.display()
    );
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match parse_args() {
        Ok(ParseResult::Help) => {
            println!("{}", usage());
            ExitCode::SUCCESS
        }
        Ok(ParseResult::Version) => {
            println!("void {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(ParseResult::Run(arguments)) => match dispatch(arguments).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(2)
        }
    }
}
