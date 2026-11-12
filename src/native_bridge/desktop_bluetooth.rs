use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::Duration,
};

use btleplug::{
    api::{Central, CharPropFlags, Manager as _, Peripheral as _, ScanFilter, WriteType},
    platform::{Manager, Peripheral},
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use tao::event_loop::EventLoopProxy;
use tokio::{runtime::Runtime, task::JoinHandle};
use uuid::Uuid;

use super::UserEvent;

const SCAN_TIME: Duration = Duration::from_secs(5);
const IO_TIMEOUT: Duration = Duration::from_secs(25);

struct GrantedDevice {
    origin: String,
    name: Option<String>,
    peripheral: Peripheral,
    allowed_services: HashSet<Uuid>,
}

static RUNTIME: OnceLock<Result<Runtime, String>> = OnceLock::new();
static DEVICES: OnceLock<Mutex<HashMap<String, GrantedDevice>>> = OnceLock::new();
static SESSION_ORIGINS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static NOTIFICATIONS: OnceLock<Mutex<HashMap<String, JoinHandle<()>>>> = OnceLock::new();

pub(super) fn handle(
    origin: &str,
    method: &str,
    args: &Value,
    proxy: EventLoopProxy<UserEvent>,
) -> Result<Value, (String, &'static str)> {
    runtime()?.block_on(async { handle_async(origin, method, args, proxy).await })
}

fn runtime() -> Result<&'static Runtime, (String, &'static str)> {
    RUNTIME
        .get_or_init(|| Runtime::new().map_err(|error| error.to_string()))
        .as_ref()
        .map_err(|error| {
            (
                format!("Bluetooth runtime could not start: {error}"),
                "NotSupportedError",
            )
        })
}

async fn handle_async(
    origin: &str,
    method: &str,
    args: &Value,
    proxy: EventLoopProxy<UserEvent>,
) -> Result<Value, (String, &'static str)> {
    match method {
        "bluetooth.getAvailability" => Ok(json!(first_adapter().await.is_ok())),
        "bluetooth.requestDevice" => request_device(origin, args, proxy.clone()).await,
        "bluetooth.getDevices" => get_devices(origin),
        "bluetooth.connectDevice" => connect_device(origin, args).await,
        "bluetooth.disconnectDevice" => {
            let device = granted_device(origin, string(args, "deviceId")?)?;
            device.disconnect().await.map_err(network_error)?;
            Ok(Value::Null)
        }
        "bluetooth.readValue" => read_value(origin, args).await,
        "bluetooth.writeValue" => write_value(origin, args).await,
        "bluetooth.startNotifications" => start_notifications(origin, args, proxy).await,
        "bluetooth.stopNotifications" => stop_notifications(origin, args).await,
        _ => Err((
            "Unsupported Bluetooth operation.".into(),
            "NotSupportedError",
        )),
    }
}

async fn first_adapter() -> Result<btleplug::platform::Adapter, (String, &'static str)> {
    let manager = Manager::new().await.map_err(network_error)?;
    manager
        .adapters()
        .await
        .map_err(network_error)?
        .into_iter()
        .next()
        .ok_or_else(|| {
            (
                "No Bluetooth adapter is available on this device.".into(),
                "NotFoundError",
            )
        })
}

