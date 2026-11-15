#[cfg(not(target_os = "android"))]
use tao::event_loop::EventLoopProxy;
#[cfg(not(target_os = "android"))]
use wry::NewWindowResponse;
use wry::WebViewBuilder;

#[cfg(not(target_os = "android"))]
use crate::native_bridge::UserEvent;

pub(crate) fn apply(builder: WebViewBuilder<'_>) -> WebViewBuilder<'_> {
    #[cfg(target_os = "android")]
    {
        builder
    }

    #[cfg(not(target_os = "android"))]
    {
        apply_desktop(builder, None, false)
    }
}

#[cfg(not(target_os = "android"))]
pub(crate) fn apply_with_proxy(
    builder: WebViewBuilder<'_>,
    proxy: EventLoopProxy<UserEvent>,
    allow_new_window: bool,
) -> WebViewBuilder<'_> {
    apply_desktop(builder, Some(proxy), allow_new_window)
}

#[cfg(not(target_os = "android"))]
fn apply_desktop(
    builder: WebViewBuilder<'_>,
    proxy: Option<EventLoopProxy<UserEvent>>,
    allow_new_window: bool,
) -> WebViewBuilder<'_> {
    builder
        .with_back_forward_navigation_gestures(true)
        .with_navigation_handler(|url| {
            if should_open_externally(&url) {
                open_external_url(&url);
                false
            } else {
                is_internal_navigation(&url)
            }
        })
        .with_new_window_req_handler(move |url, _features| {
            if should_open_externally(&url) {
                open_external_url(&url);
                return NewWindowResponse::Deny;
            }

            if is_internal_navigation(&url) {
                if allow_new_window {
                    // Keep cookies/session in the same WebContext by allowing an
                    // in-app popup window when explicitly enabled.
                    return NewWindowResponse::Allow;
                }
                if let Some(ref event_proxy) = proxy {
                    let _ = event_proxy.send_event(UserEvent::NavigateInApp { url });
                }
            }

            NewWindowResponse::Deny
        })
}

#[cfg(not(target_os = "android"))]
fn should_open_externally(url: &str) -> bool {
    if is_store_url(url) {
        return true;
    }

    !matches!(
        url_scheme(url).as_deref(),
        Some("http" | "https" | "void" | "about" | "blob" | "data" | "file" | "javascript") | None
    )
}

#[cfg(not(target_os = "android"))]
fn is_internal_navigation(url: &str) -> bool {
    matches!(
        url_scheme(url).as_deref(),
        Some("http" | "https" | "void" | "about" | "blob" | "data")
    )
}

#[cfg(not(target_os = "android"))]
fn can_open_external_url(url: &str) -> bool {
    !matches!(
        url_scheme(url).as_deref(),
        Some("void" | "about" | "blob" | "data" | "file" | "javascript") | None
    )
}

#[cfg(not(target_os = "android"))]
fn url_scheme(url: &str) -> Option<String> {
    let (scheme, _) = url.split_once(':')?;
    if scheme.is_empty()
        || !scheme.chars().enumerate().all(|(index, character)| {
            character.is_ascii_alphabetic()
                || (index > 0
                    && (character.is_ascii_digit() || matches!(character, '+' | '.' | '-')))
        })
    {
        return None;
    }

    Some(scheme.to_ascii_lowercase())
}

#[cfg(not(target_os = "android"))]
fn is_store_url(url: &str) -> bool {
    let Some((_, remainder)) = url.split_once("://") else {
        return false;
    };
    let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority
        .rsplit('@')
        .next()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let path = remainder
        .strip_prefix(authority)
        .unwrap_or_default()
        .split(['?', '#'])
        .next()
        .unwrap_or_default();

    (host == "play.google.com" && path.starts_with("/store")) || host == "apps.apple.com"
}

#[cfg(not(target_os = "android"))]
fn open_external_url(url: &str) {
    if !can_open_external_url(url) {
        return;
    }

    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).spawn();

    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("rundll32")
        .arg("url.dll,FileProtocolHandler")
        .arg(url)
        .spawn();

    #[cfg(target_os = "linux")]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    let result: std::io::Result<std::process::Child> = Err(std::io::Error::new(
        std::io::Error::Unsupported,
        "Opening links externally is not supported on this platform",
    ));

    match result {
        Ok(mut child) => {
            let _ = std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(error) => eprintln!("Could not open link in the system browser: {error}"),
    }
}
