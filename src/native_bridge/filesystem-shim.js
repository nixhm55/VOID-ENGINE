(() => {
  if (typeof window.showDirectoryPicker === 'function') return;
  if (!window.ipc || typeof window.ipc.postMessage !== 'function') return;

  let nextId = 1;
  const pending = new Map();

  function makeError(message, name = 'NotReadableError') {
    try { return new DOMException(String(message), name); }
    catch { const error = new Error(String(message)); error.name = name; return error; }
  }

  function call(method, args = {}) {
    return new Promise((resolve, reject) => {
      const id = String(nextId++);
      pending.set(id, { resolve, reject });
      window.ipc.postMessage(JSON.stringify({ id, method, args }));
    });
  }

  window.__voidFileSystemReply = (reply) => {
    if (!reply || !pending.has(String(reply.id))) return;
    const task = pending.get(String(reply.id));
    pending.delete(String(reply.id));
    if (reply.error) task.reject(makeError(reply.error, reply.name || 'NotReadableError'));
    else task.resolve(reply.value);
  };

  function toBase64(bytes) {
    let binary = '';
    const step = 0x8000;
    for (let index = 0; index < bytes.length; index += step) {
      binary += String.fromCharCode.apply(null, bytes.subarray(index, index + step));
    }
    return btoa(binary);
  }

  // Writable streams buffer in the page and submit once on close, so a native
  // approval covers the whole file instead of every chunk.
  async function encodeChunks(chunks) {
    const parts = chunks.map((chunk) => {
      if (chunk && typeof chunk === 'object' && 'data' in chunk) return chunk.data;
      return chunk;
    });
    const blob = new Blob(parts);
    return toBase64(new Uint8Array(await blob.arrayBuffer()));
  }

  class VoidFileSystemHandle {
    constructor(id, path, name, kind, mode) {
      this._id = id;
      this._path = path;
      this.name = name;
      this.kind = kind;
      this._mode = mode === 'readwrite' ? 'readwrite' : 'read';
    }
    async isSameEntry(other) {
      return other instanceof VoidFileSystemHandle && other._id === this._id &&
        JSON.stringify(other._path) === JSON.stringify(this._path);
    }
    async queryPermission(descriptor = {}) {
      return descriptor.mode === 'readwrite' && this._mode !== 'readwrite' ? 'prompt' : 'granted';
    }
    async requestPermission(descriptor = {}) {
      if (descriptor.mode !== 'readwrite' || this._mode === 'readwrite') return 'granted';
      // Layer 1 runs again before a read-only grant is widened.
      const grant = await call('filesystem.requestWriteAccess', { id: this._id });
      if (grant && grant.mode === 'readwrite') {
        this._mode = 'readwrite';
        return 'granted';
      }
      return 'denied';
    }
    get [Symbol.toStringTag]() { return 'FileSystemHandle'; }
  }

  class VoidFileSystemFileHandle extends VoidFileSystemHandle {
    constructor(id, path, name, mode) { super(id, path, name, 'file', mode); }
    async getFile() {
      const result = await call('filesystem.readFile', { id: this._id, path: this._path });
      const binary = atob(result.bytes);
      const bytes = new Uint8Array(binary.length);
      for (let index = 0; index < binary.length; index++) bytes[index] = binary.charCodeAt(index);
      return new File([bytes], this.name, {
        type: mimeType(this.name),
        lastModified: Number(result.lastModified) || 0,
      });
    }
    async createWritable() {
      const id = this._id;
      const path = this._path;
      const chunks = [];
      let closed = false;
      const push = (data) => {
        if (closed) throw makeError('The writer is closed.', 'InvalidStateError');
        chunks.push(data);
      };
      return {
        async write(data) { push(data); },
        async truncate(size = 0) {
          if (Number(size) !== 0) {
            throw makeError('Only truncating to zero bytes is supported.', 'NotSupportedError');
          }
          chunks.length = 0;
        },
        async seek() {
          throw makeError('Seeking is not supported by this folder connection.', 'NotSupportedError');
        },
        async close() {
          if (closed) return;
          closed = true;
          const bytes = await encodeChunks(chunks);
          await call('filesystem.writeFile', { id, path, bytes });
        },
        async abort() {
          closed = true;
          chunks.length = 0;
        },
      };
    }
    get [Symbol.toStringTag]() { return 'FileSystemFileHandle'; }
  }

  class VoidFileSystemDirectoryHandle extends VoidFileSystemHandle {
    constructor(id, path, name, mode) { super(id, path, name, 'directory', mode); }
    async _entries() {
      return call('filesystem.listDirectory', { id: this._id, path: this._path });
    }
    async getDirectoryHandle(name, options = {}) {
      validateName(name);
      if (options.create) {
        await call('filesystem.createDirectory', { id: this._id, path: [...this._path, name] });
        return new VoidFileSystemDirectoryHandle(this._id, [...this._path, name], name, this._mode);
      }
      const entry = (await this._entries()).find((item) => item.name === name);
      if (!entry) throw makeError(`Folder not found: ${name}`, 'NotFoundError');
      if (entry.kind !== 'directory') throw makeError(`${name} is not a folder.`, 'TypeMismatchError');
      return new VoidFileSystemDirectoryHandle(this._id, [...this._path, name], name, this._mode);
    }
    async getFileHandle(name, options = {}) {
      validateName(name);
      // With create the file is materialised on the first write, which keeps
      // one approval per change instead of one per lookup.
      if (options.create) {
        return new VoidFileSystemFileHandle(this._id, [...this._path, name], name, this._mode);
      }
      const entry = (await this._entries()).find((item) => item.name === name);
      if (!entry) throw makeError(`File not found: ${name}`, 'NotFoundError');
      if (entry.kind !== 'file') throw makeError(`${name} is not a file.`, 'TypeMismatchError');
      return new VoidFileSystemFileHandle(this._id, [...this._path, name], name, this._mode);
    }
    async *entries() {
      for (const entry of await this._entries()) {
        const path = [...this._path, entry.name];
        const handle = entry.kind === 'directory'
          ? new VoidFileSystemDirectoryHandle(this._id, path, entry.name, this._mode)
          : new VoidFileSystemFileHandle(this._id, path, entry.name, this._mode);
        yield [entry.name, handle];
      }
    }
    async *keys() {
      for (const entry of await this._entries()) yield entry.name;
    }
    async *values() {
      for await (const [, handle] of this.entries()) yield handle;
    }
    [Symbol.asyncIterator]() { return this.entries(); }
    async removeEntry(name, options = {}) {
      validateName(name);
      if (options.recursive) {
        throw makeError(
          'Recursive removal is not supported through this folder connection.',
          'NotSupportedError'
        );
      }
      await call('filesystem.removeEntry', { id: this._id, path: [...this._path, name] });
    }
    async resolve(possibleDescendant) {
      if (!(possibleDescendant instanceof VoidFileSystemHandle) || possibleDescendant._id !== this._id) return null;
      const prefix = this._path;
      if (possibleDescendant._path.length < prefix.length ||
          !prefix.every((part, index) => possibleDescendant._path[index] === part)) return null;
      return possibleDescendant._path.slice(prefix.length);
    }
    get [Symbol.toStringTag]() { return 'FileSystemDirectoryHandle'; }
  }

  function validateName(name) {
    if (typeof name !== 'string' || !name || name === '.' || name === '..' || /[\\/\0]/.test(name)) {
      throw new TypeError('A single valid file or folder name is required.');
    }
  }

  function mimeType(name) {
    const extension = name.split('.').pop().toLowerCase();
    return ({
      csv: 'text/csv', html: 'text/html', htm: 'text/html', js: 'text/javascript',
      json: 'application/json', md: 'text/markdown', pdf: 'application/pdf',
      txt: 'text/plain', xml: 'application/xml', yaml: 'text/yaml', yml: 'text/yaml',
    })[extension] || '';
  }

  async function showDirectoryPicker(options = {}) {
    if (navigator.userActivation && !navigator.userActivation.isActive) {
      throw makeError('Choose a folder from a user action.', 'SecurityError');
    }
    const mode = options.mode === 'readwrite' ? 'readwrite' : 'read';
    const selected = await call('filesystem.pickDirectory', { mode });
    return new VoidFileSystemDirectoryHandle(
      selected.id,
      [],
      selected.name,
      selected.mode || mode
    );
  }

  Object.defineProperty(window, 'showDirectoryPicker', {
    configurable: true,
    writable: true,
    value: showDirectoryPicker,
  });
  if (typeof window.FileSystemHandle === 'undefined') window.FileSystemHandle = VoidFileSystemHandle;
  if (typeof window.FileSystemDirectoryHandle === 'undefined') window.FileSystemDirectoryHandle = VoidFileSystemDirectoryHandle;
  if (typeof window.FileSystemFileHandle === 'undefined') window.FileSystemFileHandle = VoidFileSystemFileHandle;
})();
