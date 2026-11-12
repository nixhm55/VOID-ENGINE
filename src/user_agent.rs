use wry::WebViewBuilder;

// Keep the browser version centralized so it is easy to refresh as Chrome
// releases new stable versions. macOS deliberately keeps WKWebView's native UA.
#[cfg(target_os = "windows")]
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/157.0.0.0 Safari/537.36";

#[cfg(target_os = "linux")]
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/157.0.0.0 Safari/537.36";

#[cfg(target_os = "android")]
const USER_AGENT: &str = "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/157.0.0.0 Mobile Safari/537.36";

#[cfg(not(any(
    target_os = "macos",
    target_os = "windows",
    target_os = "linux",
    target_os = "android"
)))]
const USER_AGENT: &str =
    "Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko) Chrome/157.0.0.0 Safari/537.36";

#[cfg(not(target_os = "macos"))]
pub(crate) const fn current() -> &'static str {
    USER_AGENT
}

pub(crate) fn apply<'a>(builder: WebViewBuilder<'a>) -> WebViewBuilder<'a> {
    #[cfg(target_os = "macos")]
    {
        // WKWebView should identify itself naturally. A Chrome User-Agent on
        // WebKit can make sign-in providers reject the embedded browser.
        builder
    }

    #[cfg(not(target_os = "macos"))]
    {
        builder.with_user_agent(current())
    }
}
