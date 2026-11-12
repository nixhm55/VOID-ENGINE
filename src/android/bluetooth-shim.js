(() => {
  if (window.__voidBluetoothShim) return;
  const nativeAndroid = window.VoidNativeBluetooth;
  const pending = new Map();
  let nextId = 1;

  function call(method, args = {}) {
    if (nativeAndroid) {
      const result = nativeAndroid.call(method, JSON.stringify(jsonSafe(args)));
      const reply = JSON.parse(result || '{}');
      if (reply.error) return Promise.reject(makeError(reply.error, reply.name || 'NotFoundError'));
      return Promise.resolve(reply.value);
    }
    if (!window.ipc || typeof window.ipc.postMessage !== 'function') {
      return Promise.reject(makeError('The native Bluetooth bridge is unavailable.'));
    }
    return new Promise((resolve, reject) => {
      const id = String(nextId++);
      pending.set(id, { resolve, reject });
      window.ipc.postMessage(JSON.stringify({ id, method: `bluetooth.${method}`, args: jsonSafe(args) }));
    });
  }

  function makeError(message, name = 'NotFoundError') {
    try { return new DOMException(String(message), name); }
    catch { const error = new Error(String(message)); error.name = name; return error; }
  }

  function jsonSafe(value) {
    if (value instanceof ArrayBuffer) return Array.from(new Uint8Array(value));
    if (ArrayBuffer.isView(value)) return Array.from(new Uint8Array(value.buffer, value.byteOffset, value.byteLength));
    if (Array.isArray(value)) return value.map(jsonSafe);
    if (value && typeof value === 'object') {
      const output = {};
      for (const [key, item] of Object.entries(value)) output[key] = jsonSafe(item);
      return output;
    }
    return value;
  }

  window.__voidBluetoothReply = (reply) => {
    if (!reply || !pending.has(String(reply.id))) return;
    const task = pending.get(String(reply.id));
    pending.delete(String(reply.id));
    if (reply.error) task.reject(makeError(reply.error, reply.name || 'NotFoundError'));
    else task.resolve(reply.value);
  };

  window.__voidBluetoothNotification = (payload) => {
    const characteristic = liveCharacteristics.get(payload.key);
    if (!characteristic) return;
    characteristic.value = dataView(payload.value || []);
    characteristic.dispatchEvent(new Event('characteristicvaluechanged'));
  };

  const liveCharacteristics = new Map();
  const uuidAliases = {
    alert_notification:'1811', battery_service:'180f', blood_pressure:'1810',
    current_time:'1805', cycling_power:'1818', cycling_speed_and_cadence:'1816',
    device_information:'180a', environmental_sensing:'181a', fitness_machine:'1826',
    generic_access:'1800', generic_attribute:'1801', glucose:'1808',
    health_thermometer:'1809', heart_rate:'180d', human_interface_device:'1812',
    location_and_navigation:'1819', phone_alert_status:'180e', pulse_oximeter:'1822',
    scan_parameters:'1813', weight_scale:'181d',
    battery_level:'2a19', blood_pressure_measurement:'2a35', body_sensor_location:'2a38',
    cycling_power_measurement:'2a63', cycling_speed_and_cadence_measurement:'2a5b',
    device_name:'2a00', firmware_revision_string:'2a26', glucose_measurement:'2a18',
    hardware_revision_string:'2a27', heart_rate_control_point:'2a39',
    heart_rate_measurement:'2a37', manufacturer_name_string:'2a29',
    model_number_string:'2a24', serial_number_string:'2a25', software_revision_string:'2a28',
    system_id:'2a23', temperature_measurement:'2a1c', temperature_type:'2a1d'
  };
  const uuid = (value) => {
    if (typeof value !== 'string') throw new TypeError('Bluetooth UUID must be a string.');
    const input = (uuidAliases[value.toLowerCase()] || value).toLowerCase();
    if (/^[0-9a-f]{4}$/.test(input)) return `0000${input}-0000-1000-8000-00805f9b34fb`;
    if (/^[0-9a-f]{8}$/.test(input)) return `${input}-0000-1000-8000-00805f9b34fb`;
    if (/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(input)) return input;
    throw new TypeError(`Invalid Bluetooth UUID: ${value}`);
  };

  function dataView(bytes) {
    const data = new Uint8Array(bytes);
    return new DataView(data.buffer, data.byteOffset, data.byteLength);
  }

  function normalizeOptions(options) {
    const output = jsonSafe(options);
    if (Array.isArray(output.optionalServices)) output.optionalServices = output.optionalServices.map(uuid);
    if (Array.isArray(output.filters)) {
      output.filters = output.filters.map((filter) => ({
        ...filter,
        ...(Array.isArray(filter.services) ? { services: filter.services.map(uuid) } : {}),
        ...(Array.isArray(filter.serviceData) ? { serviceData: filter.serviceData.map((entry) => ({ ...entry, service: uuid(entry.service) })) } : {})
      }));
    }
    return output;
  }

  class VoidBluetoothDevice extends EventTarget {
    constructor(info, allowedServices) {
      super();
      this.id = info.id;
      this.name = info.name || null;
      this._allowedServices = new Set(allowedServices || []);
      this.gatt = new VoidGattServer(this);
    }
    get uuids() { return [...this._allowedServices]; }
    get watchingAdvertisements() { return false; }
    watchAdvertisements() { return Promise.reject(makeError('Advertisement watching is not supported by this build.', 'NotSupportedError')); }
    forget() { return Promise.reject(makeError('forget() is not supported by this build.', 'NotSupportedError')); }
    _disconnect() { this.dispatchEvent(new Event('gattserverdisconnected')); }
  }

  class VoidGattServer {
    constructor(device) { this.device = device; this.connected = false; this._services = new Map(); }
    async connect() {
      const result = await call('connectDevice', { deviceId: this.device.id });
      this.connected = true;
      this._services.clear();
      for (const service of result.services || []) {
        const item = new VoidGattService(this, service);
        this._services.set(item.uuid, item);
      }
      return this;
    }
    disconnect() {
      if (!this.connected) return;
      this.connected = false;
      call('disconnectDevice', { deviceId: this.device.id }).catch(() => {});
      this.device._disconnect();
    }
    async getPrimaryService(serviceUuid) {
      const key = uuid(serviceUuid);
      if (!this.device._allowedServices.has(key)) throw makeError(`Service ${key} was not requested by this origin.`);
      const service = this._services.get(key);
      if (!service) throw makeError(`Bluetooth service ${key} was not found.`);
      return service;
    }
    async getPrimaryServices(serviceUuid) {
      if (serviceUuid !== undefined) return [await this.getPrimaryService(serviceUuid)];
      return [...this._services.values()].filter((service) => this.device._allowedServices.has(service.uuid));
    }
  }

  class VoidGattService {
    constructor(server, info) {
      this.device = server.device;
      this.isPrimary = true;
      this.uuid = uuid(info.uuid);
      this._server = server;
      this._characteristics = new Map();
      for (const item of info.characteristics || []) {
        const characteristic = new VoidGattCharacteristic(this, item);
        this._characteristics.set(characteristic.uuid, characteristic);
      }
    }
    async getCharacteristic(characteristicUuid) {
      const key = uuid(characteristicUuid);
      const characteristic = this._characteristics.get(key);
      if (!characteristic) throw makeError(`Bluetooth characteristic ${key} was not found.`);
      return characteristic;
    }
    async getCharacteristics(characteristicUuid) {
      if (characteristicUuid !== undefined) return [await this.getCharacteristic(characteristicUuid)];
      return [...this._characteristics.values()];
    }
  }

  class VoidGattCharacteristic extends EventTarget {
    constructor(service, info) {
      super();
      this.service = service;
      this.device = service.device;
      this.uuid = uuid(info.uuid);
      this.properties = Object.assign({ broadcast:false, read:false, writeWithoutResponse:false,
        write:false, notify:false, indicate:false, authenticatedSignedWrites:false,
        reliableWrite:false, writableAuxiliaries:false }, info.properties || {});
      this.value = null;
      this._key = `${this.device.id}|${service.uuid}|${this.uuid}`;
      liveCharacteristics.set(this._key, this);
    }
    async readValue() {
      this.value = dataView(await call('readValue', { deviceId:this.device.id, serviceUuid:this.service.uuid, characteristicUuid:this.uuid }));
      return this.value;
    }
    async writeValue(value) { return this.writeValueWithResponse(value); }
    async writeValueWithResponse(value) {
      const bytes = Array.from(new Uint8Array(value.buffer || value, value.byteOffset || 0, value.byteLength));
      await call('writeValue', { deviceId:this.device.id, serviceUuid:this.service.uuid, characteristicUuid:this.uuid, value:bytes, withResponse:true });
    }
    async writeValueWithoutResponse(value) {
      const bytes = Array.from(new Uint8Array(value.buffer || value, value.byteOffset || 0, value.byteLength));
      await call('writeValue', { deviceId:this.device.id, serviceUuid:this.service.uuid, characteristicUuid:this.uuid, value:bytes, withResponse:false });
    }
    async startNotifications() {
      await call('startNotifications', { deviceId:this.device.id, serviceUuid:this.service.uuid, characteristicUuid:this.uuid, key:this._key });
      return this;
    }
    async stopNotifications() {
      await call('stopNotifications', { deviceId:this.device.id, serviceUuid:this.service.uuid, characteristicUuid:this.uuid, key:this._key });
      return this;
    }
  }

  const bluetooth = {
    async getAvailability() { return !!(await call('getAvailability')); },
    async getDevices() {
      const devices = await call('getDevices');
      return (devices || []).map((device) => new VoidBluetoothDevice(device, device.services || []));
    },
    async requestDevice(options = {}) {
      if (!options.acceptAllDevices && (!Array.isArray(options.filters) || !options.filters.length)) {
        throw new TypeError('requestDevice requires filters or acceptAllDevices: true.');
      }
      const normalizedOptions = normalizeOptions(options);
      const info = await call('requestDevice', { options: normalizedOptions });
      const allowed = new Set(normalizedOptions.optionalServices || []);
      for (const filter of normalizedOptions.filters || []) for (const service of filter.services || []) allowed.add(service);
      return new VoidBluetoothDevice(info, [...allowed]);
    },
    addEventListener() {}, removeEventListener() {}, dispatchEvent() { return false; }
  };

  try { Object.defineProperty(navigator, 'bluetooth', { configurable:true, value:bluetooth }); }
  catch { navigator.bluetooth = bluetooth; }
  window.__voidBluetoothShim = true;
})();
