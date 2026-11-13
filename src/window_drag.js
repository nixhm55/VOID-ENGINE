// Runs in the webview before the page loads.
//
// WKWebView consumes every mouse-down inside the web content, so the window
// never sees the press that should start a drag. This script marks the page
// header as a drag region (CSS) and forwards mouse-down over it through the
// wry IPC bridge, where the native side starts the actual window drag.
(() => {
  if (window.__voidDragRegionInstalled) return;
  window.__voidDragRegionInstalled = true;

  // Gemini renders its top bar inside a <header>; the extra selectors keep
  // other wrapped apps and older Gemini layouts working too.
  const DRAG_REGION = [
    'header',
    'app-header',
    'mat-toolbar',
    '[role="banner"]',
    '[data-void-drag]',
    '.top-bar',
    'bard-sidenav-container > header',
    'chat-window > header'
  ].join(',');

  // Controls inside the header stay clickable: they are never drag handles.
  const NO_DRAG = [
    'a',
    'button',
    'input',
    'select',
    'textarea',
    'option',
    'label',
    'summary',
    'video',
    'audio',
    'iframe',
    'canvas',
    '[contenteditable=""]',
    '[contenteditable="true"]',
    '[role="button"]',
    '[role="link"]',
    '[role="tab"]',
    '[role="menuitem"]',
    '[role="checkbox"]',
    '[role="radio"]',
    '[role="switch"]',
    '[data-void-no-drag]'
  ].join(',');

  // Chromium-based builds move the window from this rule alone. WKWebView does
  // not implement app-region, so on macOS the IPC bridge below is what moves
  // the window; the rule is still injected so both webviews behave the same.
  const CSS = `
    ${DRAG_REGION} {
      -webkit-app-region: drag;
      app-region: drag;
      -webkit-user-select: none;
      user-select: none;
    }
    ${DRAG_REGION} :is(${NO_DRAG}) {
      -webkit-app-region: no-drag;
      app-region: no-drag;
    }
  `;

  const installStyle = () => {
    if (!document.documentElement) return;
    if (document.getElementById('void-drag-region-style')) return;
    const style = document.createElement('style');
    style.id = 'void-drag-region-style';
    style.textContent = CSS;
    (document.head || document.documentElement).appendChild(style);
  };

  if (document.documentElement) {
    installStyle();
  } else {
    document.addEventListener('DOMContentLoaded', installStyle, { once: true });
  }

  const post = (method) => {
    try {
      if (window.ipc && typeof window.ipc.postMessage === 'function') {
        window.ipc.postMessage(JSON.stringify({ id: 'void-drag', method }));
      }
    } catch (_) {
      // Dragging is a convenience; never let a failed bridge break the page.
    }
  };

  const isDragRegion = (event) => {
    const path =
      typeof event.composedPath === 'function'
        ? event.composedPath()
        : [event.target];
    for (const node of path) {
      if (!node || node.nodeType !== 1) continue;
      if (node.matches(NO_DRAG)) return false;
      if (node.matches(DRAG_REGION)) return true;
    }
    return false;
  };

  document.addEventListener(
    'mousedown',
    (event) => {
      if (event.button !== 0) return;
      if (event.defaultPrevented) return;
      if (!isDragRegion(event)) return;
      // Suppress text selection for the duration of the drag.
      event.preventDefault();
      post('window.drag');
    },
    true
  );
})();
