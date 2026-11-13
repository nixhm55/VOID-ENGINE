use wry::WebViewBuilder;

// Keep these platform identities aligned with current browsers. The macOS
// string matches Pake's Safari-compatible WKWebView identity.
#[cfg(target_os = "macos")]
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15 Gemini/1.0 VOIDEngine/0.1.0";

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

pub(crate) const fn current() -> &'static str {
    USER_AGENT
}

#[cfg(target_os = "macos")]
const NATIVE_PRELOAD: &str = r#"(() => {
  window.isNativeDesktop = true;
  const nativeApi = {
    isDesktop: true,
    app: 'VOID Engine',
    platform: 'macos',
    version: '0.1.0',
    get supportsFolderPicker() {
      return typeof window.showDirectoryPicker === 'function';
    },
    async pickDirectory(options = {}) {
      if (typeof window.showDirectoryPicker !== 'function') {
        throw new DOMException('Folder selection is unavailable in this build.', 'NotSupportedError');
      }
      return window.showDirectoryPicker(options);
    }
  };
  Object.defineProperty(window, '__VOID_NATIVE__', {
    configurable: false,
    enumerable: false,
    writable: false,
    value: Object.freeze(nativeApi)
  });
})();"#;

pub(crate) fn apply<'a>(builder: WebViewBuilder<'a>) -> WebViewBuilder<'a> {
    let builder = builder.with_user_agent(current());
    #[cfg(target_os = "macos")]
    let builder = builder.with_initialization_script(NATIVE_PRELOAD);
    builder
}
