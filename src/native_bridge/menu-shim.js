(() => {
  if (!window.ipc || typeof window.ipc.postMessage !== 'function') return;

  const handlers = new Map();
  const nativeMenu = {
    setMenus(menus) {
      window.ipc.postMessage(JSON.stringify({ id: 'void-menu', menus }));
    },
    onAction(id, handler) {
      if (typeof handler !== 'function') throw new TypeError('Menu action handler must be a function');
      const key = String(id);
      handlers.set(key, handler);
      return () => handlers.delete(key);
    }
  };

  Object.defineProperty(window, 'voidNativeMenu', {
    configurable: false,
    enumerable: false,
    value: Object.freeze(nativeMenu)
  });
  Object.defineProperty(window, '__voidNativeMenuAction', {
    configurable: false,
    enumerable: false,
    value(id) {
      const handler = handlers.get(String(id));
      if (handler) handler();
    }
  });
})();
