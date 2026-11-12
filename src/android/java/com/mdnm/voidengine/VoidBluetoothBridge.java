package com.mdnm.voidengine;

import android.Manifest;
import android.app.AlertDialog;
import android.bluetooth.BluetoothAdapter;
import android.bluetooth.BluetoothDevice;
import android.bluetooth.BluetoothGatt;
import android.bluetooth.BluetoothGattCallback;
import android.bluetooth.BluetoothGattCharacteristic;
import android.bluetooth.BluetoothGattDescriptor;
import android.bluetooth.BluetoothGattService;
import android.bluetooth.BluetoothManager;
import android.bluetooth.BluetoothProfile;
import android.bluetooth.le.BluetoothLeScanner;
import android.bluetooth.le.ScanCallback;
import android.bluetooth.le.ScanRecord;
import android.bluetooth.le.ScanResult;
import android.content.Context;
import android.content.DialogInterface;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Build;
import android.webkit.JavascriptInterface;
import android.webkit.WebView;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;

/** Android BLE central bridge used by the injected Web Bluetooth facade. */
final class VoidBluetoothBridge implements VoidActivity.BluetoothPermissionReceiver {
    static final int PERMISSION_REQUEST = 0x5648;
    private static final String PREFS = "void_bluetooth_permissions";
    private static final int SCAN_SECONDS = 8;
    private static final long OP_TIMEOUT_SECONDS = 20;
    private static final UUID CCCD = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb");

    private final VoidActivity activity;
    private final WebView webView;
    private final Object permissionLock = new Object();
    private final Set<String> sessionOrigins = new HashSet<>();
    private final Map<String, BluetoothDevice> discovered = new HashMap<>();
    private final Map<String, String> deviceOrigins = new HashMap<>();
    private final Map<String, Set<UUID>> deviceAllowedServices = new HashMap<>();
    private final Map<String, GattSession> sessions = new HashMap<>();
    private final Map<String, String> sessionOriginsByDevice = new HashMap<>();
    private CountDownLatch runtimePermissionLatch;
    private boolean runtimePermissionGranted;

    VoidBluetoothBridge(VoidActivity activity, WebView webView) {
        this.activity = activity;
        this.webView = webView;
    }

    @JavascriptInterface
    public String call(String method, String rawArguments) {
        try {
            String origin = currentSecureOrigin();
            JSONObject args = new JSONObject(rawArguments == null ? "{}" : rawArguments);
            Object value;
            switch (method) {
                case "getAvailability":
                    value = isBluetoothAvailable();
                    break;
                case "requestDevice":
                    value = requestDevice(origin, args.optJSONObject("options"));
                    break;
                case "getDevices":
                    value = getDevices(origin);
                    break;
                case "connectDevice":
                    value = connectDevice(origin, args.optString("deviceId"));
                    break;
                case "disconnectDevice":
                    disconnectDevice(origin, args.optString("deviceId"));
                    value = JSONObject.NULL;
                    break;
                case "readValue":
                    value = readValue(origin, args);
                    break;
                case "writeValue":
                    writeValue(origin, args);
                    value = JSONObject.NULL;
                    break;
                case "startNotifications":
                    startNotifications(origin, args);
                    value = JSONObject.NULL;
                    break;
                case "stopNotifications":
                    stopNotifications(origin, args);
                    value = JSONObject.NULL;
                    break;
                default:
                    throw new BridgeException("Unsupported Bluetooth operation: " + method,
                            "NotSupportedError");
            }
            JSONObject response = new JSONObject();
            response.put("value", value == null ? JSONObject.NULL : value);
            return response.toString();
        } catch (BridgeException error) {
            return errorResponse(error.getMessage(), error.name);
        } catch (SecurityException error) {
            return errorResponse("Android denied Bluetooth access. Check the app's Bluetooth permissions.",
                    "NotAllowedError");
        } catch (Exception error) {
            String message = error.getMessage();
            return errorResponse(message == null ? "Bluetooth operation failed." : message,
                    "NetworkError");
        }
    }

    @Override public void onRuntimePermissionResult(int[] grantResults) {
        synchronized (permissionLock) {
            runtimePermissionGranted = grantResults.length > 0;
            for (int result : grantResults) {
                if (result != PackageManager.PERMISSION_GRANTED) runtimePermissionGranted = false;
            }
            if (runtimePermissionLatch != null) runtimePermissionLatch.countDown();
        }
    }