async fn request_device(
    origin: &str,
    args: &Value,
    proxy: EventLoopProxy<UserEvent>,
) -> Result<Value, (String, &'static str)> {
    let options = args.get("options").unwrap_or(&Value::Null);
    let accept_all = options
        .get("acceptAllDevices")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let filters = options.get("filters").and_then(Value::as_array);
    if !accept_all && filters.map_or(true, Vec::is_empty) {
        return Err((
            "Provide Bluetooth filters or set acceptAllDevices to true.".into(),
            "TypeError",
        ));
    }
    if accept_all && filters.is_some_and(|filters| !filters.is_empty()) {
        return Err((
            "acceptAllDevices cannot be combined with filters.".into(),
            "TypeError",
        ));
    }
    if !allow_origin(origin, &proxy)? {
        return Err((
            "Bluetooth access was denied for this website.".into(),
            "NotAllowedError",
        ));
    }

    let adapter = first_adapter().await?;
    adapter
        .start_scan(ScanFilter::default())
        .await
        .map_err(network_error)?;
    tokio::time::sleep(SCAN_TIME).await;
    let stop_result = adapter.stop_scan().await;
    let peripherals = adapter.peripherals().await.map_err(network_error)?;
    if let Err(error) = stop_result {
        return Err(network_error(error));
    }

    let mut candidates = Vec::new();
    for peripheral in peripherals {
        let properties = match peripheral.properties().await {
            Ok(Some(properties)) => properties,
            _ => continue,
        };
        if accept_all
            || filters.is_some_and(|items| {
                items
                    .iter()
                    .any(|filter| matches_filter(filter, &properties))
            })
        {
            candidates.push((peripheral, properties));
        }
    }
    if candidates.is_empty() {
        return Err((
            "No matching Bluetooth LE devices were found.".into(),
            "NotFoundError",
        ));
    }
    let selected = choose_device(&candidates, &proxy)?.ok_or_else(|| {
        (
            "Bluetooth device selection was cancelled.".into(),
            "NotAllowedError",
        )
    })?;
    let (peripheral, properties) = candidates.swap_remove(selected);
    let name = properties.local_name.clone();
    let id = Uuid::new_v4().to_string();

    let mut allowed_services = HashSet::new();
    for value in options
        .get("optionalServices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .chain(filters.into_iter().flatten().flat_map(|filter| {
            filter
                .get("services")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        }))
    {
        if let Some(service) = value.as_str().and_then(parse_uuid) {
            allowed_services.insert(service);
        }
    }

    devices().lock().map_err(lock_error)?.insert(
        id.clone(),
        GrantedDevice {
            origin: origin.into(),
            name: name.clone(),
            peripheral,
            allowed_services,
        },
    );
    Ok(json!({ "id": id, "name": name }))
}

fn get_devices(origin: &str) -> Result<Value, (String, &'static str)> {
    let mut result = Vec::new();
    for (id, device) in devices().lock().map_err(lock_error)?.iter() {
        if device.origin != origin {
            continue;
        }
        result.push(json!({
            "id": id,
            "name": device.name,
            "services": device.allowed_services.iter().map(Uuid::to_string).collect::<Vec<_>>()
        }));
    }
    Ok(Value::Array(result))
}

fn matches_filter(filter: &Value, properties: &btleplug::api::PeripheralProperties) -> bool {
    let mut has_constraint = false;
    if let Some(name) = filter.get("name").and_then(Value::as_str) {
        has_constraint = true;
        if properties.local_name.as_deref() != Some(name) {
            return false;
        }
    }
    if let Some(prefix) = filter.get("namePrefix").and_then(Value::as_str) {
        has_constraint = true;
        if !properties
            .local_name
            .as_deref()
            .is_some_and(|name| name.starts_with(prefix))
        {
            return false;
        }
    }
    if let Some(services) = filter.get("services").and_then(Value::as_array) {
        if !services.is_empty() {
            has_constraint = true;
            let matched = services
                .iter()
                .filter_map(Value::as_str)
                .filter_map(parse_uuid)
                .any(|service| properties.services.contains(&service));
            if !matched {
                return false;
            }
        }
    }
    if let Some(manufacturer_data) = filter.get("manufacturerData").and_then(Value::as_array) {
        if !manufacturer_data.is_empty() {
            has_constraint = true;
        }
        for entry in manufacturer_data {
            let Some(company_id) = entry
                .get("companyIdentifier")
                .and_then(Value::as_u64)
                .filter(|id| *id <= u16::MAX as u64)
            else {
                return false;
            };
            let Some(advertised) = properties.manufacturer_data.get(&(company_id as u16)) else {
                return false;
            };
            if let Some(prefix) = entry.get("dataPrefix").and_then(Value::as_array) {
                if !matches_prefix(advertised, prefix) {
                    return false;
                }
            }
        }
    }
    if let Some(service_data) = filter.get("serviceData").and_then(Value::as_array) {
        if !service_data.is_empty() {
            has_constraint = true;
        }
        for entry in service_data {
            let Some(service) = entry
                .get("service")
                .and_then(Value::as_str)
                .and_then(parse_uuid)
            else {
                return false;
            };
            let Some(advertised) = properties.service_data.get(&service) else {
                return false;
            };
            if let Some(prefix) = entry.get("dataPrefix").and_then(Value::as_array) {
                if !matches_prefix(advertised, prefix) {
                    return false;
                }
            }
        }
    }
    has_constraint
}

