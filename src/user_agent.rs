#[cfg(target_os = "macos")]
use std::sync::OnceLock;
use wry::WebViewBuilder;

// Safari-compatible WKWebView identity. Google gates the Gemini desktop
// features (Task Mode, Connected Folders) on the trailing client token, so it
// has to remain the last value in the string.
#[cfg(target_os = "macos")]
const MACOS_USER_AGENT_BASE: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";
#[cfg(target_os = "macos")]
const GEMINI_DESKTOP_CLIENT: &str = "OPA/1.0.0 CoreAssistant/macos";

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

pub(crate) fn current() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        static SPOOFED: OnceLock<String> = OnceLock::new();
        SPOOFED
            .get_or_init(|| format!("{MACOS_USER_AGENT_BASE} {GEMINI_DESKTOP_CLIENT}"))
            .as_str()
    }

    #[cfg(not(target_os = "macos"))]
    {
        USER_AGENT
    }
}

/// Identity preload for the macOS desktop build. The native user agent is the
/// signal Google actually checks; the globals below cover any additional
/// JavaScript-level environment probe the page performs.
#[cfg(target_os = "macos")]
mod preload {
    use super::current;
    use std::sync::OnceLock;

    const TEMPLATE: &str = r#"(() => {
  if (window.__voidDesktopIdentityInstalled) return;
  window.__voidDesktopIdentityInstalled = true;

  const NATIVE_USER_AGENT = __VOID_USER_AGENT__;

  // WKWebView sends this agent on the wire. Pin the JS view to the exact same
  // string so a page can never observe a different identity than the network
  // did, even if it reads navigator before the native override is reflected.
  try {
    Object.defineProperty(Navigator.prototype, 'userAgent', {
      configurable: true,
      get: () => NATIVE_USER_AGENT
    });
    Object.defineProperty(Navigator.prototype, 'appVersion', {
      configurable: true,
      get: () => NATIVE_USER_AGENT.replace(/^Mozilla\//, '')
    });
  } catch (_) {}

  // Best-effort desktop flags. Configurable so a page that defines its own
  // values keeps winning.
  const flags = {
    isNativeDesktop: true,
    geminiDesktop: true,
    __GEMINI_DESKTOP__: true
  };
  for (const name of Object.keys(flags)) {
    try {
      Object.defineProperty(window, name, {
        configurable: true,
        enumerable: false,
        value: flags[name]
      });
    } catch (_) {
      window[name] = flags[name];
    }
  }

  const nativeApi = {
    isDesktop: true,
    app: 'VOID Engine',
    platform: 'macos',
    version: '0.1.0',
    userAgent: NATIVE_USER_AGENT,
    get supportsFolderPicker() {
      return typeof window.showDirectoryPicker === 'function';
    },
    async pickDirectory(options = {}) {
      if (typeof window.showDirectoryPicker !== 'function') {
        throw new DOMException(
          'Folder selection is unavailable in this build.',
          'NotSupportedError'
        );
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

    pub(super) fn script() -> &'static str {
        static SCRIPT: OnceLock<String> = OnceLock::new();
        SCRIPT
            .get_or_init(|| {
                let agent = serde_json::to_string(current()).unwrap_or_else(|_| "\"\"".to_string());
                TEMPLATE.replace("__VOID_USER_AGENT__", &agent)
            })
            .as_str()
    }
}

pub(crate) fn apply<'a>(builder: WebViewBuilder<'a>) -> WebViewBuilder<'a> {
    let builder = builder.with_user_agent(current());
    #[cfg(target_os = "macos")]
    let builder = builder.with_initialization_script(preload::script());
    builder
}
