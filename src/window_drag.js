// Runs in the webview before the page loads.
//
// 1. Marks the page header as a drag region (CSS) and forwards mouse-down over
//    it on macOS through the wry IPC bridge so frameless/overlay windows can
//    be dragged smoothly.
// 2. Detects the current operating system (macOS vs Windows/Linux/Android) and
//    wires standard desktop/tablet keyboard shortcuts (Quit, Close, Reload,
//    Back/Forward, Zoom, Fullscreen, Select All, Copy, Cut, Paste, Undo, Redo).
(() => {
  if (window.__voidDragRegionInstalled) return;
  window.__voidDragRegionInstalled = true;

  const platformStr =
    (navigator.userAgentData && navigator.userAgentData.platform) ||
    navigator.platform ||
    navigator.userAgent ||
    '';
  const isMac = /Mac|iPhone|iPod|iPad/i.test(platformStr);

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

  const post = (payload) => {
    try {
      if (window.ipc && typeof window.ipc.postMessage === 'function') {
        window.ipc.postMessage(JSON.stringify(payload));
      }
    } catch (_) {
      // Never let a failed bridge break the page.
    }
  };

  const postShortcut = (action) => {
    post({ id: 'void-shortcut', method: 'window.shortcut', action });
  };

  const isEditable = (el) => {
    if (!el || el.nodeType !== 1) return false;
    const tag = (el.tagName || '').toUpperCase();
    if (tag === 'INPUT' || tag === 'TEXTAREA') return !el.readOnly && !el.disabled;
    return Boolean(el.isContentEditable);
  };

  // OS-aware keyboard shortcuts across macOS (Cmd), Windows, Linux, and Android (Ctrl/Alt).
  window.addEventListener(
    'keydown',
    (event) => {
      const key = (event.key || '').toLowerCase();
      const primaryMod = isMac
        ? event.metaKey && !event.ctrlKey && !event.altKey
        : event.ctrlKey && !event.metaKey && !event.altKey;

      // Fullscreen: Ctrl+Cmd+F on macOS, F11 on Windows/Linux
      if (
        (isMac && event.metaKey && event.ctrlKey && !event.altKey && key === 'f') ||
        (!isMac && event.key === 'F11')
      ) {
        event.preventDefault();
        postShortcut('fullscreen');
        return;
      }

      // Reload: Cmd+R on macOS, Ctrl+R or F5 on Windows/Linux/Android
      if ((primaryMod && key === 'r') || (!isMac && event.key === 'F5')) {
        event.preventDefault();
        postShortcut('reload');
        return;
      }

      // History Back / Forward
      if (
        (isMac && primaryMod && (key === '[' || event.key === 'ArrowLeft')) ||
        (!isMac && event.altKey && !event.ctrlKey && !event.metaKey && event.key === 'ArrowLeft')
      ) {
        if (!isEditable(document.activeElement)) {
          event.preventDefault();
          postShortcut('back');
          return;
        }
      }
      if (
        (isMac && primaryMod && (key === ']' || event.key === 'ArrowRight')) ||
        (!isMac && event.altKey && !event.ctrlKey && !event.metaKey && event.key === 'ArrowRight')
      ) {
        if (!isEditable(document.activeElement)) {
          event.preventDefault();
          postShortcut('forward');
          return;
        }
      }

      // Windows/Linux Alt+F4 quit
      if (!isMac && event.altKey && !event.ctrlKey && !event.metaKey && event.key === 'F4') {
        event.preventDefault();
        postShortcut('quit');
        return;
      }

      if (!primaryMod) return;

      switch (key) {
        case 'q':
          event.preventDefault();
          postShortcut('quit');
          break;
        case 'w':
          event.preventDefault();
          postShortcut('close');
          break;
        case 'm':
          if (isMac) {
            event.preventDefault();
            postShortcut('minimize');
          }
          break;
        case '=':
        case '+':
          event.preventDefault();
          postShortcut('zoom_in');
          break;
        case '-':
        case '_':
          event.preventDefault();
          postShortcut('zoom_out');
          break;
        case '0':
          event.preventDefault();
          postShortcut('zoom_reset');
          break;
        case 'a': {
          const active = document.activeElement;
          if (
            active &&
            (active.tagName === 'INPUT' || active.tagName === 'TEXTAREA') &&
            typeof active.select === 'function'
          ) {
            event.preventDefault();
            active.select();
          } else {
            try {
              document.execCommand('selectAll');
              event.preventDefault();
            } catch (_) {}
          }
          break;
        }
        case 'c': {
          const sel = window.getSelection ? String(window.getSelection()) : '';
          if (sel || isEditable(document.activeElement)) {
            try {
              document.execCommand('copy');
            } catch (_) {}
          }
          break;
        }
        case 'x': {
          if (isEditable(document.activeElement)) {
            try {
              document.execCommand('cut');
            } catch (_) {}
          }
          break;
        }
        case 'v': {
          if (isEditable(document.activeElement)) {
            try {
              document.execCommand('paste');
            } catch (_) {}
          }
          break;
        }
        case 'z': {
          if (isEditable(document.activeElement)) {
            try {
              document.execCommand(event.shiftKey ? 'redo' : 'undo');
              event.preventDefault();
            } catch (_) {}
          }
          break;
        }
        case 'y': {
          if (!isMac && isEditable(document.activeElement)) {
            try {
              document.execCommand('redo');
              event.preventDefault();
            } catch (_) {}
          }
          break;
        }
        default:
          break;
      }
    },
    true
  );

  if (!isMac) return;

  const isDragRegion = (event) => {
    const path =
      typeof event.composedPath === 'function'
        ? event.composedPath()
        : [event.target];

    for (const node of path) {
      if (!node || node.nodeType !== 1) continue;
      if (node.matches(NO_DRAG)) return false;
    }

    if (event.clientY <= 60) {
      return true;
    }

    for (const node of path) {
      if (!node || node.nodeType !== 1) continue;
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
      post({ id: 'void-drag', method: 'window.drag' });
    },
    true
  );
})();
