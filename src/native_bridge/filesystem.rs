use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    path::{Component, Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::UNIX_EPOCH,
};

type RootKey = (String, String);

static GRANTED_ROOTS: OnceLock<Mutex<HashMap<RootKey, PathBuf>>> = OnceLock::new();
const MAX_DIRECTORY_ENTRIES: usize = 2_000;
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

pub(super) fn handle(
    origin: &str,
    method: &str,
    args: &Value,
) -> Result<Value, (String, &'static str)> {
    match method {
        "filesystem.pickDirectory" => pick_directory(origin),
        "filesystem.listDirectory" => list_directory(origin, args),
        "filesystem.readFile" => read_file(origin, args),
        _ => Err((
            "This native folder operation is not supported by this build.".into(),
            "NotSupportedError",
        )),
    }
}

fn pick_directory(origin: &str) -> Result<Value, (String, &'static str)> {
    let selected = rfd::FileDialog::new()
        .set_title("Choose a folder to share with this website")
        .pick_folder()
        .ok_or_else(|| ("Folder selection was cancelled.".into(), "AbortError"))?;
    let root = fs::canonicalize(&selected).map_err(|error| {
        (
            format!("Could not open the selected folder: {error}"),
            "NotFoundError",
        )
    })?;
    let id = uuid::Uuid::new_v4().to_string();
    let name = root
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("Folder")
        .to_string();
    roots()
        .lock()
        .map_err(|_| ("Folder access state is unavailable.".into(), "UnknownError"))?
        .insert((origin.to_string(), id.clone()), root);
    Ok(json!({ "id": id, "name": name }))
}

fn list_directory(origin: &str, args: &Value) -> Result<Value, (String, &'static str)> {
    let (_root, path) = resolve_handle(origin, args)?;
    let mut entries = Vec::new();
    let items = fs::read_dir(path).map_err(|error| {
        (
            format!("Could not read this folder: {error}"),
            "NotReadableError",
        )
    })?;
    for item in items.take(MAX_DIRECTORY_ENTRIES) {
        let item = item.map_err(|error| {
            (
                format!("Could not inspect a folder entry: {error}"),
                "NotReadableError",
            )
        })?;
        let kind = item.file_type().map_err(|error| {
            (
                format!("Could not inspect a folder entry: {error}"),
                "NotReadableError",
            )
        })?;
        // Do not follow symbolic links; a selected folder must not silently
        // grant access to files outside the folder the user chose.
        let entry_kind = if kind.is_dir() {
            "directory"
        } else if kind.is_file() {
            "file"
        } else {
            continue;
        };
        let Some(name) = item.file_name().to_str().map(str::to_string) else {
            continue;
        };
        entries.push(json!({ "name": name, "kind": entry_kind }));
    }
    entries.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
    Ok(Value::Array(entries))
}

fn read_file(origin: &str, args: &Value) -> Result<Value, (String, &'static str)> {
    let (root, path) = resolve_handle(origin, args)?;
    let file_type = fs::symlink_metadata(&path)
        .map_err(|error| {
            (
                format!("Could not open this file: {error}"),
                "NotFoundError",
            )
        })?
        .file_type();
    if !file_type.is_file() {
        return Err((
            "The selected entry is not a regular file.".into(),
            "TypeMismatchError",
        ));
    }
    let canonical = fs::canonicalize(&path).map_err(|error| {
        (
            format!("Could not open this file: {error}"),
            "NotFoundError",
        )
    })?;
    if !canonical.starts_with(&root) {
        return Err((
            "This file is outside the selected folder.".into(),
            "SecurityError",
        ));
    }
    let metadata = fs::metadata(&canonical).map_err(|error| {
        (
            format!("Could not inspect this file: {error}"),
            "NotReadableError",
        )
    })?;
    if metadata.len() > MAX_FILE_BYTES {
        return Err((
            format!(
                "This file exceeds the {} MiB read limit.",
                MAX_FILE_BYTES / 1024 / 1024
            ),
            "NotReadableError",
        ));
    }
    let bytes = fs::read(&canonical).map_err(|error| {
        (
            format!("Could not read this file: {error}"),
            "NotReadableError",
        )
    })?;
    let last_modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    Ok(json!({
        "bytes": base64(&bytes),
        "size": bytes.len(),
        "lastModified": last_modified
    }))
}

fn resolve_handle(
    origin: &str,
    args: &Value,
) -> Result<(PathBuf, PathBuf), (String, &'static str)> {
    let id = args
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| ("The folder handle is invalid.".into(), "NotFoundError"))?;
    let root = roots()
        .lock()
        .map_err(|_| ("Folder access state is unavailable.".into(), "UnknownError"))?
        .get(&(origin.to_string(), id.to_string()))
        .cloned()
        .ok_or_else(|| {
            (
                "This folder permission has expired.".into(),
                "NotFoundError",
            )
        })?;
    let segments = args
        .get("path")
        .and_then(Value::as_array)
        .ok_or_else(|| ("The folder handle path is invalid.".into(), "SecurityError"))?;
    let mut joined = root.clone();
    for segment in segments {
        let name = segment
            .as_str()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| ("The folder handle path is invalid.".into(), "SecurityError"))?;
        let mut components = Path::new(name).components();
        if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
            return Err(("The folder handle path is invalid.".into(), "SecurityError"));
        }
        joined.push(name);
    }
    let path = fs::canonicalize(&joined).map_err(|error| {
        (
            format!("Could not open this folder entry: {error}"),
            "NotFoundError",
        )
    })?;
    if !path.starts_with(&root) {
        return Err((
            "This entry is outside the selected folder.".into(),
            "SecurityError",
        ));
    }
    Ok((root, path))
}

fn roots() -> &'static Mutex<HashMap<RootKey, PathBuf>> {
    GRANTED_ROOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);
        output.push(TABLE[(first >> 2) as usize] as char);
        output.push(TABLE[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        output.push(if chunk.len() > 1 {
            TABLE[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            TABLE[(third & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    output
}