fn matches_prefix(bytes: &[u8], prefix: &[Value]) -> bool {
    bytes.len() >= prefix.len()
        && prefix
            .iter()
            .enumerate()
            .all(|(index, value)| value.as_u64() == Some(bytes[index] as u64))
}

fn choose_device(
    candidates: &[(Peripheral, btleplug::api::PeripheralProperties)],
    proxy: &EventLoopProxy<UserEvent>,
) -> Result<Option<usize>, (String, &'static str)> {
    let mut first = 0;
    loop {
        let one = format!("[{}] {}", first + 1, display_name(&candidates[first].1));
        let second_index = first + 1;
        let second = candidates
            .get(second_index)
            .map(|(_, properties)| format!("[{}] {}", second_index + 1, display_name(properties)))
            .unwrap_or_else(|| "Cancel selection".into());
        let more = if first + 2 < candidates.len() {
            "More devices…"
        } else {
            "Cancel selection"
        };
        let result = show_native_prompt(
            proxy,
            "Choose a Bluetooth device",
            "Select the nearby Bluetooth LE device to share with this website.",
            [one, second, more.into()],
        )?;
        match result.as_str() {
            label if label.starts_with('[') => {
                let Some(index) = label
                    .split(']')
                    .next()
                    .and_then(|value| value.trim_start_matches('[').parse::<usize>().ok())
                    .and_then(|value| value.checked_sub(1))
                else {
                    return Ok(None);
                };
                if index < candidates.len() {
                    return Ok(Some(index));
                }
            }
            "More devices…" => first += 2,
            _ => return Ok(None),
        }
        if first >= candidates.len() {
            first = 0;
        }
    }
}

fn display_name(properties: &btleplug::api::PeripheralProperties) -> String {
    properties
        .local_name
        .clone()
        .unwrap_or_else(|| properties.address.to_string())
}

async fn connect_device(origin: &str, args: &Value) -> Result<Value, (String, &'static str)> {
    let id = string(args, "deviceId")?;
    let (peripheral, allowed) = {
        let devices = devices().lock().map_err(lock_error)?;
        let device = devices
            .get(id)
            .filter(|device| device.origin == origin)
            .ok_or_else(|| {
                (
                    "This device was not selected by this website.".into(),
                    "NotAllowedError",
                )
            })?;
        (device.peripheral.clone(), device.allowed_services.clone())
    };
    if !peripheral.is_connected().await.map_err(network_error)? {
        tokio::time::timeout(IO_TIMEOUT, peripheral.connect())
            .await
            .map_err(|_| {
                (
                    "Connecting to the Bluetooth device timed out.".into(),
                    "NetworkError",
                )
            })?
            .map_err(network_error)?;
    }
    tokio::time::timeout(IO_TIMEOUT, peripheral.discover_services())
        .await
        .map_err(|_| {
            (
                "Discovering Bluetooth services timed out.".into(),
                "NetworkError",
            )
        })?
        .map_err(network_error)?;

    let services = peripheral.services().into_iter().filter(|service| allowed.contains(&service.uuid)).map(|service| {
        let characteristics = service.characteristics.into_iter().map(|characteristic| {
            json!({
                "uuid": characteristic.uuid.to_string(),
                "properties": {
                    "broadcast": characteristic.properties.contains(CharPropFlags::BROADCAST),
                    "read": characteristic.properties.contains(CharPropFlags::READ),
                    "writeWithoutResponse": characteristic.properties.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE),
                    "write": characteristic.properties.contains(CharPropFlags::WRITE),
                    "notify": characteristic.properties.contains(CharPropFlags::NOTIFY),
                    "indicate": characteristic.properties.contains(CharPropFlags::INDICATE),
                    "authenticatedSignedWrites": characteristic.properties.contains(CharPropFlags::AUTHENTICATED_SIGNED_WRITES),
                    "reliableWrite": false,
                    "writableAuxiliaries": false
                }
            })
        }).collect::<Vec<_>>();
        json!({ "uuid": service.uuid.to_string(), "characteristics": characteristics })
    }).collect::<Vec<_>>();
    Ok(json!({ "services": services }))
}

