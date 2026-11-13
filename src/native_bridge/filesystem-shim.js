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

  class VoidFileSystemHandle {
    constructor(id, path, name, kind) {
      this._id = id;
      this._path = path;
      this.name = name;
      this.kind = kind;
    }
    async isSameEntry(other) {
      return other instanceof VoidFileSystemHandle && other._id === this._id &&
        JSON.stringify(other._path) === JSON.stringify(this._path);
    }
    async queryPermission(descriptor = {}) {
      return descriptor.mode === 'readwrite' ? 'denied' : 'granted';
    }
    async requestPermission(descriptor = {}) {
      return this.queryPermission(descriptor);
    }
    get [Symbol.toStringTag]() { return 'FileSystemHandle'; }
  }

  class VoidFileSystemFileHandle extends VoidFileSystemHandle {
    constructor(id, path, name) { super(id, path, name, 'file'); }
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
      throw makeError('Writing through this folder connection is not supported.', 'NotSupportedError');
    }
    get [Symbol.toStringTag]() { return 'FileSystemFileHandle'; }
  }

  class VoidFileSystemDirectoryHandle extends VoidFileSystemHandle {
    constructor(id, path, name) { super(id, path, name, 'directory'); }
    async _entries() {
      return call('filesystem.listDirectory', { id: this._id, path: this._path });
    }
    async getDirectoryHandle(name, options = {}) {
      validateName(name);
      if (options.create) throw makeError('Creating folders through this connection is not supported.', 'NotSupportedError');
      const entry = (await this._entries()).find((item) => item.name === name);
      if (!entry) throw makeError(`Folder not found: ${name}`, 'NotFoundError');
      if (entry.kind !== 'directory') throw makeError(`${name} is not a folder.`, 'TypeMismatchError');
      return new VoidFileSystemDirectoryHandle(this._id, [...this._path, name], name);
    }
    async getFileHandle(name, options = {}) {
      validateName(name);
      if (options.create) throw makeError('Creating files through this connection is not supported.', 'NotSupportedError');
      const entry = (await this._entries()).find((item) => item.name === name);
      if (!entry) throw makeError(`File not found: ${name}`, 'NotFoundError');
      if (entry.kind !== 'file') throw makeError(`${name} is not a file.`, 'TypeMismatchError');
      return new VoidFileSystemFileHandle(this._id, [...this._path, name], name);
    }
    async *entries() {
      for (const entry of await this._entries()) {
        const path = [...this._path, entry.name];
        const handle = entry.kind === 'directory'
          ? new VoidFileSystemDirectoryHandle(this._id, path, entry.name)
          : new VoidFileSystemFileHandle(this._id, path, entry.name);
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
    async removeEntry() {
      throw makeError('Removing entries through this folder connection is not supported.', 'NotSupportedError');
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
    if (options.mode && options.mode !== 'read') {
      throw makeError('This folder connection supports read-only access.', 'NotSupportedError');
    }
    if (navigator.userActivation && !navigator.userActivation.isActive) {
      throw makeError('Choose a folder from a user action.', 'SecurityError');
    }
    const selected = await call('filesystem.pickDirectory');
    return new VoidFileSystemDirectoryHandle(selected.id, [], selected.name);
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
