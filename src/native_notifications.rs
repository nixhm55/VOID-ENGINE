use serde::Deserialize;
use std::{process::Command, thread};
use wry::{http::Request, WebViewBuilder};

const NOTIFICATION_SHIM: &str = r"(() => {
  if (!window.ipc || typeof window.ipc.postMessage !== 'function') return;
  class VoidNotification extends EventTarget {
    constructor(title, options = {}) {
      super();
      this.title = String(title || '');
      this.body = String(options.body || '');
      this.tag = String(options.tag || '');
      window.ipc.postMessage(JSON.stringify({ title: this.title, body: this.body }));
      setTimeout(() => this.dispatchEvent(new Event('show')), 0);
    }
    static get permission() { return 'granted'; }
    static requestPermission(callback) {
      const result = Promise.resolve('granted');
      if (typeof callback === 'function') result.then(callback);
      return result;
    }
    close() {}
  }
  window.Notification = VoidNotification;
})();";

#[derive(Deserialize)]
struct NotificationMessage {
    title: String,
    #[serde(default)]
    body: String,
}

pub(crate) fn install<'a>(
    builder: WebViewBuilder<'a>,
    allowed_host: Option<&str>,
) -> WebViewBuilder<'a> {
    if allowed_host.is_none() {
        return builder;
    }

    builder.with_initialization_script(NOTIFICATION_SHIM)
}

pub(crate) fn handle(request: &Request<String>, allowed_host: Option<&str>) -> bool {
    let Some(allowed_host) = allowed_host else {
        return false;
    };
    let uri = request.uri();
    if uri.scheme_str() != Some("https")
        || uri.host().map(str::to_ascii_lowercase).as_deref() != Some(allowed_host)
    {
        return false;
    }

    let Ok(message) = serde_json::from_str::<NotificationMessage>(request.body()) else {
        return false;
    };
    let title = limit_text(&message.title, 180);
    let body = limit_text(&message.body, 600);
    if title.is_empty() && body.is_empty() {
        return true;
    }

    thread::spawn(move || show_notification(&title, &body));
    true
}

fn limit_text(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control() || *character == '\n' || *character == '\t')
        .take(max_chars)
        .collect::<String>()
        .replace(['\n', '\r', '\t'], " ")
}

fn show_notification(title: &str, body: &str) {
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        escape_applescript(body),
        escape_applescript(title)
    );
    let _ = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .status();
}

fn escape_applescript(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