async fn read_value(origin: &str, args: &Value) -> Result<Value, (String, &'static str)> {
    let (peripheral, characteristic) = characteristic(origin, args)?;
    let value = tokio::time::timeout(IO_TIMEOUT, peripheral.read(&characteristic))
        .await
        .map_err(|_| {
            (
                "Reading the Bluetooth characteristic timed out.".into(),
                "NetworkError",
            )
        })?
        .map_err(network_error)?;
    Ok(json!(value))
}

async fn write_value(origin: &str, args: &Value) -> Result<Value, (String, &'static str)> {
    let (peripheral, characteristic) = characteristic(origin, args)?;
    let value = args
        .get("value")
        .and_then(Value::as_array)
        .ok_or_else(|| ("The Bluetooth write value is invalid.".into(), "TypeError"))?
        .iter()
        .map(|byte| {
            byte.as_u64()
                .filter(|byte| *byte <= 255)
                .map(|byte| byte as u8)
                .ok_or_else(|| {
                    (
                        "The Bluetooth write value must contain bytes.".into(),
                        "TypeError",
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let write_type = if args
        .get("withResponse")
        .and_then(Value::as_bool)
        .unwrap_or(true)
    {
        WriteType::WithResponse
    } else {
        WriteType::WithoutResponse
    };
    tokio::time::timeout(
        IO_TIMEOUT,
        peripheral.write(&characteristic, &value, write_type),
    )
    .await
    .map_err(|_| {
        (
            "Writing the Bluetooth characteristic timed out.".into(),
            "NetworkError",
        )
    })?
    .map_err(network_error)?;
    Ok(Value::Null)
}

async fn start_notifications(
    origin: &str,
    args: &Value,
    proxy: EventLoopProxy<UserEvent>,
) -> Result<Value, (String, &'static str)> {
    let (peripheral, characteristic) = characteristic(origin, args)?;
    peripheral
        .subscribe(&characteristic)
        .await
        .map_err(network_error)?;
    let mut stream = peripheral.notifications().await.map_err(network_error)?;
    let device_id = string(args, "deviceId")?.to_string();
    let key = string(args, "key")?.to_string();
    let origin = origin.to_string();
    let service_id = characteristic.service_uuid;
    let characteristic_id = characteristic.uuid;
    let task = runtime()?.spawn(async move {
        while let Some(notification) = stream.next().await {
            if notification.service_uuid != service_id || notification.uuid != characteristic_id { continue; }
            let message = json!({ "key": key, "value": notification.value });
            if let Ok(encoded) = serde_json::to_string(&message) {
                let script = format!("if (typeof window.__voidBluetoothNotification === 'function') window.__voidBluetoothNotification({encoded});");
                let _ = proxy.send_event(UserEvent::Script { origin: origin.clone(), source: script });
            }
        }
    });
    notifications()
        .lock()
        .map_err(lock_error)?
        .insert(notification_key(&device_id, &characteristic), task);
    Ok(Value::Null)
}

async fn stop_notifications(origin: &str, args: &Value) -> Result<Value, (String, &'static str)> {
    let (peripheral, characteristic) = characteristic(origin, args)?;
    peripheral
        .unsubscribe(&characteristic)
        .await
        .map_err(network_error)?;
    if let Ok(mut tasks) = notifications().lock() {
        if let Some(task) = tasks.remove(&notification_key(
            string(args, "deviceId")?,
            &characteristic,
        )) {
            task.abort();
        }
    }
    Ok(Value::Null)
}

fn characteristic(
    origin: &str,
    args: &Value,
) -> Result<(Peripheral, btleplug::api::Characteristic), (String, &'static str)> {
    let id = string(args, "deviceId")?;
    let service = string(args, "serviceUuid")?
        .parse::<Uuid>()
        .map_err(|_| ("The Bluetooth service UUID is invalid.".into(), "TypeError"))?;
    let uuid = string(args, "characteristicUuid")?
        .parse::<Uuid>()
        .map_err(|_| {
            (
                "The Bluetooth characteristic UUID is invalid.".into(),
                "TypeError",
            )
        })?;
    let devices = devices().lock().map_err(lock_error)?;
    let device = devices
        .get(id)
        .filter(|device| device.origin == origin)
        .ok_or_else(|| {
            (
                "This device was not selected by this website.".into(),
                "NotAllowedError",
            )
        })?;
    if !device.allowed_services.contains(&service) {
        return Err((
            "This website did not request access to that Bluetooth service.".into(),
            "NotAllowedError",
        ));
    }
    let characteristic = device
        .peripheral
        .characteristics()
        .into_iter()
        .find(|item| item.service_uuid == service && item.uuid == uuid)
        .ok_or_else(|| {
            (
                "The Bluetooth characteristic was not found.".into(),
                "NotFoundError",
            )
        })?;
    Ok((device.peripheral.clone(), characteristic))
}

fn granted_device(origin: &str, id: &str) -> Result<Peripheral, (String, &'static str)> {
    devices()
        .lock()
        .map_err(lock_error)?
        .get(id)
        .filter(|device| device.origin == origin)
        .map(|device| device.peripheral.clone())
        .ok_or_else(|| {
            (
                "This device was not selected by this website.".into(),
                "NotAllowedError",
            )
        })
}

fn notification_key(device_id: &str, characteristic: &btleplug::api::Characteristic) -> String {
    format!(
        "{device_id}|{}|{}",
        characteristic.service_uuid, characteristic.uuid
    )
}

fn allow_origin(
    origin: &str,
    proxy: &EventLoopProxy<UserEvent>,
) -> Result<bool, (String, &'static str)> {
    if persistent_origins()?.contains(origin) {
        return Ok(true);
    }
    let sessions = SESSION_ORIGINS.get_or_init(|| Mutex::new(HashSet::new()));
    if sessions.lock().map_err(lock_error)?.contains(origin) {
        return Ok(true);
    }

    let choice = show_native_prompt(
        proxy,
        "Bluetooth access",
        &format!(
            "{origin} wants to scan for nearby Bluetooth devices and connect to a selected device."
        ),
        [
            "Allow Always".into(),
            "Allow This Time Only".into(),
            "Don't Allow".into(),
        ],
    )?;
    match choice.as_str() {
        "Allow Always" => {
            let mut origins = persistent_origins()?;
            origins.insert(origin.to_string());
            save_persistent_origins(&origins)?;
            Ok(true)
        }
        "Allow This Time Only" => {
            sessions
                .lock()
                .map_err(lock_error)?
                .insert(origin.to_string());
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn persistent_origins() -> Result<HashSet<String>, (String, &'static str)> {
    let path = permissions_path()?;
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashSet::new()),
        Err(error) => {
            return Err((
                format!("Could not read Bluetooth permissions: {error}"),
                "NotAllowedError",
            ))
        }
    };
    let entries = serde_json::from_str::<Vec<String>>(&content).map_err(|error| {
        (
            format!("Bluetooth permission storage is invalid: {error}"),
            "NotAllowedError",
        )
    })?;
    Ok(entries.into_iter().collect())
}

fn save_persistent_origins(origins: &HashSet<String>) -> Result<(), (String, &'static str)> {
    let path = permissions_path()?;
    let parent = path.parent().ok_or_else(|| {
        (
            "Bluetooth permission storage path is invalid.".into(),
            "NotAllowedError",
        )
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        (
            format!("Could not create Bluetooth permission storage: {error}"),
            "NotAllowedError",
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).map_err(|error| {
            (
                format!("Could not secure Bluetooth permission storage: {error}"),
                "NotAllowedError",
            )
        })?;
    }
    let serialized =
        serde_json::to_vec_pretty(&origins.iter().collect::<Vec<_>>()).map_err(|error| {
            (
                format!("Could not serialize Bluetooth permissions: {error}"),
                "NotAllowedError",
            )
        })?;
    fs::write(&path, serialized).map_err(|error| {
        (
            format!("Could not save Bluetooth permissions: {error}"),
            "NotAllowedError",
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|error| {
            (
                format!("Could not secure Bluetooth permissions: {error}"),
                "NotAllowedError",
            )
        })?;
    }
    Ok(())
}

fn permissions_path() -> Result<PathBuf, (String, &'static str)> {
    #[cfg(target_os = "macos")]
    let root = env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library/Application Support/VOID"));
    #[cfg(target_os = "windows")]
    let root = env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|appdata| appdata.join("VOID"));
    #[cfg(target_os = "linux")]
    let root = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".config"))
        })
        .map(|config| config.join("void"));
    root.map(|root| root.join("bluetooth-permissions.json"))
        .ok_or_else(|| {
            (
                "Could not find a private configuration directory for Bluetooth permissions."
                    .into(),
                "NotAllowedError",
            )
        })
}

fn parse_uuid(value: &str) -> Option<Uuid> {
    if value.len() == 4 {
        return Uuid::parse_str(&format!("0000{value}-0000-1000-8000-00805f9b34fb")).ok();
    }
    if value.len() == 8 {
        return Uuid::parse_str(&format!("{value}-0000-1000-8000-00805f9b34fb")).ok();
    }
    Uuid::parse_str(value).ok()
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, (String, &'static str)> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| (format!("Missing Bluetooth field: {key}."), "TypeError"))
}

fn devices() -> &'static Mutex<HashMap<String, GrantedDevice>> {
    DEVICES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn notifications() -> &'static Mutex<HashMap<String, JoinHandle<()>>> {
    NOTIFICATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> (String, &'static str) {
    (
        "Bluetooth bridge state is unavailable.".into(),
        "NetworkError",
    )
}

fn network_error(error: impl std::fmt::Display) -> (String, &'static str) {
    (
        format!("Bluetooth operation failed: {error}"),
        "NetworkError",
    )
}

fn show_native_prompt(
    proxy: &EventLoopProxy<UserEvent>,
    title: &str,
    description: &str,
    buttons: [String; 3],
) -> Result<String, (String, &'static str)> {
    let (sender, receiver) = std::sync::mpsc::channel();
    proxy
        .send_event(UserEvent::Prompt {
            title: title.to_string(),
            description: description.to_string(),
            buttons,
            response: sender,
        })
        .map_err(|error| {
            (
                format!("Could not display the Bluetooth permission dialog: {error}"),
                "NotAllowedError",
            )
        })?;
    receiver
        .recv_timeout(Duration::from_secs(5 * 60))
        .map_err(|_| {
            (
                "The Bluetooth dialog was dismissed or timed out.".into(),
                "NotAllowedError",
            )
        })
}
