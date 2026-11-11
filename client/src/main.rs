use std::{env, process::ExitCode, time::Duration};

use reqwest::{header, Client, Url};
use serde_json::{json, Value};

const DEFAULT_REPOSITORY: &str = "nixhm55/VOID-ENGINE";
const DEFAULT_WORKFLOW: &str = "build.yml";
const DEFAULT_REF: &str = "main";
const API_VERSION: &str = "2026-03-10";

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
       VOID_GITHUB_TOKEN        GitHub token with Actions: write permission\n\
       VOID_GITHUB_REPOSITORY   Build repository (default: nixhm55/VOID-ENGINE)\n\
       VOID_GITHUB_WORKFLOW     Workflow file (default: build.yml)\n\
       VOID_GITHUB_REF          Branch or tag (default: main)"
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

fn validate_workflow(workflow: &str) -> Result<&str, String> {
    if workflow.is_empty()
        || !workflow
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("VOID_GITHUB_WORKFLOW must be a workflow filename or numeric ID".to_string());
    }
    Ok(workflow)
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

async fn dispatch(arguments: Arguments) -> Result<(), String> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| "could not initialize the HTTPS crypto provider".to_string())?;

    let token = env::var("VOID_GITHUB_TOKEN")
        .or_else(|_| env::var("GH_TOKEN"))
        .or_else(|_| env::var("GITHUB_TOKEN"))
        .map_err(|_| {
            "set VOID_GITHUB_TOKEN to a GitHub token with Actions: write permission".to_string()
        })?;
    if token.trim().is_empty() {
        return Err("VOID_GITHUB_TOKEN is empty".to_string());
    }

    let repository = env_value("VOID_GITHUB_REPOSITORY", DEFAULT_REPOSITORY);
    let (owner, repo) = validate_repository(&repository)?;
    let workflow = env_value("VOID_GITHUB_WORKFLOW", DEFAULT_WORKFLOW);
    validate_workflow(&workflow)?;
    let git_ref = env_value("VOID_GITHUB_REF", DEFAULT_REF);
    let app_name = arguments
        .app_name
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| title_from_url(&arguments.target_url));
    let icon_url = arguments.icon_url.unwrap_or_default();
    let endpoint = format!(
        "https://api.github.com/repos/{owner}/{repo}/actions/workflows/{workflow}/dispatches"
    );

    let client = Client::builder()
        .user_agent(concat!("void-cli/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("could not create the GitHub API client: {error}"))?;

    let response = client
        .post(endpoint)
        .bearer_auth(token.trim())
        .header(header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", API_VERSION)
        .json(&json!({
            "ref": git_ref,
            "inputs": {
                "target_url": arguments.target_url,
                "app_name": app_name,
                "icon_url": icon_url,
            },
            "return_run_details": true,
        }))
        .send()
        .await
        .map_err(|error| format!("GitHub workflow dispatch failed: {error}"))?;

    let status = response.status();
    let response_text = response
        .text()
        .await
        .map_err(|error| format!("could not read GitHub's response: {error}"))?;
    if !status.is_success() {
        let detail = response_text.trim();
        let detail = if detail.is_empty() {
            String::new()
        } else {
            format!(": {detail}")
        };
        return Err(format!("GitHub returned HTTP {status}{detail}"));
    }

    let details: Value = serde_json::from_str(&response_text).unwrap_or(Value::Null);
    if let Some(url) = details.get("html_url").and_then(Value::as_str) {
        println!("Cloud build queued: {url}");
    } else {
        println!("Cloud build queued for {app_name} ({repository}, {git_ref}).");
    }
    println!("The workflow will build the selected platform packages in GitHub Actions.");
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
