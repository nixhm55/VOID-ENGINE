use std::{
    io::{self, Write},
    time::Duration,
};

use keyring::{Entry, Error as KeyringError};
use reqwest::{header, Client, Url};
use serde_json::Value;

const GITHUB_OAUTH_CLIENT_ID: &str = "Ov23lirA2D30daCI1h2T";
const DEVICE_CODE_ENDPOINT: &str = "https://github.com/login/device/code";
const ACCESS_TOKEN_ENDPOINT: &str = "https://github.com/login/oauth/access_token";
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
const KEYRING_SERVICE: &str = "void-cli";
const KEYRING_ACCOUNT: &str = "github-access-token";
const DEFAULT_POLL_INTERVAL_SECS: u64 = 5;
const SLOW_DOWN_INCREMENT_SECS: u64 = 5;

struct DeviceAuthorization {
    device_code: String,
    user_code: String,
    verification_uri: String,
    interval: u64,
    expires_in: u64,
}

pub fn github_token() -> Result<String, String> {
    let entry = credential_entry()?;
    match entry.get_password() {
        Ok(token) if !token.trim().is_empty() => Ok(token),
        Ok(_) => Err("The saved GitHub credential is empty. Run `void login` again.".to_string()),
        Err(KeyringError::NoEntry) => {
            Err("You are not signed in to GitHub. Run `void login` first.".to_string())
        }
        Err(error) => Err(format!(
            "Could not read the GitHub credential from the OS keychain: {error}"
        )),
    }
}

pub async fn login(client: &Client) -> Result<(), String> {
    let authorization = request_device_authorization(client).await?;
    println!("\nSign in to GitHub to authorize VOID:");
    println!("  Open this URL: {}", authorization.verification_uri);
    println!("  Enter this code: {}", authorization.user_code);
    println!("\nWaiting for authorization… (Ctrl+C to cancel)");
    let _ = io::stdout().flush();

    let expires_at = tokio::time::Instant::now() + Duration::from_secs(authorization.expires_in);
    let mut interval = authorization.interval;

    loop {
        let now = tokio::time::Instant::now();
        if now >= expires_at {
            return Err(
                "The GitHub sign-in code expired. Run `void login` to get a new one.".to_string(),
            );
        }

        let wait = Duration::from_secs(interval);
        if now + wait >= expires_at {
            tokio::time::sleep_until(expires_at).await;
            return Err(
                "The GitHub sign-in code expired. Run `void login` to get a new one.".to_string(),
            );
        }
        tokio::time::sleep(wait).await;

        let response = client
            .post(ACCESS_TOKEN_ENDPOINT)
            .header(header::ACCEPT, "application/json")
            .form(&[
                ("client_id", GITHUB_OAUTH_CLIENT_ID),
                ("device_code", authorization.device_code.as_str()),
                ("grant_type", DEVICE_GRANT_TYPE),
            ])
            .send()
            .await
            .map_err(|error| format!("Could not poll GitHub sign-in status: {error}"))?;
        let payload = oauth_response_json(response, "checking sign-in status").await?;

        if let Some(token) = payload.get("access_token").and_then(Value::as_str) {
            if token.trim().is_empty() {
                return Err("GitHub returned an empty access token.".to_string());
            }
            credential_entry()?.set_password(token).map_err(|error| {
                format!("Could not save the GitHub token to the OS keychain: {error}")
            })?;
            println!("\nGitHub sign-in complete. Your token is saved in the OS keychain.");
            return Ok(());
        }

        match payload.get("error").and_then(Value::as_str) {
            Some("authorization_pending") => {}
            Some("slow_down") => {
                interval = interval.saturating_add(SLOW_DOWN_INCREMENT_SECS);
                println!("GitHub asked us to slow down; next check in {interval} seconds.");
            }
            Some("expired_token" | "token_expired") => {
                return Err(
                    "The GitHub sign-in code expired. Run `void login` to get a new one."
                        .to_string(),
                );
            }
            Some("access_denied") => {
                return Err("GitHub sign-in was cancelled.".to_string());
            }
            Some("device_flow_disabled") => {
                return Err("Device Flow is disabled for this GitHub OAuth App. Enable it in the app settings.".to_string());
            }
            Some("incorrect_client_credentials") => {
                return Err("GitHub rejected the OAuth App Client ID. Check client/src/auth.rs and try again.".to_string());
            }
            Some("incorrect_device_code") => {
                return Err(
                    "GitHub rejected the device code. Run `void login` to start again.".to_string(),
                );
            }
            Some("unsupported_grant_type") => {
                return Err("GitHub rejected the device authorization grant type.".to_string());
            }
            Some(error) => return Err(format!("GitHub sign-in failed: {error}")),
            None => return Err("GitHub returned no access token or sign-in status.".to_string()),
        }
    }
}

async fn request_device_authorization(client: &Client) -> Result<DeviceAuthorization, String> {
    let response = client
        .post(DEVICE_CODE_ENDPOINT)
        .header(header::ACCEPT, "application/json")
        .form(&[("client_id", GITHUB_OAUTH_CLIENT_ID)])
        .send()
        .await
        .map_err(|error| format!("Could not start GitHub sign-in: {error}"))?;
    let payload = oauth_response_json(response, "starting sign-in").await?;

    if let Some(error) = payload.get("error").and_then(Value::as_str) {
        return Err(describe_oauth_error(error));
    }

    let device_code = required_string(&payload, "device_code")?;
    let user_code = required_string(&payload, "user_code")?;
    let verification_uri = required_string(&payload, "verification_uri")?;
    let verification_url = Url::parse(&verification_uri)
        .map_err(|_| "GitHub returned an invalid verification URL.".to_string())?;
    if verification_url.scheme() != "https" || verification_url.host_str() != Some("github.com") {
        return Err("GitHub returned an unexpected verification URL; open https://github.com/login/device manually.".to_string());
    }
    let expires_in = payload
        .get("expires_in")
        .and_then(Value::as_u64)
        .filter(|seconds| *seconds > 0)
        .ok_or_else(|| "GitHub returned an invalid device-code expiration time.".to_string())?;
    let interval = payload
        .get("interval")
        .and_then(Value::as_u64)
        .filter(|seconds| *seconds > 0)
        .unwrap_or(DEFAULT_POLL_INTERVAL_SECS);

    Ok(DeviceAuthorization {
        device_code,
        user_code,
        verification_uri,
        interval,
        expires_in,
    })
}

async fn oauth_response_json(response: reqwest::Response, context: &str) -> Result<Value, String> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("Could not read GitHub's response while {context}: {error}"))?;
    let payload: Value = serde_json::from_str(&body).map_err(|_| {
        format!("GitHub returned an invalid response while {context} (HTTP {status}).")
    })?;

    if !status.is_success() {
        let detail = payload
            .get("error_description")
            .or_else(|| payload.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("request rejected");
        return Err(format!(
            "GitHub returned HTTP {status} while {context}: {detail}"
        ));
    }

    Ok(payload)
}

fn required_string(payload: &Value, field: &str) -> Result<String, String> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("GitHub's device authorization response is missing `{field}`."))
}

fn describe_oauth_error(error: &str) -> String {
    match error {
        "device_flow_disabled" => {
            "Device Flow is disabled for this GitHub OAuth App. Enable it in the app settings."
                .to_string()
        }
        "incorrect_client_credentials" => {
            "GitHub rejected the OAuth App Client ID. Check client/src/auth.rs and try again."
                .to_string()
        }
        other => format!("GitHub could not start device authorization: {other}"),
    }
}

fn credential_entry() -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .map_err(|error| format!("Could not open the OS credential store: {error}"))
}
