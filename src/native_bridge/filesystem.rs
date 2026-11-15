use super::UserEvent;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    sync::{mpsc, Mutex, OnceLock},
    time::{Duration, UNIX_EPOCH},
};
use tao::event_loop::EventLoopProxy;

#[cfg(target_os = "macos")]
use super::desktop_auth;

type RootKey = (String, String);
type Reply = Result<Value, (String, &'static str)>;

#[derive(Clone)]
struct Grant {
    root: PathBuf,
    writable: bool,
    auto_approve: bool,
}

static GRANTED_ROOTS: OnceLock<Mutex<HashMap<RootKey, Grant>>> = OnceLock::new();
// Only the app's configured website may touch the folder bridge; every other
// origin is refused before a prompt can be raised.
static ALLOWED_HOST: OnceLock<Option<String>> = OnceLock::new();
const MAX_DIRECTORY_ENTRIES: usize = 2_000;
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const PROMPT_TIMEOUT: Duration = Duration::from_secs(300);

pub(super) fn handle(
    origin: &str,
    method: &str,
    args: &Value,
    proxy: &EventLoopProxy<UserEvent>,
) -> Reply {
    if !origin_allowed(origin) {
        return Err((
            "Folder access is limited to the website this app was built for.".into(),
            "SecurityError",
        ));
    }

    match method {
        "filesystem.pickDirectory" => pick_directory(origin, args, proxy),
        "filesystem.requestWriteAccess" => request_write_access(origin, args),
        "filesystem.listDirectory" => list_directory(origin, args),
        "filesystem.readFile" => read_file(origin, args),
        "filesystem.writeFile" => write_file(origin, args, proxy),
        "filesystem.createDirectory" => create_directory(origin, args, proxy),
        "filesystem.removeEntry" => remove_entry(origin, args, proxy),
        _ => Err((
            "This native folder operation is not supported by this build.".into(),
            "NotSupportedError",
        )),
    }
}

fn pick_directory(origin: &str, args: &Value, proxy: &EventLoopProxy<UserEvent>) -> Reply {
    let writable = args.get("mode").and_then(Value::as_str) == Some("readwrite");

    // Layer 1 runs before the picker: nobody shares a folder without proving
    // they are the person sitting at the Mac.
    authenticate(if writable {
        "Allow this website to read and change files in a folder you choose"
    } else {
        "Allow this website to read a folder you choose"
    })?;

    let (sender, receiver) = mpsc::channel();
    proxy
        .send_event(UserEvent::PickFolder { response: sender })
        .map_err(|_| {
            (
                "VOID could not open the folder picker.".to_string(),
                "UnknownError",
            )
        })?;
    let selected = receiver
        .recv_timeout(PROMPT_TIMEOUT)
        .map_err(|_| ("Folder selection timed out.".to_string(), "AbortError"))?
        .ok_or_else(|| ("Folder selection was cancelled.".to_string(), "AbortError"))?;

    let root = fs::canonicalize(PathBuf::from(selected)).map_err(|error| {
        (
            format!("Could not open the selected folder: {error}"),
            "NotFoundError",
        )
    })?;
    let name = root
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("Folder")
        .to_string();
    let id = uuid::Uuid::new_v4().to_string();

    grants()
        .lock()
        .map_err(|_| {
            (
                "Folder access state is unavailable.".to_string(),
                "UnknownError",
            )
        })?
        .insert(
            (origin.to_string(), id.clone()),
            Grant {
                root: root.clone(),
                writable,
                auto_approve: false,
            },
        );

    eprintln!(
        "[void] shared folder {} with {origin} ({})",
        root.display(),
        if writable { "read-write" } else { "read-only" }
    );

    Ok(json!({
        "id": id,
        "name": name,
        "mode": if writable { "readwrite" } else { "read" },
    }))
}

pub(super) fn set_allowed_host(host: Option<String>) {
    let _ = ALLOWED_HOST.set(host);
}

fn origin_allowed(origin: &str) -> bool {
    match ALLOWED_HOST.get() {
        Some(Some(host)) => origin
            .parse::<wry::http::Uri>()
            .ok()
            .and_then(|uri| uri.host().map(|value| value.eq_ignore_ascii_case(host)))
            .unwrap_or(false),
        // Local builds have no remote host; only the bundled scheme is trusted.
        Some(None) => origin == "void://localhost",
        // Fail closed when the host was never configured.
        None => false,
    }
}

fn request_write_access(origin: &str, args: &Value) -> Reply {
    let id = handle_id(args)?;
    let grant = lookup_grant(origin, args)?;
    if grant.writable {
        return Ok(json!({ "mode": "readwrite" }));
    }

    authenticate("Allow this website to change files in the folder you shared")?;

    if let Ok(mut grants) = grants().lock() {
        if let Some(stored) = grants.get_mut(&(origin.to_string(), id)) {
            stored.writable = true;
        }
    }
    eprintln!("[void] upgraded {} to read-write", grant.root.display());
    Ok(json!({ "mode": "readwrite" }))
}

fn list_directory(origin: &str, args: &Value) -> Reply {
    let grant = lookup_grant(origin, args)?;
    let path = resolve_existing(&grant.root, &segments(args)?)?;
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

fn read_file(origin: &str, args: &Value) -> Reply {
    let grant = lookup_grant(origin, args)?;
    let path = resolve_existing(&grant.root, &segments(args)?)?;
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
    let metadata = fs::metadata(&path).map_err(|error| {
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
    let bytes = fs::read(&path).map_err(|error| {
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

fn write_file(origin: &str, args: &Value, proxy: &EventLoopProxy<UserEvent>) -> Reply {
    let id = handle_id(args)?;
    let grant = lookup_grant(origin, args)?;
    if !grant.writable {
        return Err((
            "This folder is shared read-only. Share it again with write access.".into(),
            "NoModificationAllowedError",
        ));
    }

    let target = write_target(&grant.root, &segments(args)?)?;
    let encoded = args
        .get("bytes")
        .and_then(Value::as_str)
        .ok_or_else(|| ("The file contents are missing.".to_string(), "TypeError"))?;
    if encoded.len() as u64 / 4 * 3 > MAX_FILE_BYTES + 4 {
        return Err(exceeds_write_limit());
    }
    let contents = decode_base64(encoded).ok_or_else(|| {
        (
            "The file contents are not valid base64.".to_string(),
            "TypeError",
        )
    })?;
    if contents.len() as u64 > MAX_FILE_BYTES {
        return Err(exceeds_write_limit());
    }

    require_approval(origin, &id, &grant, "write", &target, proxy)?;

    write_atomic(&target, &contents).map_err(|error| {
        (
            format!("Could not write this file: {error}"),
            "NoModificationAllowedError",
        )
    })?;
    let last_modified = fs::metadata(&target)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    Ok(json!({ "size": contents.len(), "lastModified": last_modified }))
}

fn create_directory(origin: &str, args: &Value, proxy: &EventLoopProxy<UserEvent>) -> Reply {
    let id = handle_id(args)?;
    let grant = lookup_grant(origin, args)?;
    if !grant.writable {
        return Err((
            "This folder is shared read-only. Share it again with write access.".into(),
            "NoModificationAllowedError",
        ));
    }

    let target = write_target(&grant.root, &segments(args)?)?;
    if fs::symlink_metadata(&target).is_ok() {
        return Err((
            "This entry already exists.".into(),
            "InvalidModificationError",
        ));
    }

    require_approval(origin, &id, &grant, "create the folder", &target, proxy)?;

    fs::create_dir(&target).map_err(|error| {
        (
            format!("Could not create this folder: {error}"),
            "NoModificationAllowedError",
        )
    })?;
    Ok(json!({
        "name": target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
    }))
}

fn remove_entry(origin: &str, args: &Value, proxy: &EventLoopProxy<UserEvent>) -> Reply {
    let id = handle_id(args)?;
    let grant = lookup_grant(origin, args)?;
    if !grant.writable {
        return Err((
            "This folder is shared read-only. Share it again with write access.".into(),
            "NoModificationAllowedError",
        ));
    }

    let segments = segments(args)?;
    if segments.is_empty() {
        return Err((
            "The shared folder itself cannot be removed.".into(),
            "NoModificationAllowedError",
        ));
    }
    let target = resolve_existing(&grant.root, &segments)?;
    if target == grant.root {
        return Err((
            "The shared folder itself cannot be removed.".into(),
            "NoModificationAllowedError",
        ));
    }

    let kind = fs::symlink_metadata(&target)
        .map_err(|error| {
            (
                format!("Could not inspect this entry: {error}"),
                "NotFoundError",
            )
        })?
        .file_type();
    let action = if kind.is_dir() {
        "delete the folder"
    } else {
        "delete"
    };

    require_approval(origin, &id, &grant, action, &target, proxy)?;

    let result = if kind.is_dir() {
        fs::remove_dir(&target)
    } else {
        fs::remove_file(&target)
    };
    result.map_err(|error| {
        (
            format!("Could not delete this entry: {error}"),
            "NoModificationAllowedError",
        )
    })?;
    Ok(json!({ "removed": true }))
}

/// Layer 3: a person confirms every change the model proposes. Reads stay
/// behind Layer 2 only; writes, new folders, and deletions come through here.
fn require_approval(
    origin: &str,
    id: &str,
    grant: &Grant,
    action: &str,
    target: &Path,
    proxy: &EventLoopProxy<UserEvent>,
) -> Result<(), (String, &'static str)> {
    if grant.auto_approve {
        eprintln!(
            "[void] session approval reused to {action}: {}",
            target.display()
        );
        return Ok(());
    }

    let buttons = [
        "Allow Once".to_string(),
        "Deny".to_string(),
        "Allow for Session".to_string(),
    ];
    eprintln!(
        "[void] approval requested to {action}: {}",
        target.display()
    );

    let (sender, receiver) = mpsc::channel();
    proxy
        .send_event(UserEvent::Prompt {
            title: "VOID — approve a file change".to_string(),
            description: format!(
                "{origin} wants to {action}:\n\n{}\n\nFolder you shared:\n{}",
                target.display(),
                grant.root.display()
            ),
            buttons: buttons.clone(),
            response: sender,
        })
        .map_err(|_| {
            (
                "VOID could not show the approval prompt.".to_string(),
                "UnknownError",
            )
        })?;

    match receiver.recv_timeout(PROMPT_TIMEOUT) {
        Ok(label) if label == buttons[0] => {
            eprintln!("[void] approved once: {}", target.display());
            Ok(())
        }
        Ok(label) if label == buttons[2] => {
            eprintln!("[void] approved for this session: {}", target.display());
            if let Ok(mut grants) = grants().lock() {
                if let Some(stored) = grants.get_mut(&(origin.to_string(), id.to_string())) {
                    stored.auto_approve = true;
                }
            }
            Ok(())
        }
        Ok(_) => {
            eprintln!("[void] denied: {}", target.display());
            Err(("The user denied this change.".into(), "NotAllowedError"))
        }
        Err(_) => Err((
            "The approval prompt timed out, so nothing was changed.".into(),
            "NotAllowedError",
        )),
    }
}

fn authenticate(reason: &str) -> Result<(), (String, &'static str)> {
    #[cfg(target_os = "macos")]
    {
        desktop_auth::authenticate(reason).map_err(|message| (message, "SecurityError"))
    }

    #[cfg(not(target_os = "macos"))]
    {
        // Layer 1 is macOS-only for now; the other platforms keep Layer 2 and
        // Layer 3.
        let _ = reason;
        Ok(())
    }
}

fn lookup_grant(origin: &str, args: &Value) -> Result<Grant, (String, &'static str)> {
    let id = handle_id(args)?;
    grants()
        .lock()
        .map_err(|_| {
            (
                "Folder access state is unavailable.".to_string(),
                "UnknownError",
            )
        })?
        .get(&(origin.to_string(), id))
        .cloned()
        .ok_or_else(|| {
            (
                "This folder permission has expired.".to_string(),
                "NotFoundError",
            )
        })
}

fn handle_id(args: &Value) -> Result<String, (String, &'static str)> {
    args.get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ("The folder handle is invalid.".to_string(), "NotFoundError"))
}

fn segments(args: &Value) -> Result<Vec<String>, (String, &'static str)> {
    let list = args.get("path").and_then(Value::as_array).ok_or_else(|| {
        (
            "The folder handle path is invalid.".to_string(),
            "SecurityError",
        )
    })?;
    let mut segments = Vec::with_capacity(list.len());
    for segment in list {
        let name = segment
            .as_str()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                (
                    "The folder handle path is invalid.".to_string(),
                    "SecurityError",
                )
            })?;
        let mut components = Path::new(name).components();
        if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
            return Err((
                "The folder handle path is invalid.".to_string(),
                "SecurityError",
            ));
        }
        segments.push(name.to_string());
    }
    Ok(segments)
}

/// Layer 2: every resolved path has to stay inside the folder the user picked,
/// even after symbolic links are followed.
fn resolve_existing(root: &Path, segments: &[String]) -> Result<PathBuf, (String, &'static str)> {
    let mut joined = root.to_path_buf();
    for name in segments {
        joined.push(name);
    }
    let path = fs::canonicalize(&joined).map_err(|error| {
        (
            format!("Could not open this folder entry: {error}"),
            "NotFoundError",
        )
    })?;
    if !path.starts_with(root) {
        return Err((
            "This entry is outside the selected folder.".into(),
            "SecurityError",
        ));
    }
    Ok(path)
}

/// Resolve a path that may not exist yet. The parent must already be inside the
/// grant, and the leaf must not be a symbolic link that escapes it.
fn write_target(root: &Path, segments: &[String]) -> Result<PathBuf, (String, &'static str)> {
    let (name, parents) = segments
        .split_last()
        .ok_or_else(|| ("A file name is required.".to_string(), "TypeError"))?;
    let parent = resolve_existing(root, parents)?;
    let target = parent.join(name);

    if let Ok(metadata) = fs::symlink_metadata(&target) {
        if metadata.file_type().is_symlink() {
            return Err((
                "Symbolic links cannot be written through.".into(),
                "SecurityError",
            ));
        }
        if metadata.is_dir() {
            return Err(("This entry is a folder.".into(), "TypeMismatchError"));
        }
    }
    Ok(target)
}

/// Replace a file in one step: write a sibling temp file, flush it, then rename
/// over the target. A crash or a full disk can no longer leave a half-written
/// document, and `create_new` refuses to follow a symlink swapped in after the
/// path check.
fn write_atomic(target: &Path, contents: &[u8]) -> std::io::Result<()> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("file");
    let temp = parent.join(format!(".{name}.void-{}.tmp", uuid::Uuid::new_v4()));

    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(contents)?;
        file.sync_all()?;
        if let Ok(metadata) = fs::metadata(target) {
            let _ = fs::set_permissions(&temp, metadata.permissions());
        }
        fs::rename(&temp, target)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn exceeds_write_limit() -> (String, &'static str) {
    (
        format!(
            "This file exceeds the {} MiB write limit.",
            MAX_FILE_BYTES / 1024 / 1024
        ),
        "QuotaExceededError",
    )
}

fn grants() -> &'static Mutex<HashMap<RootKey, Grant>> {
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

fn decode_base64(input: &str) -> Option<Vec<u8>> {
    fn value(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let bytes: Vec<u8> = input
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    if bytes.len() % 4 != 0 {
        return None;
    }

    let mut output = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let mut accumulator = 0u32;
        let mut padding = 0u8;
        for byte in chunk {
            if *byte == b'=' {
                padding += 1;
                accumulator <<= 6;
            } else {
                accumulator = (accumulator << 6) | value(*byte)? as u32;
            }
        }
        output.push((accumulator >> 16) as u8);
        if padding < 2 {
            output.push((accumulator >> 8) as u8);
        }
        if padding < 1 {
            output.push(accumulator as u8);
        }
    }
    Some(output)
}
