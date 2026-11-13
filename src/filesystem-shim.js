(() => {
  if (window.__voidFilesystemShimInstalled) return;
  window.__voidFilesystemShimInstalled = true;

  // 1. Expose native desktop WebView identity
  try {
    Object.defineProperty(window, 'isNativeDesktop', {
      value: true,
      writable: false,
      configurable: true,
      enumerable: true,
    });
  } catch (_) {
    window.isNativeDesktop = true;
  }
  window.__VOID_NATIVE_DESKTOP__ = true;

  // 2. Desktop User-Agent and Navigator Client Hints override
  const ua = navigator.userAgent || '';
  const isMac = /Macintosh|Mac OS X/i.test(ua);
  const isWin = /Windows/i.test(ua);
  const desktopUa = isMac
    ? 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36'
    : isWin
      ? 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36'
      : 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36';

  const platformName = isMac ? 'macOS' : isWin ? 'Windows' : 'Linux';
  const legacyPlatform = isMac ? 'MacIntel' : isWin ? 'Win32' : 'Linux x86_64';

  try {
    Object.defineProperty(Navigator.prototype, 'userAgent', {
      get: () => desktopUa,
      configurable: true,
    });
    Object.defineProperty(Navigator.prototype, 'appVersion', {
      get: () => desktopUa.replace(/^Mozilla\//, ''),
      configurable: true,
    });
    Object.defineProperty(Navigator.prototype, 'platform', {
      get: () => legacyPlatform,
      configurable: true,
    });
    Object.defineProperty(Navigator.prototype, 'vendor', {
      get: () => 'Google Inc.',
      configurable: true,
    });
    Object.defineProperty(Navigator.prototype, 'webdriver', {
      get: () => false,
      configurable: true,
    });
    if (!('userAgentData' in navigator)) {
      const brands = [
        { brand: 'Chromium', version: '131' },
        { brand: 'Google Chrome', version: '131' },
        { brand: 'Not_A Brand', version: '24' },
      ];
      Object.defineProperty(Navigator.prototype, 'userAgentData', {
        get: () => ({
          brands,
          mobile: false,
          platform: platformName,
          getHighEntropyValues: async () => ({
            architecture: 'x86',
            bitness: '64',
            brands,
            fullVersionList: brands,
            mobile: false,
            model: '',
            platform: platformName,
            platformVersion: '15.0.0',
            uaFullVersion: '131.0.0.0',
          }),
          toJSON: () => ({ brands, mobile: false, platform: platformName }),
        }),
        configurable: true,
      });
    }
  } catch (_) {}

  // 3. File System Access API & Native Bridge Shim
  const wrapFileAsHandle = (file) => ({
    kind: 'file',
    name: file.name,
    getFile: async () => file,
    queryPermission: async () => 'granted',
    requestPermission: async () => 'granted',
    createWritable: async () => {
      const chunks = [];
      return {
        write: async (data) => {
          if (data && typeof data === 'object' && 'data' in data) {
            chunks.push(data.data);
          } else {
            chunks.push(data);
          }
        },
        close: async () => {
          const blob = new Blob(chunks, { type: file.type || 'application/octet-stream' });
          const url = URL.createObjectURL(blob);
          const a = document.createElement('a');
          a.href = url;
          a.download = file.name || 'download';
          document.body.appendChild(a);
          a.click();
          a.remove();
          setTimeout(() => URL.revokeObjectURL(url), 1000);
        },
      };
    },
  });

  const pickFilesViaInput = ({ multiple = false, directory = false, accept = '' } = {}) =>
    new Promise((resolve, reject) => {
      const input = document.createElement('input');
      input.type = 'file';
      input.multiple = Boolean(multiple);
      if (directory) {
        input.webkitdirectory = true;
        input.directory = true;
      }
      if (accept) input.accept = accept;
      input.style.display = 'none';
      document.addEventListener(
        'focus',
        () => {
          setTimeout(() => {
            if (!input.files || input.files.length === 0) {
              reject(new DOMException('The user aborted a request.', 'AbortError'));
            }
          }, 500);
        },
        { once: true }
      );
      input.addEventListener('change', () => {
        const files = Array.from(input.files || []);
        input.remove();
        if (!files.length) {
          reject(new DOMException('The user aborted a request.', 'AbortError'));
          return;
        }
        resolve(files);
      });
      (document.body || document.documentElement).appendChild(input);
      input.click();
    });

  if (typeof window.showOpenFilePicker !== 'function') {
    window.showOpenFilePicker = async (options = {}) => {
      const acceptList = [];
      if (Array.isArray(options.types)) {
        for (const t of options.types) {
          if (t && t.accept && typeof t.accept === 'object') {
            for (const [mime, exts] of Object.entries(t.accept)) {
              acceptList.push(mime);
              if (Array.isArray(exts)) acceptList.push(...exts);
            }
          }
        }
      }
      const files = await pickFilesViaInput({
        multiple: Boolean(options.multiple),
        accept: acceptList.join(','),
      });
      return files.map(wrapFileAsHandle);
    };
  }

  if (typeof window.showSaveFilePicker !== 'function') {
    window.showSaveFilePicker = async (options = {}) => {
      const suggestedName = options.suggestedName || 'untitled';
      const emptyFile = new File([], suggestedName, { type: 'application/octet-stream' });
      return wrapFileAsHandle(emptyFile);
    };
  }

  if (typeof window.showDirectoryPicker !== 'function') {
    window.showDirectoryPicker = async () => {
      const files = await pickFilesViaInput({ multiple: true, directory: true });
      const dirName =
        (files[0] && files[0].webkitRelativePath && files[0].webkitRelativePath.split('/')[0]) ||
        'selected-folder';
      return {
        kind: 'directory',
        name: dirName,
        queryPermission: async () => 'granted',
        requestPermission: async () => 'granted',
        values: async function* () {
          for (const file of files) {
            yield wrapFileAsHandle(file);
          }
        },
        entries: async function* () {
          for (const file of files) {
            yield [file.name, wrapFileAsHandle(file)];
          }
        },
      };
    };
  }
})();