    @Override public void closeBridge() {
        synchronized (permissionLock) {
            if (runtimePermissionLatch != null) runtimePermissionLatch.countDown();
            runtimePermissionLatch = null;
        }
        synchronized (sessions) {
            for (GattSession session : sessions.values()) {
                if (session.gatt != null) {
                    try { session.gatt.disconnect(); session.gatt.close(); } catch (RuntimeException ignored) { }
                }
            }
            sessions.clear();
        }
        synchronized (sessionOriginsByDevice) { sessionOriginsByDevice.clear(); }
        synchronized (deviceOrigins) { deviceOrigins.clear(); }
        synchronized (discovered) { discovered.clear(); }
        synchronized (deviceAllowedServices) { deviceAllowedServices.clear(); }
    }

    private boolean isBluetoothAvailable() {
        BluetoothAdapter adapter = adapter();
        if (adapter == null) return false;
        if (Build.VERSION.SDK_INT >= 31
                && activity.checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT)
                    != PackageManager.PERMISSION_GRANTED) return true;
        return adapter.isEnabled();
    }

    private synchronized JSONObject requestDevice(String origin, JSONObject options) throws Exception {
        if (options == null) options = new JSONObject();
        if (!options.optBoolean("acceptAllDevices")
                && (options.optJSONArray("filters") == null || options.optJSONArray("filters").length() == 0)) {
            throw new BridgeException("Provide filters or set acceptAllDevices to true.", "TypeError");
        }
        if (options.optBoolean("acceptAllDevices") && options.has("filters")) {
            throw new BridgeException("acceptAllDevices cannot be used together with filters.", "TypeError");
        }
        if (!ensureOriginPermission(origin)) {
            throw new BridgeException("Bluetooth access was denied.", "NotAllowedError");
        }
        if (!ensureAndroidPermissions()) {
            throw new BridgeException("Android Bluetooth permissions were denied.", "NotAllowedError");
        }
        BluetoothAdapter adapter = adapter();
        if (adapter == null || !adapter.isEnabled()) {
            throw new BridgeException("Turn on Bluetooth in Android settings and try again.", "NotFoundError");
        }
        BluetoothLeScanner scanner = adapter.getBluetoothLeScanner();
        if (scanner == null) throw new BridgeException("This device has no Bluetooth LE scanner.", "NotFoundError");

        final CountDownLatch scanFinished = new CountDownLatch(1);
        final Map<String, BluetoothDevice> scanResults = new HashMap<>();
        synchronized (scanServiceUuids) {
            scanServiceUuids.clear();
            scanRecords.clear();
        }
        final ScanCallback callback = new ScanCallback() {
            @Override public void onScanResult(int callbackType, ScanResult result) {
                addScanResult(scanResults, result);
            }
            @Override public void onBatchScanResults(List<ScanResult> results) {
                for (ScanResult result : results) addScanResult(scanResults, result);
            }
            @Override public void onScanFailed(int errorCode) { scanFinished.countDown(); }
        };

        try {
            scanner.startScan(callback);
            scanFinished.await(SCAN_SECONDS, TimeUnit.SECONDS);
        } finally {
            try { scanner.stopScan(callback); } catch (RuntimeException ignored) { }
        }
        ArrayList<BluetoothDevice> matches = new ArrayList<>();
        ArrayList<BluetoothDevice> scanned;
        synchronized (scanResults) { scanned = new ArrayList<>(scanResults.values()); }
        for (BluetoothDevice device : scanned) {
            if (matchesFilters(device, options)) matches.add(device);
        }
        if (matches.isEmpty()) throw new BridgeException("No matching Bluetooth LE devices were found.", "NotFoundError");
        BluetoothDevice selected = chooseDevice(matches);
        if (selected == null) throw new BridgeException("No Bluetooth device was selected.", "NotFoundError");
        String id = UUID.randomUUID().toString();
        synchronized (discovered) { discovered.put(id, selected); }
        synchronized (deviceOrigins) { deviceOrigins.put(id, origin); }
        synchronized (deviceAllowedServices) { deviceAllowedServices.put(id, requestedServices(options)); }
        JSONObject result = new JSONObject();
        result.put("id", id);
        String name = selected.getName();
        result.put("name", name == null || name.isEmpty() ? JSONObject.NULL : name);
        return result;
    }

    private JSONArray getDevices(String origin) throws Exception {
        android.content.SharedPreferences preferences = activity.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        boolean allowed = preferences.getBoolean(origin, false);
        synchronized (sessionOrigins) { allowed |= sessionOrigins.contains(origin); }
        JSONArray result = new JSONArray();
        if (!allowed) return result;
        synchronized (discovered) {
            synchronized (deviceOrigins) {
                synchronized (deviceAllowedServices) {
                    for (Map.Entry<String, BluetoothDevice> entry : discovered.entrySet()) {
                        String id = entry.getKey();
                        if (!origin.equals(deviceOrigins.get(id))) continue;
                        JSONObject item = new JSONObject();
                        item.put("id", id);
                        String name = entry.getValue().getName();
                        item.put("name", name == null || name.isEmpty() ? JSONObject.NULL : name);
                        JSONArray services = new JSONArray();
                        for (UUID service : deviceAllowedServices.getOrDefault(id, new HashSet<UUID>())) {
                            services.put(service.toString());
                        }
                        item.put("services", services);
                        result.put(item);
                    }
                }
            }
        }
        return result;
    }

    private Set<UUID> requestedServices(JSONObject options) {
        Set<UUID> allowed = new HashSet<>();
        JSONArray optional = options.optJSONArray("optionalServices");
        if (optional != null) {
            for (int index = 0; index < optional.length(); index++) {
                try { allowed.add(normalizeUuid(optional.getString(index))); } catch (Exception ignored) { }
            }
        }
        JSONArray filters = options.optJSONArray("filters");
        if (filters != null) {
            for (int index = 0; index < filters.length(); index++) {
                JSONObject filter = filters.optJSONObject(index);
                JSONArray services = filter == null ? null : filter.optJSONArray("services");
                if (services == null) continue;
                for (int serviceIndex = 0; serviceIndex < services.length(); serviceIndex++) {
                    try { allowed.add(normalizeUuid(services.getString(serviceIndex))); } catch (Exception ignored) { }
                }
            }
        }
        return allowed;
    }

    private void addScanResult(Map<String, BluetoothDevice> results, ScanResult result) {
        BluetoothDevice device = result == null ? null : result.getDevice();
        if (device == null) return;
        synchronized (results) { results.put(device.getAddress(), device); }
        if (result.getScanRecord() != null && result.getScanRecord().getServiceUuids() != null) {
            Set<UUID> advertised = new HashSet<>();
            for (android.os.ParcelUuid uuid : result.getScanRecord().getServiceUuids()) advertised.add(uuid.getUuid());
            synchronized (scanServiceUuids) { scanServiceUuids.put(device.getAddress(), advertised); }
        }
        if (result.getScanRecord() != null) {
            synchronized (scanServiceUuids) { scanRecords.put(device.getAddress(), result.getScanRecord()); }
        }
    }

    private boolean matchesFilters(BluetoothDevice device, JSONObject options) {
        if (options.optBoolean("acceptAllDevices")) return true;
        JSONArray filters = options.optJSONArray("filters");
        if (filters == null) return false;
        for (int index = 0; index < filters.length(); index++) {
            JSONObject filter = filters.optJSONObject(index);
            if (filter == null) continue;
            boolean matches = true;
            boolean hasConstraint = false;
            String name = device.getName();
            synchronized (scanServiceUuids) {
                ScanRecord record = scanRecords.get(device.getAddress());
                if (record != null && record.getDeviceName() != null) name = record.getDeviceName();
            }
            if (filter.has("name") && !filter.optString("name").equals(name)) matches = false;
            if (filter.has("name")) hasConstraint = true;
            if (filter.has("namePrefix")) {
                hasConstraint = true;
                if (name == null || !name.startsWith(filter.optString("namePrefix"))) matches = false;
            }
            JSONArray services = filter.optJSONArray("services");
            if (services != null && services.length() > 0) {
                hasConstraint = true;
                boolean found = false;
                try {
                    for (int serviceIndex = 0; serviceIndex < services.length(); serviceIndex++) {
                        UUID wanted = normalizeUuid(services.getString(serviceIndex));
                        if (recordHasService(device, wanted)) { found = true; break; }
                    }
                } catch (Exception ignored) { }
                if (!found) matches = false;
            }
            JSONArray manufacturers = filter.optJSONArray("manufacturerData");
            if (manufacturers != null) {
                if (manufacturers.length() > 0) hasConstraint = true;
                for (int item = 0; item < manufacturers.length(); item++) {
                    JSONObject entry = manufacturers.optJSONObject(item);
                    if (entry == null || !matchesManufacturerData(device, entry)) { matches = false; break; }
                }
            }
            JSONArray serviceData = filter.optJSONArray("serviceData");
            if (serviceData != null) {
                if (serviceData.length() > 0) hasConstraint = true;
                for (int item = 0; item < serviceData.length(); item++) {
                    JSONObject entry = serviceData.optJSONObject(item);
                    if (entry == null || !matchesServiceData(device, entry)) { matches = false; break; }
                }
            }
            if (matches && hasConstraint) return true;
        }
        return false;
    }

    private boolean matchesManufacturerData(BluetoothDevice device, JSONObject entry) {
        int companyId = entry.optInt("companyIdentifier", -1);
        if (companyId < 0) return false;
        byte[] advertised;
        synchronized (scanServiceUuids) {
            ScanRecord record = scanRecords.get(device.getAddress());
            advertised = record == null ? null : record.getManufacturerSpecificData(companyId);
        }
        return matchesPrefix(advertised, entry.optJSONArray("dataPrefix"));
    }

    private boolean matchesServiceData(BluetoothDevice device, JSONObject entry) {
        UUID service;
        try { service = normalizeUuid(entry.getString("service")); }
        catch (Exception error) { return false; }
        byte[] advertised;
        synchronized (scanServiceUuids) {
            ScanRecord record = scanRecords.get(device.getAddress());
            advertised = record == null ? null : record.getServiceData(new android.os.ParcelUuid(service));
        }
        return matchesPrefix(advertised, entry.optJSONArray("dataPrefix"));
    }

    private boolean matchesPrefix(byte[] advertised, JSONArray prefix) {
        if (advertised == null) return false;
        if (prefix == null) return true;
        if (advertised.length < prefix.length()) return false;
        for (int index = 0; index < prefix.length(); index++) {
            if ((advertised[index] & 0xff) != prefix.optInt(index, -1)) return false;
        }
        return true;
    }

    private boolean recordHasService(BluetoothDevice device, UUID service) {
        // Android's cached scan record is not exposed by BluetoothDevice. Scan filters are
        // evaluated again from the most recent ScanResult below via its service UUID list.
        synchronized (scanServiceUuids) {
            Set<UUID> advertised = scanServiceUuids.get(device.getAddress());
            return advertised != null && advertised.contains(service);
        }
    }

    private final Map<String, Set<UUID>> scanServiceUuids = new HashMap<>();
    private final Map<String, ScanRecord> scanRecords = new HashMap<>();

    private UUID normalizeUuid(String value) {
        String input = value.toLowerCase(Locale.ROOT);
        if (input.matches("[0-9a-f]{4}")) input = "0000" + input + "-0000-1000-8000-00805f9b34fb";
        else if (input.matches("[0-9a-f]{8}")) input += "-0000-1000-8000-00805f9b34fb";
        return UUID.fromString(input);
    }

    private BluetoothDevice chooseDevice(final List<BluetoothDevice> devices)
            throws InterruptedException, BridgeException {
        final CountDownLatch chosen = new CountDownLatch(1);
        final BluetoothDevice[] result = new BluetoothDevice[1];
        activity.runOnUiThread(new Runnable() {
            @Override public void run() {
                ArrayList<String> labels = new ArrayList<>();
                for (BluetoothDevice device : devices) {
                    String name = device.getName();
                    labels.add((name == null || name.isEmpty() ? "Unknown device" : name)
                            + "  ·  " + device.getAddress());
                }
                new AlertDialog.Builder(activity)
                        .setTitle("Choose a Bluetooth device")
                        .setItems(labels.toArray(new String[0]), new DialogInterface.OnClickListener() {
                            @Override public void onClick(DialogInterface dialog, int which) {
                                if (which >= 0 && which < devices.size()) result[0] = devices.get(which);
                                chosen.countDown();
                            }
                        })
                        .setOnCancelListener(new DialogInterface.OnCancelListener() {
                            @Override public void onCancel(DialogInterface dialog) { chosen.countDown(); }
                        })
                        .show();
            }
        });
        if (!chosen.await(5, TimeUnit.MINUTES)) throw new BridgeException("Bluetooth device selection timed out.", "NotAllowedError");
        return result[0];
    }

    private boolean ensureOriginPermission(final String origin) throws InterruptedException {
        android.content.SharedPreferences preferences = activity.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        if (preferences.getBoolean(origin, false)) return true;
        synchronized (sessionOrigins) { if (sessionOrigins.contains(origin)) return true; }
        final CountDownLatch answered = new CountDownLatch(1);
        final int[] choice = new int[]{0};
        activity.runOnUiThread(new Runnable() {
            @Override public void run() {
                new AlertDialog.Builder(activity)
                        .setTitle("Bluetooth access")
                        .setMessage(origin + " wants to scan for and connect to nearby Bluetooth devices.")
                        .setPositiveButton("Allow Always", new DialogInterface.OnClickListener() {
                            @Override public void onClick(DialogInterface dialog, int which) {
                                choice[0] = 1;
                                answered.countDown();
                            }
                        })
                        .setNeutralButton("Allow This Time Only", new DialogInterface.OnClickListener() {
                            @Override public void onClick(DialogInterface dialog, int which) {
                                choice[0] = 2;
                                answered.countDown();
                            }
                        })
                        .setNegativeButton("Don't Allow", new DialogInterface.OnClickListener() {
                            @Override public void onClick(DialogInterface dialog, int which) {
                                choice[0] = 0;
                                answered.countDown();
                            }
                        })
                        .setOnCancelListener(new DialogInterface.OnCancelListener() {
                            @Override public void onCancel(DialogInterface dialog) { answered.countDown(); }
                        })
                        .show();
            }
        });
        if (!answered.await(5, TimeUnit.MINUTES)) return false;
        if (choice[0] == 1) preferences.edit().putBoolean(origin, true).apply();
        else if (choice[0] == 2) synchronized (sessionOrigins) { sessionOrigins.add(origin); }
        return choice[0] != 0;
    }

    private boolean ensureAndroidPermissions() throws InterruptedException {
        String[] required;
        if (Build.VERSION.SDK_INT >= 31) {
            required = new String[]{Manifest.permission.BLUETOOTH_SCAN, Manifest.permission.BLUETOOTH_CONNECT};
        } else {
            required = new String[]{Manifest.permission.ACCESS_FINE_LOCATION};
        }
        boolean missing = false;
        for (String permission : required) {
            if (activity.checkSelfPermission(permission) != PackageManager.PERMISSION_GRANTED) missing = true;
        }
        if (!missing) return true;
        final CountDownLatch latch = new CountDownLatch(1);
        synchronized (permissionLock) {
            runtimePermissionLatch = latch;
            runtimePermissionGranted = false;
        }
        activity.runOnUiThread(new Runnable() {
            @Override public void run() { activity.requestPermissions(required, PERMISSION_REQUEST); }
        });
        boolean completed = latch.await(2, TimeUnit.MINUTES);
        synchronized (permissionLock) {
            runtimePermissionLatch = null;
            return completed && runtimePermissionGranted;
        }
    }

    private JSONObject connectDevice(String origin, String id) throws Exception {
        BluetoothDevice device = requireDevice(id, origin);
        synchronized (sessions) {
            GattSession existing = sessions.get(id);
            BluetoothManager manager = (BluetoothManager) activity.getSystemService(Context.BLUETOOTH_SERVICE);
            if (existing != null && manager != null) {
                try {
                    if (manager.getConnectionState(device, BluetoothProfile.GATT)
                            == BluetoothProfile.STATE_CONNECTED) return serviceList(existing);
                } catch (SecurityException ignored) { }
            }
            if (existing != null && existing.gatt != null) {
                try { existing.gatt.close(); } catch (RuntimeException ignored) { }
                sessions.remove(id);
            }
        }
        GattSession session = new GattSession(device, id);
        BluetoothGatt gatt = device.connectGatt(activity, false, session, BluetoothDevice.TRANSPORT_LE);
        session.gatt = gatt;
        if (gatt == null || !session.connected.await(OP_TIMEOUT_SECONDS, TimeUnit.SECONDS)
                || session.connectionStatus != BluetoothGatt.GATT_SUCCESS
                || session.connectionState != BluetoothProfile.STATE_CONNECTED) {
            if (gatt != null) gatt.close();
            throw new BridgeException("Could not connect to the Bluetooth device.", "NetworkError");
        }
        if (!gatt.discoverServices() || !session.services.await(OP_TIMEOUT_SECONDS, TimeUnit.SECONDS)
                || session.serviceStatus != BluetoothGatt.GATT_SUCCESS) {
            gatt.disconnect();
            gatt.close();
            throw new BridgeException("Could not discover Bluetooth services.", "NetworkError");
        }
        synchronized (sessions) { sessions.put(id, session); }
        synchronized (sessionOriginsByDevice) { sessionOriginsByDevice.put(id, origin); }
        return serviceList(session);
    }

    private JSONObject serviceList(GattSession session) throws Exception {
        JSONArray services = new JSONArray();
        Set<UUID> allowed;
        synchronized (deviceAllowedServices) {
            allowed = new HashSet<>(deviceAllowedServices.getOrDefault(
                    session.deviceId, new HashSet<UUID>()));
        }
        for (BluetoothGattService service : session.gatt.getServices()) {
            if (!allowed.contains(service.getUuid())) continue;
            JSONObject serviceJson = new JSONObject();
            serviceJson.put("uuid", service.getUuid().toString());
            JSONArray characteristics = new JSONArray();
            for (BluetoothGattCharacteristic characteristic : service.getCharacteristics()) {
                JSONObject item = new JSONObject();
                item.put("uuid", characteristic.getUuid().toString());
                int properties = characteristic.getProperties();
                JSONObject flags = new JSONObject();
                flags.put("broadcast", has(properties, BluetoothGattCharacteristic.PROPERTY_BROADCAST));
                flags.put("read", has(properties, BluetoothGattCharacteristic.PROPERTY_READ));
                flags.put("writeWithoutResponse", has(properties, BluetoothGattCharacteristic.PROPERTY_WRITE_NO_RESPONSE));
                flags.put("write", has(properties, BluetoothGattCharacteristic.PROPERTY_WRITE));
                flags.put("notify", has(properties, BluetoothGattCharacteristic.PROPERTY_NOTIFY));
                flags.put("indicate", has(properties, BluetoothGattCharacteristic.PROPERTY_INDICATE));
                flags.put("authenticatedSignedWrites", has(properties, BluetoothGattCharacteristic.PROPERTY_SIGNED_WRITE));
                flags.put("reliableWrite", false);
                flags.put("writableAuxiliaries", false);
                item.put("properties", flags);
                characteristics.put(item);
            }
            serviceJson.put("characteristics", characteristics);
            services.put(serviceJson);
        }
        JSONObject result = new JSONObject();
        result.put("services", services);
        return result;
    }

    private static boolean has(int flags, int value) { return (flags & value) != 0; }

    private JSONArray readValue(String origin, JSONObject args) throws Exception {
        GattSession session = requireSession(origin, args.optString("deviceId"));
        BluetoothGattCharacteristic characteristic = requireCharacteristic(session, args);
        PendingOperation operation = session.beginOperation();
        if (!session.gatt.readCharacteristic(characteristic)) {
            session.clearOperation(operation);
            throw new BridgeException("The characteristic rejected the read.", "NetworkError");
        }
        session.awaitOperation(operation);
        JSONArray bytes = new JSONArray();
        for (byte value : operation.value) bytes.put(value & 0xff);
        return bytes;
    }

    private void writeValue(String origin, JSONObject args) throws Exception {
        GattSession session = requireSession(origin, args.optString("deviceId"));
        BluetoothGattCharacteristic characteristic = requireCharacteristic(session, args);
        JSONArray input = args.optJSONArray("value");
        if (input == null) throw new BridgeException("The write value is invalid.", "TypeError");
        byte[] bytes = new byte[input.length()];
        for (int index = 0; index < input.length(); index++) bytes[index] = (byte) input.optInt(index);
        characteristic.setWriteType(args.optBoolean("withResponse", true)
                ? BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT
                : BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE);
        characteristic.setValue(bytes);
        boolean withResponse = args.optBoolean("withResponse", true);
        PendingOperation operation = withResponse ? session.beginOperation() : null;
        if (!session.gatt.writeCharacteristic(characteristic)) {
            if (operation != null) session.clearOperation(operation);
            throw new BridgeException("The characteristic rejected the write.", "NetworkError");
        }
        if (operation != null) session.awaitOperation(operation);
    }

    private void startNotifications(String origin, JSONObject args) throws Exception {
        GattSession session = requireSession(origin, args.optString("deviceId"));
        BluetoothGattCharacteristic characteristic = requireCharacteristic(session, args);
        BluetoothGattDescriptor descriptor = characteristic.getDescriptor(CCCD);
        if (descriptor == null) throw new BridgeException("This characteristic has no notification descriptor.", "NotSupportedError");
        if (!session.gatt.setCharacteristicNotification(characteristic, true)) {
            throw new BridgeException("Notifications could not be enabled.", "NetworkError");
        }
        descriptor.setValue(BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE);
        PendingOperation operation = session.beginOperation();
        if (!session.gatt.writeDescriptor(descriptor)) {
            session.clearOperation(operation);
            throw new BridgeException("Notifications could not be enabled.", "NetworkError");
        }
        session.awaitOperation(operation);
        String key = args.optString("key");
        synchronized (session.notificationKeys) { session.notificationKeys.put(characteristic.getUuid(), key); }
    }

    private void stopNotifications(String origin, JSONObject args) throws Exception {
        GattSession session = requireSession(origin, args.optString("deviceId"));
        BluetoothGattCharacteristic characteristic = requireCharacteristic(session, args);
        session.gatt.setCharacteristicNotification(characteristic, false);
        BluetoothGattDescriptor descriptor = characteristic.getDescriptor(CCCD);
        if (descriptor != null) {
            descriptor.setValue(BluetoothGattDescriptor.DISABLE_NOTIFICATION_VALUE);
            PendingOperation operation = session.beginOperation();
            if (session.gatt.writeDescriptor(descriptor)) session.awaitOperation(operation);
            else session.clearOperation(operation);
        }
        synchronized (session.notificationKeys) { session.notificationKeys.remove(characteristic.getUuid()); }
    }

    private void disconnectDevice(String origin, String id) {
        synchronized (sessionOriginsByDevice) {
            if (!origin.equals(sessionOriginsByDevice.get(id))) return;
            sessionOriginsByDevice.remove(id);
        }
        GattSession session;
        synchronized (sessions) { session = sessions.remove(id); }
        if (session != null && session.gatt != null) {
            try { session.gatt.disconnect(); session.gatt.close(); } catch (RuntimeException ignored) { }
        }
    }

    private BluetoothDevice requireDevice(String id, String origin) throws BridgeException {
        synchronized (deviceOrigins) {
            if (!origin.equals(deviceOrigins.get(id))) throw new BridgeException("This Bluetooth device was not selected by this origin.", "NotAllowedError");
        }
        synchronized (discovered) {
            BluetoothDevice device = discovered.get(id);
            if (device == null) throw new BridgeException("The Bluetooth device is no longer available. Select it again.", "NotFoundError");
            return device;
        }
    }

    private GattSession requireSession(String origin, String id) throws BridgeException {
        synchronized (sessionOriginsByDevice) {
            if (!origin.equals(sessionOriginsByDevice.get(id))) throw new BridgeException("This Bluetooth connection belongs to a different origin.", "NotAllowedError");
        }
        synchronized (sessions) {
            GattSession session = sessions.get(id);
            if (session == null || session.gatt == null) throw new BridgeException("The Bluetooth device is not connected.", "NetworkError");
            return session;
        }
    }

    private BluetoothGattCharacteristic requireCharacteristic(GattSession session, JSONObject args) throws BridgeException {
        UUID serviceId = normalizeUuid(args.optString("serviceUuid"));
        UUID characteristicId = normalizeUuid(args.optString("characteristicUuid"));
        synchronized (deviceAllowedServices) {
            Set<UUID> allowed = deviceAllowedServices.get(session.deviceId);
            if (allowed == null || !allowed.contains(serviceId)) {
                throw new BridgeException("This website did not request access to that Bluetooth service.", "NotAllowedError");
            }
        }
        BluetoothGattService service = session.gatt.getService(serviceId);
        BluetoothGattCharacteristic characteristic = service == null ? null : service.getCharacteristic(characteristicId);
        if (characteristic == null) throw new BridgeException("The Bluetooth characteristic was not found.", "NotFoundError");
        return characteristic;
    }

    private BluetoothAdapter adapter() {
        BluetoothManager manager = (BluetoothManager) activity.getSystemService(Context.BLUETOOTH_SERVICE);
        return manager == null ? null : manager.getAdapter();
    }

    private String currentSecureOrigin() throws BridgeException {
        Uri uri = Uri.parse(webView.getUrl() == null ? "" : webView.getUrl());
        String scheme = uri.getScheme();
        String host = uri.getHost();
        if (host == null || !("https".equalsIgnoreCase(scheme)
                || (("http".equalsIgnoreCase(scheme) || "void".equalsIgnoreCase(scheme))
                    && ("localhost".equalsIgnoreCase(host) || "127.0.0.1".equals(host) || "::1".equals(host))))) {
            throw new BridgeException("Bluetooth is available only to secure website origins.", "SecurityError");
        }
        int port = uri.getPort();
        int defaultPort = "https".equalsIgnoreCase(scheme) ? 443 : 80;
        return scheme.toLowerCase(Locale.ROOT) + "://" + host.toLowerCase(Locale.ROOT)
                + (port < 0 || port == defaultPort ? "" : ":" + port);
    }

    private String errorResponse(String message, String name) {
        JSONObject result = new JSONObject();
        try { result.put("error", message); result.put("name", name); }
        catch (Exception ignored) { }
        return result.toString();
    }

    private static final class PendingOperation {
        final CountDownLatch done = new CountDownLatch(1);
        volatile int status = BluetoothGatt.GATT_FAILURE;
        volatile byte[] value;
        void await() throws Exception {
            if (!done.await(OP_TIMEOUT_SECONDS, TimeUnit.SECONDS)) throw new BridgeException("The Bluetooth device did not respond in time.", "NetworkError");
            if (status != BluetoothGatt.GATT_SUCCESS) throw new BridgeException("The Bluetooth device reported GATT error " + status + ".", "NetworkError");
        }
    }

    private final class GattSession extends BluetoothGattCallback {
        final BluetoothDevice device;
        final String deviceId;
        final CountDownLatch connected = new CountDownLatch(1);
        final CountDownLatch services = new CountDownLatch(1);
        final Map<UUID, String> notificationKeys = new HashMap<>();
        volatile BluetoothGatt gatt;
        volatile PendingOperation operation;
        volatile int connectionStatus = BluetoothGatt.GATT_FAILURE;
        volatile int connectionState = BluetoothProfile.STATE_DISCONNECTED;
        volatile int serviceStatus = BluetoothGatt.GATT_FAILURE;
        GattSession(BluetoothDevice device, String deviceId) {
            this.device = device;
            this.deviceId = deviceId;
        }
        synchronized PendingOperation beginOperation() throws BridgeException {
            if (operation != null) throw new BridgeException("Another Bluetooth operation is still pending.", "NetworkError");
            operation = new PendingOperation();
            return operation;
        }
        synchronized void clearOperation(PendingOperation expected) {
            if (operation == expected) operation = null;
        }
        void awaitOperation(PendingOperation expected) throws Exception {
            try { expected.await(); } finally { clearOperation(expected); }
        }
        private void finish(int status, byte[] value) {
            PendingOperation active = operation;
            if (active != null) { active.status = status; active.value = value == null ? new byte[0] : value.clone(); active.done.countDown(); operation = null; }
        }
        @Override public void onConnectionStateChange(BluetoothGatt gatt, int status, int newState) {
            connectionStatus = status;
            connectionState = newState;
            if (newState == BluetoothProfile.STATE_CONNECTED || newState == BluetoothProfile.STATE_DISCONNECTED) connected.countDown();
        }
        @Override public void onServicesDiscovered(BluetoothGatt gatt, int status) {
            serviceStatus = status;
            services.countDown();
        }
        @Override public void onCharacteristicRead(BluetoothGatt gatt, BluetoothGattCharacteristic characteristic, int status) {
            finish(status, characteristic.getValue());
        }
        @Override public void onCharacteristicWrite(BluetoothGatt gatt, BluetoothGattCharacteristic characteristic, int status) { finish(status, null); }
        @Override public void onDescriptorWrite(BluetoothGatt gatt, BluetoothGattDescriptor descriptor, int status) { finish(status, null); }
        @Override public void onCharacteristicChanged(BluetoothGatt gatt, BluetoothGattCharacteristic characteristic) {
            byte[] value = characteristic.getValue();
            dispatchNotification(characteristic, value == null ? new byte[0] : value);
        }
        @Override public void onCharacteristicChanged(
                BluetoothGatt gatt, BluetoothGattCharacteristic characteristic, byte[] value) {
            dispatchNotification(characteristic, value);
        }
        private void dispatchNotification(BluetoothGattCharacteristic characteristic, byte[] value) {
            String key;
            synchronized (notificationKeys) { key = notificationKeys.get(characteristic.getUuid()); }
            if (key == null || webView == null) return;
            JSONArray bytes = new JSONArray();
            for (byte item : value) bytes.put(item & 0xff);
            final String script = "window.__voidBluetoothNotification&&window.__voidBluetoothNotification({key:"
                    + JSONObject.quote(key) + ",value:" + bytes + "})";
            webView.post(new Runnable() {
                @Override public void run() { webView.evaluateJavascript(script, null); }
            });
        }
    }

    private static final class BridgeException extends Exception {
        final String name;
        BridgeException(String message, String name) { super(message); this.name = name; }
    }
}
