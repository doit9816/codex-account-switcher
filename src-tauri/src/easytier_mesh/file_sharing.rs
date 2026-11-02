use super::*;

fn safe_log_label(value: &str) -> String {
    value
        .chars()
        .take(80)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

pub(super) fn add_file_share_to_group(
    group: &mut MeshShareGroup,
    path: &str,
    kind: &str,
) -> Result<String, String> {
    let canonical = PathBuf::from(path.trim())
        .canonicalize()
        .map_err(|error| format!("无法访问分享路径：{error}"))?;
    let metadata = fs::metadata(&canonical).map_err(display_err)?;
    let normalized_kind = if metadata.is_dir() {
        "directory"
    } else {
        "file"
    };
    if !metadata.is_file() && !metadata.is_dir() {
        return Err("只支持分享普通文件或目录".to_string());
    }
    if kind != normalized_kind {
        return Err("分享路径类型发生变化，请重新选择".to_string());
    }
    let canonical_path = canonical.to_string_lossy().to_string();
    if group
        .file_shares
        .iter()
        .any(|share| share.path == canonical_path)
    {
        return Err("此路径已在分享列表中".to_string());
    }
    let name = canonical
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("Shared files")
        .to_string();
    let share = MeshFileShare {
        id: format!("file-{}", uuid::Uuid::new_v4().simple()),
        name,
        path: canonical_path,
        kind: normalized_kind.to_string(),
        added_at: now_string(),
    };
    let share_id = share.id.clone();
    group.file_shares.push(share);
    Ok(share_id)
}

pub(super) fn remove_file_share_from_group(
    group: &mut MeshShareGroup,
    share_id: &str,
) -> Result<(), String> {
    let before = group.file_shares.len();
    group.file_shares.retain(|share| share.id != share_id);
    if before == group.file_shares.len() {
        return Err("分享项不存在".to_string());
    }
    Ok(())
}

pub(super) fn update_file_device_access(
    group: &mut MeshShareGroup,
    local_device_id: &str,
    device_id: &str,
    allowed: bool,
) -> Result<(), String> {
    if device_id == local_device_id {
        return Err("本机文件权限始终由本机控制".to_string());
    }
    let device = group
        .authorized_devices
        .iter()
        .find(|device| device.id == device_id)
        .ok_or_else(|| "device not found in mesh group".to_string())?;
    if !device.trusted || device.revoked_at.is_some() {
        return Err("设备未授权或已撤销".to_string());
    }
    group.file_access_devices.retain(|id| id != device_id);
    if allowed {
        group.file_access_devices.push(device_id.to_string());
    }
    Ok(())
}

pub(super) fn revoke_file_device_access(group: &mut MeshShareGroup, device_id: &str) -> bool {
    let previous_len = group.file_access_devices.len();
    group.file_access_devices.retain(|id| id != device_id);
    previous_len != group.file_access_devices.len()
}

pub(super) fn update_ai_file_access(
    group: &mut MeshShareGroup,
    enabled: bool,
    key: &[u8; 32],
) -> Result<Option<String>, String> {
    if enabled {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let token = URL_SAFE_NO_PAD.encode(bytes);
        group.encrypted_ai_file_token = Some(encrypt_secret(token.as_bytes(), key)?);
        group.ai_file_token_revoked = false;
        Ok(Some(token))
    } else {
        group.ai_file_token_revoked = true;
        Ok(None)
    }
}

fn save_file_sharing_change(
    app: &AppHandle,
    store: &AppStore,
    group_id: &str,
    operation: &str,
) -> Result<(), String> {
    match save_store(app, store) {
        Ok(()) => {
            append_app_log(
                app,
                "info",
                &format!("mesh_file_change_completed group_id={group_id} operation={operation}"),
            );
            Ok(())
        }
        Err(error) => {
            append_app_log(
                app,
                "error",
                &format!("mesh_file_change_failed group_id={group_id} operation={operation} stage=persist error={error}"),
            );
            Err(error)
        }
    }
}

fn log_file_sharing_failure(
    app: &AppHandle,
    group_id: &str,
    operation: &str,
    stage: &str,
    error: String,
) -> String {
    append_app_log(
        app,
        "warn",
        &format!("mesh_file_change_failed group_id={group_id} operation={operation} stage={stage} error={error}"),
    );
    error
}

pub(crate) fn add_group_file_share(
    app: AppHandle,
    group_id: String,
    path: String,
    kind: String,
) -> Result<MeshGroupStatus, String> {
    append_app_log(
        &app,
        "info",
        &format!("mesh_file_share_add_started group_id={group_id}"),
    );
    let mut store = load_store(&app).map_err(|error| {
        log_file_sharing_failure(&app, &group_id, "add_share", "load_store", error)
    })?;
    let group = store
        .settings
        .mesh
        .groups
        .iter_mut()
        .find(|group| group.group_id == group_id)
        .ok_or_else(|| "mesh group not found".to_string())
        .map_err(|error| {
            log_file_sharing_failure(&app, &group_id, "add_share", "resolve_group", error)
        })?;
    let share_id = add_file_share_to_group(group, &path, &kind).map_err(|error| {
        log_file_sharing_failure(&app, &group_id, "add_share", "validate_share", error)
    })?;
    save_file_sharing_change(&app, &store, &group_id, "add_share")?;
    append_app_log(
        &app,
        "debug",
        &format!("mesh_file_share_added group_id={group_id} share_id={share_id}"),
    );
    group_status(app.clone(), group_id.clone()).map_err(|error| {
        log_file_sharing_failure(&app, &group_id, "add_share", "refresh_status", error)
    })
}

pub(crate) fn remove_group_file_share(
    app: AppHandle,
    group_id: String,
    share_id: String,
) -> Result<MeshGroupStatus, String> {
    append_app_log(
        &app,
        "info",
        &format!("mesh_file_share_remove_started group_id={group_id} share_id={share_id}"),
    );
    let mut store = load_store(&app).map_err(|error| {
        log_file_sharing_failure(&app, &group_id, "remove_share", "load_store", error)
    })?;
    let group = store
        .settings
        .mesh
        .groups
        .iter_mut()
        .find(|group| group.group_id == group_id)
        .ok_or_else(|| "mesh group not found".to_string())
        .map_err(|error| {
            log_file_sharing_failure(&app, &group_id, "remove_share", "resolve_group", error)
        })?;
    remove_file_share_from_group(group, &share_id).map_err(|error| {
        log_file_sharing_failure(&app, &group_id, "remove_share", "validate_share", error)
    })?;
    save_file_sharing_change(&app, &store, &group_id, "remove_share")?;
    append_app_log(
        &app,
        "debug",
        &format!("mesh_file_share_removed group_id={group_id} share_id={share_id}"),
    );
    group_status(app.clone(), group_id.clone()).map_err(|error| {
        log_file_sharing_failure(&app, &group_id, "remove_share", "refresh_status", error)
    })
}

pub(crate) fn set_group_file_device_access(
    app: AppHandle,
    group_id: String,
    device_id: String,
    allowed: bool,
) -> Result<MeshGroupStatus, String> {
    append_app_log(&app, "info", &format!("mesh_file_device_access_started group_id={group_id} device_id={device_id} allowed={allowed}"));
    let mut store = load_store(&app).map_err(|error| {
        log_file_sharing_failure(&app, &group_id, "device_access", "load_store", error)
    })?;
    let group = store
        .settings
        .mesh
        .groups
        .iter_mut()
        .find(|group| group.group_id == group_id)
        .ok_or_else(|| "mesh group not found".to_string())
        .map_err(|error| {
            log_file_sharing_failure(&app, &group_id, "device_access", "resolve_group", error)
        })?;
    update_file_device_access(group, &local_device_id(), &device_id, allowed).map_err(|error| {
        log_file_sharing_failure(&app, &group_id, "device_access", "validate_device", error)
    })?;
    save_file_sharing_change(&app, &store, &group_id, "device_access")?;
    append_app_log(&app, "debug", &format!("mesh_file_device_access_updated group_id={group_id} device_id={device_id} allowed={allowed}"));
    group_status(app.clone(), group_id.clone()).map_err(|error| {
        log_file_sharing_failure(&app, &group_id, "device_access", "refresh_status", error)
    })
}

pub(crate) fn set_group_ai_file_access(
    app: AppHandle,
    group_id: String,
    enabled: bool,
) -> Result<MeshGroupStatus, String> {
    append_app_log(
        &app,
        "info",
        &format!("mesh_ai_file_access_started group_id={group_id} enabled={enabled}"),
    );
    let mut store = load_store(&app).map_err(|error| {
        log_file_sharing_failure(&app, &group_id, "ai_access", "load_store", error)
    })?;
    let key = load_master_key(&app).map_err(|error| {
        log_file_sharing_failure(&app, &group_id, "ai_access", "load_master_key", error)
    })?;
    let group = store
        .settings
        .mesh
        .groups
        .iter_mut()
        .find(|group| group.group_id == group_id)
        .ok_or_else(|| "mesh group not found".to_string())
        .map_err(|error| {
            log_file_sharing_failure(&app, &group_id, "ai_access", "resolve_group", error)
        })?;
    let token = update_ai_file_access(group, enabled, &key).map_err(|error| {
        log_file_sharing_failure(&app, &group_id, "ai_access", "update_access", error)
    })?;
    save_file_sharing_change(&app, &store, &group_id, "ai_access")?;
    append_app_log(&app, "debug", &format!("mesh_ai_file_access_updated group_id={group_id} enabled={enabled} token_rotated={enabled}"));
    let mut status = group_status(app.clone(), group_id.clone()).map_err(|error| {
        log_file_sharing_failure(&app, &group_id, "ai_access", "refresh_status", error)
    })?;
    if let Some(file_access) = status.file_access.as_mut() {
        file_access.ai_token = token;
        file_access.ai_access_enabled = enabled;
    }
    Ok(status)
}

pub(super) fn safe_shared_path(
    group: &MeshShareGroup,
    share_id: &str,
    relative: &str,
) -> Result<PathBuf, String> {
    if relative.contains('\0') {
        return Err("invalid file path".to_string());
    }
    let share = group
        .file_shares
        .iter()
        .find(|share| share.id == share_id)
        .ok_or_else(|| "shared item was not found".to_string())?;
    let base = Path::new(&share.path)
        .canonicalize()
        .map_err(|_| "shared path is unavailable".to_string())?;
    let mut candidate = base.clone();
    if !relative.is_empty() {
        let relative_path = Path::new(relative);
        if relative_path.is_absolute()
            || relative_path
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err("invalid relative file path".to_string());
        }
        candidate.push(relative_path);
    }
    let resolved = candidate
        .canonicalize()
        .map_err(|_| "shared file or directory is unavailable".to_string())?;
    if !resolved.starts_with(&base) {
        return Err("file path escapes the shared directory".to_string());
    }
    let metadata = fs::metadata(&resolved).map_err(display_err)?;
    if !metadata.is_file() && !metadata.is_dir() {
        return Err("unsupported file type".to_string());
    }
    Ok(resolved)
}

pub(super) fn list_shared_files(
    group: &MeshShareGroup,
    share_id: &str,
    relative: &str,
) -> Result<Vec<MeshFileEntry>, String> {
    let path = safe_shared_path(group, share_id, relative)?;
    let mut entries = Vec::new();
    if path.is_file() {
        let metadata = fs::metadata(&path).map_err(display_err)?;
        let share = group
            .file_shares
            .iter()
            .find(|share| share.id == share_id)
            .unwrap();
        entries.push(MeshFileEntry {
            id: relative.to_string(),
            name: share.name.clone(),
            kind: "file".to_string(),
            size: metadata.len(),
            modified_at: metadata
                .modified()
                .ok()
                .map(|time| chrono::DateTime::<Utc>::from(time).to_rfc3339()),
        });
        return Ok(entries);
    }
    for entry in fs::read_dir(&path).map_err(display_err)? {
        if entries.len() >= MESH_FILE_DIRECTORY_ENTRY_LIMIT {
            return Err(format!(
                "directory exceeds the {MESH_FILE_DIRECTORY_ENTRY_LIMIT} entry limit"
            ));
        }
        let entry = entry.map_err(display_err)?;
        let entry_path = entry.path();
        let metadata = fs::symlink_metadata(&entry_path).map_err(display_err)?;
        if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().to_string();
        let relative_path = if relative.is_empty() {
            file_name.clone()
        } else {
            format!("{relative}/{file_name}")
        };
        entries.push(MeshFileEntry {
            id: relative_path,
            name: file_name,
            kind: if metadata.is_dir() {
                "directory"
            } else {
                "file"
            }
            .to_string(),
            size: metadata.len(),
            modified_at: metadata
                .modified()
                .ok()
                .map(|time| chrono::DateTime::<Utc>::from(time).to_rfc3339()),
        });
    }
    entries.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    Ok(entries)
}

pub(super) fn search_shared_files(
    group: &MeshShareGroup,
    share_id: &str,
    query: &str,
) -> Result<Vec<MeshFileEntry>, String> {
    safe_shared_path(group, share_id, "")?;
    let mut pending = vec![String::new()];
    let mut results = Vec::new();
    let mut visited = 0usize;
    let mut visited_entries = 0usize;
    let query = query.to_lowercase();
    while let Some(relative) = pending.pop() {
        visited += 1;
        if visited > 1000 {
            break;
        }
        let entries = list_shared_files(group, share_id, &relative)?;
        visited_entries = visited_entries.saturating_add(entries.len());
        if visited_entries > MESH_FILE_SEARCH_ENTRY_LIMIT {
            break;
        }
        for entry in entries {
            if entry.name.to_lowercase().contains(&query) {
                results.push(entry.clone());
            }
            if entry.kind == "directory" && pending.len() + visited < 1000 {
                pending.push(entry.id);
            }
            if results.len() >= 1000 {
                break;
            }
        }
        if results.len() >= 1000 {
            break;
        }
    }
    results.sort_by(|left, right| left.id.to_lowercase().cmp(&right.id.to_lowercase()));
    Ok(results)
}

pub(super) fn request_is_local_or_in_group(request: &Request, group: &MeshShareGroup) -> bool {
    let Some(remote) = request.remote_addr() else {
        return false;
    };
    ip_is_local_or_in_group(remote.ip(), &group.virtual_cidr)
}

pub(super) fn ip_is_local_or_in_group(address: IpAddr, virtual_cidr: &str) -> bool {
    if address.is_loopback() {
        return true;
    }
    let IpAddr::V4(address) = address else {
        return false;
    };
    let Ok((network, prefix)) = parse_ipv4_cidr(virtual_cidr) else {
        return false;
    };
    let mask = u32::MAX << (32 - prefix);
    u32::from(address) & mask == network
}

pub(super) fn ai_file_token_matches(group: &MeshShareGroup, key: &[u8; 32], bearer: &str) -> bool {
    if group.ai_file_token_revoked {
        return false;
    }
    group
        .encrypted_ai_file_token
        .as_ref()
        .and_then(|envelope| decrypt_secret(envelope, key).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .is_some_and(|token| bearer == format!("Bearer {token}"))
}

pub(super) fn group_file_device_request_authorized(
    request: &Request,
    store: &AppStore,
    group: &MeshShareGroup,
    key: &[u8; 32],
) -> bool {
    request_header(request, MESH_GROUP_ID_HEADER)
        .filter(|request_group_id| request_group_id == &group.group_id)
        .and_then(|_| request_header(request, MESH_HELLO_DEVICE_ID_HEADER))
        .filter(|device_id| group.file_access_devices.contains(device_id))
        .is_some_and(|_| authorize_device_request(request, store, &group.group_id, key).is_ok())
}

pub(super) fn call_mcp_file_tool(
    group: &MeshShareGroup,
    name: &str,
    args: &Value,
) -> Result<String, String> {
    let share_id = args
        .get("shareId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let path = args.get("path").and_then(Value::as_str).unwrap_or_default();
    if name == "list_shared_items" {
        let shares = group
            .file_shares
            .iter()
            .map(|share| MeshFileShareView {
                id: share.id.clone(),
                name: share.name.clone(),
                kind: share.kind.clone(),
                added_at: share.added_at.clone(),
            })
            .collect::<Vec<_>>();
        serde_json::to_string(&shares).map_err(display_err)
    } else if name == "list_shared_files" {
        let relative = args.get("path").and_then(Value::as_str).unwrap_or_default();
        list_shared_files(group, share_id, relative)
            .and_then(|files| serde_json::to_string(&files).map_err(display_err))
    } else if name == "search_shared_files" {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default();
        search_shared_files(group, share_id, query)
            .and_then(|files| serde_json::to_string(&files).map_err(display_err))
    } else if name == "read_shared_file" {
        safe_shared_path(group, share_id, path).and_then(|file_path| {
            let metadata = fs::metadata(&file_path).map_err(display_err)?;
            if !metadata.is_file() { return Err("path is not a file".to_string()); }
            if metadata.len() > 1024 * 1024 { return Err("file exceeds the 1 MiB MCP read limit; use the authorized HTTP download endpoint".to_string()); }
            fs::read_to_string(file_path).map_err(display_err)
        })
    } else {
        Err("unknown tool".to_string())
    }
}

pub(crate) fn list_group_files(
    app: AppHandle,
    group_id: String,
    share_id: String,
    relative: String,
) -> Result<Vec<MeshFileEntry>, String> {
    let started = Instant::now();
    append_app_log(
        &app,
        "info",
        &format!("mesh_file_list_local_started group_id={group_id} share_id={share_id}"),
    );
    let result = (|| {
        let store = load_store(&app)?;
        let group = find_group(&store, &group_id)?;
        list_shared_files(group, &share_id, &relative)
    })();
    match result {
        Ok(files) => {
            append_app_log(
                &app,
                "info",
                &format!("mesh_file_list_local_completed group_id={group_id} share_id={share_id} count={} elapsed_ms={}", files.len(), started.elapsed().as_millis()),
            );
            Ok(files)
        }
        Err(error) => {
            append_app_log(
                &app,
                "warn",
                &format!("mesh_file_list_local_failed group_id={group_id} share_id={share_id} elapsed_ms={} error={error}", started.elapsed().as_millis()),
            );
            Err(error)
        }
    }
}

pub(crate) fn download_group_file(
    app: AppHandle,
    group_id: String,
    share_id: String,
    relative: String,
    destination: String,
) -> Result<u64, String> {
    let started = Instant::now();
    append_app_log(
        &app,
        "info",
        &format!("mesh_file_download_local_started group_id={group_id} share_id={share_id}"),
    );
    let result = (|| {
        let store = load_store(&app)?;
        let group = find_group(&store, &group_id)?;
        let source = safe_shared_path(group, &share_id, &relative)?;
        let destination = PathBuf::from(destination);
        copy_shared_file_to(&source, &destination)
    })();
    match result {
        Ok(bytes) => {
            append_app_log(
                &app,
                "info",
                &format!("mesh_file_download_local_completed group_id={group_id} share_id={share_id} bytes={bytes} elapsed_ms={}", started.elapsed().as_millis()),
            );
            Ok(bytes)
        }
        Err(error) => {
            append_app_log(
                &app,
                "warn",
                &format!("mesh_file_download_local_failed group_id={group_id} share_id={share_id} elapsed_ms={} error={error}", started.elapsed().as_millis()),
            );
            Err(error)
        }
    }
}

pub(super) fn copy_shared_file_to(source: &Path, destination: &Path) -> Result<u64, String> {
    let metadata = fs::metadata(source).map_err(display_err)?;
    if !metadata.is_file() {
        return Err("请选择一个文件下载".to_string());
    }
    if !destination.is_absolute() {
        return Err("下载位置必须是绝对路径".to_string());
    }
    let mut input = File::open(source).map_err(display_err)?;
    stream_download_to(&mut input, destination)
}

pub(super) fn stream_download_to<R: Read>(
    source: &mut R,
    destination: &Path,
) -> Result<u64, String> {
    if !destination.is_absolute() {
        return Err("download destination must be absolute".to_string());
    }
    if destination.exists() && !destination.is_file() {
        return Err("download destination must be a file path".to_string());
    }
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "download destination has an invalid file name".to_string())?;
    let temporary = destination.with_file_name(format!(
        ".{file_name}.{}.download",
        uuid::Uuid::new_v4().simple()
    ));
    let backup = destination.with_file_name(format!(
        ".{file_name}.{}.backup",
        uuid::Uuid::new_v4().simple()
    ));
    let transfer = (|| {
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(display_err)?;
        let bytes = std::io::copy(source, &mut output).map_err(display_err)?;
        output.flush().map_err(display_err)?;
        Ok(bytes)
    })();
    let bytes = match transfer {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
    };
    let had_destination = destination.exists();
    if had_destination {
        if let Err(error) = fs::rename(destination, &backup) {
            let _ = fs::remove_file(&temporary);
            return Err(display_err(error));
        }
    }
    if let Err(error) = fs::rename(&temporary, destination) {
        if had_destination {
            if let Err(restore_error) = fs::rename(&backup, destination) {
                let _ = fs::remove_file(&temporary);
                return Err(format!(
                    "{}; failed to restore previous destination: {}",
                    display_err(error),
                    display_err(restore_error)
                ));
            }
        }
        let _ = fs::remove_file(&temporary);
        return Err(display_err(error));
    }
    if had_destination {
        fs::remove_file(&backup).map_err(display_err)?;
    }
    Ok(bytes)
}

pub(super) fn remote_file_client(
    app: &AppHandle,
    group_id: &str,
    device_id: &str,
) -> Result<(reqwest::blocking::Client, String, String, String), String> {
    let mut store = load_store(app)?;
    let key = load_master_key(app)?;
    let group = find_group(&store, group_id)?;
    let device = group
        .authorized_devices
        .iter()
        .find(|device| device.id == device_id)
        .ok_or_else(|| "device is not in this share group".to_string())?;
    if !device.trusted || device.revoked_at.is_some() {
        return Err("device is not authorized".to_string());
    }
    if !group_runtime_snapshot(group_id).running {
        return Err("start this share group before pulling files".to_string());
    }
    if !build_group_status(&store, group_id, &key)?
        .devices
        .iter()
        .any(|view| view.id == device_id && view.online)
    {
        return Err("device is offline".to_string());
    }
    let address = device
        .address
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "该设备没有可用连接地址，请重新导入分享码".to_string())?;
    let mut endpoint = url::Url::parse(address).map_err(display_err)?;
    endpoint.set_path("");
    endpoint.set_query(None);
    endpoint.set_fragment(None);
    let base_url = endpoint.as_str().trim_end_matches('/').to_string();
    append_app_log(
        app,
        "debug",
        &format!(
            "mesh_remote_file_endpoint_selected group_id={group_id} device_id={device_id} host={} port={}",
            endpoint.host_str().unwrap_or("<none>"),
            endpoint.port_or_known_default().unwrap_or(0)
        ),
    );
    let credential = ensure_local_group_credential(&mut store, group_id, &key)?;
    save_store(app, &store)?;
    let proxy_url =
        should_use_mesh_socks5(&base_url).then(|| mesh_socks5_proxy_url_for_group(group_id));
    let client = build_remote_file_http_client(proxy_url.as_deref())?;
    Ok((client, base_url, credential, local_device_id()))
}

pub(super) fn build_remote_file_http_client(proxy_url: Option<&str>) -> Result<Client, String> {
    let mut builder = Client::builder()
        .connect_timeout(MESH_SYNC_CONNECT_TIMEOUT)
        .timeout(MESH_SYNC_REQUEST_TIMEOUT);
    if let Some(proxy_url) = proxy_url {
        builder = builder.proxy(reqwest::Proxy::all(proxy_url).map_err(display_err)?);
    }
    builder.build().map_err(display_err)
}

pub(super) fn remote_file_request(
    client: &Client,
    base_url: &str,
    group_id: &str,
    device_credential: &str,
    device_id: &str,
    path: &str,
    share_id: Option<&str>,
    relative: Option<&str>,
) -> reqwest::blocking::RequestBuilder {
    let mut url = url::Url::parse(&format!("{base_url}{path}")).expect("validated mesh URL");
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("groupId", group_id);
        if let Some(share_id) = share_id {
            query.append_pair("shareId", share_id);
        }
        if let Some(relative) = relative {
            query.append_pair("path", relative);
        }
    }
    client
        .get(url)
        .header(MESH_GROUP_ID_HEADER, group_id)
        .header(MESH_HELLO_DEVICE_ID_HEADER, device_id)
        .header(MESH_DEVICE_CREDENTIAL_HEADER, device_credential)
}

pub(super) fn send_remote_file_request(
    app: &AppHandle,
    group_id: &str,
    device_id: &str,
    operation: &str,
    request: reqwest::blocking::RequestBuilder,
) -> Result<reqwest::blocking::Response, String> {
    let started = Instant::now();
    append_app_log(app, "info", &format!("mesh_remote_file_request_started group_id={group_id} device_id={device_id} operation={operation}"));
    let response = request.send().map_err(|error| {
        let error = error.without_url();
        append_app_log(app, "warn", &format!("mesh_remote_file_request_failed group_id={group_id} device_id={device_id} operation={operation} elapsed_ms={} timeout={} connect={} error={error:#}", started.elapsed().as_millis(), error.is_timeout(), error.is_connect()));
        if error.is_timeout() || error.is_connect() {
            "无法连接对端文件服务。请确认对端已升级到支持文件分享的版本，并且共享组正在运行。".to_string()
        } else {
            display_err(error)
        }
    })?;
    let status = response.status();
    if !status.is_success() {
        append_app_log(app, "warn", &format!("mesh_remote_file_request_rejected group_id={group_id} device_id={device_id} operation={operation} status={status} elapsed_ms={}", started.elapsed().as_millis()));
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err("对端没有文件分享接口，请先升级对端客户端。".to_string());
        }
        return Err(format!("remote {operation} request failed: HTTP {status}"));
    }
    append_app_log(app, "info", &format!("mesh_remote_file_request_completed group_id={group_id} device_id={device_id} operation={operation} status={status} elapsed_ms={}", started.elapsed().as_millis()));
    Ok(response)
}

fn log_remote_file_operation_failure(
    app: &AppHandle,
    group_id: &str,
    device_id: &str,
    operation: &str,
    stage: &str,
    started: Instant,
    error: &str,
) {
    append_app_log(
        app,
        "warn",
        &format!(
            "mesh_remote_file_operation_failed group_id={group_id} device_id={device_id} operation={operation} stage={stage} elapsed_ms={} error={error}",
            started.elapsed().as_millis()
        ),
    );
}

pub(crate) fn list_group_remote_shares(
    app: AppHandle,
    group_id: String,
    device_id: String,
) -> Result<Vec<MeshFileShareView>, String> {
    let started = Instant::now();
    append_app_log(
        &app,
        "info",
        &format!("mesh_remote_file_shares_started group_id={group_id} device_id={device_id}"),
    );
    let (client, base_url, credential, local_id) = remote_file_client(&app, &group_id, &device_id)
        .map_err(|error| {
            log_remote_file_operation_failure(
                &app,
                &group_id,
                &device_id,
                "list_shares",
                "prepare",
                started,
                &error,
            );
            error
        })?;
    let response = send_remote_file_request(
        &app,
        &group_id,
        &device_id,
        "list_shares",
        remote_file_request(
            &client,
            &base_url,
            &group_id,
            &credential,
            &local_id,
            &format!("{MESH_FILES_PATH}/shares"),
            None,
            None,
        ),
    )
    .map_err(|error| {
        log_remote_file_operation_failure(
            &app,
            &group_id,
            &device_id,
            "list_shares",
            "request",
            started,
            &error,
        );
        error
    })?;
    #[derive(Deserialize)]
    struct Shares {
        shares: Vec<MeshFileShareView>,
    }
    let shares = response
        .json::<Shares>()
        .map_err(|error| {
            let error = display_err(error);
            log_remote_file_operation_failure(
                &app,
                &group_id,
                &device_id,
                "list_shares",
                "decode_response",
                started,
                &error,
            );
            error
        })?
        .shares;
    append_app_log(
        &app,
        "info",
        &format!(
            "mesh_remote_file_shares_completed group_id={group_id} device_id={device_id} count={} elapsed_ms={}",
            shares.len(),
            started.elapsed().as_millis()
        ),
    );
    Ok(shares)
}

pub(crate) fn list_group_remote_files(
    app: AppHandle,
    group_id: String,
    device_id: String,
    share_id: String,
    relative: String,
) -> Result<Vec<MeshFileEntry>, String> {
    let started = Instant::now();
    append_app_log(
        &app,
        "info",
        &format!("mesh_remote_file_list_started group_id={group_id} device_id={device_id} share_id={share_id}"),
    );
    let (client, base_url, credential, local_id) = remote_file_client(&app, &group_id, &device_id)
        .map_err(|error| {
            log_remote_file_operation_failure(
                &app,
                &group_id,
                &device_id,
                "list_files",
                "prepare",
                started,
                &error,
            );
            error
        })?;
    let response = send_remote_file_request(
        &app,
        &group_id,
        &device_id,
        "list_files",
        remote_file_request(
            &client,
            &base_url,
            &group_id,
            &credential,
            &local_id,
            MESH_FILES_PATH,
            Some(&share_id),
            Some(&relative),
        ),
    )
    .map_err(|error| {
        log_remote_file_operation_failure(
            &app,
            &group_id,
            &device_id,
            "list_files",
            "request",
            started,
            &error,
        );
        error
    })?;
    #[derive(Deserialize)]
    struct Files {
        files: Vec<MeshFileEntry>,
    }
    let files = response
        .json::<Files>()
        .map_err(|error| {
            let error = display_err(error);
            log_remote_file_operation_failure(
                &app,
                &group_id,
                &device_id,
                "list_files",
                "decode_response",
                started,
                &error,
            );
            error
        })?
        .files;
    append_app_log(
        &app,
        "info",
        &format!(
            "mesh_remote_file_list_completed group_id={group_id} device_id={device_id} share_id={share_id} count={} elapsed_ms={}",
            files.len(),
            started.elapsed().as_millis()
        ),
    );
    Ok(files)
}

pub(crate) fn download_group_remote_file(
    app: AppHandle,
    group_id: String,
    device_id: String,
    share_id: String,
    relative: String,
    destination: String,
) -> Result<u64, String> {
    let started = Instant::now();
    append_app_log(
        &app,
        "info",
        &format!("mesh_remote_file_download_started group_id={group_id} device_id={device_id} share_id={share_id}"),
    );
    let (client, base_url, credential, local_id) = match remote_file_client(
        &app, &group_id, &device_id,
    ) {
        Ok(remote) => remote,
        Err(error) => {
            append_app_log(
                    &app,
                    "warn",
                    &format!("mesh_remote_file_download_failed group_id={group_id} device_id={device_id} share_id={share_id} stage=prepare elapsed_ms={} error={error}", started.elapsed().as_millis()),
                );
            return Err(error);
        }
    };
    let mut response = send_remote_file_request(
        &app,
        &group_id,
        &device_id,
        "download",
        remote_file_request(
            &client,
            &base_url,
            &group_id,
            &credential,
            &local_id,
            &format!("{MESH_FILES_PATH}/download"),
            Some(&share_id),
            Some(&relative),
        )
        .timeout(MESH_FILE_DOWNLOAD_TIMEOUT),
    )
    .map_err(|error| {
        log_remote_file_operation_failure(
            &app, &group_id, &device_id, "download", "request", started, &error,
        );
        error
    })?;
    let destination = PathBuf::from(destination);
    match stream_download_to(&mut response, &destination) {
        Ok(bytes) => {
            append_app_log(&app, "info", &format!("mesh_remote_file_download_completed group_id={group_id} device_id={device_id} share_id={share_id} bytes={bytes} elapsed_ms={}", started.elapsed().as_millis()));
            Ok(bytes)
        }
        Err(error) => {
            append_app_log(&app, "warn", &format!("mesh_remote_file_download_failed group_id={group_id} device_id={device_id} share_id={share_id} stage=write_destination elapsed_ms={} error={error}", started.elapsed().as_millis()));
            Err(error)
        }
    }
}

pub(crate) fn handle_file_request(app: AppHandle, request: Request) -> Result<(), String> {
    let url = request.url().to_string();
    let query =
        url::form_urlencoded::parse(url.split_once('?').map(|(_, q)| q).unwrap_or("").as_bytes())
            .into_owned()
            .collect::<BTreeMap<_, _>>();
    let group_id = query
        .get("groupId")
        .cloned()
        .or_else(|| request_header(&request, MESH_GROUP_ID_HEADER));
    let Some(group_id) = group_id else {
        append_app_log(
            &app,
            "warn",
            "mesh_file_request_rejected reason=missing_group_id status=400",
        );
        return respond_sync_json(
            request,
            400,
            serde_json::json!({"error":"groupId is required"}),
        );
    };
    let log_group_id = safe_log_label(&group_id);
    let store = load_store(&app).map_err(|error| {
        append_app_log(
            &app,
            "error",
            &format!(
                "mesh_file_request_failed group_id={log_group_id} stage=load_store error={error}"
            ),
        );
        error
    })?;
    let key = load_master_key(&app).map_err(|error| {
        append_app_log(
            &app,
            "error",
            &format!(
                "mesh_file_request_failed group_id={log_group_id} stage=load_master_key error={error}"
            ),
        );
        error
    })?;
    let group = match store
        .settings
        .mesh
        .groups
        .iter()
        .find(|group| group.group_id == group_id)
    {
        Some(group) => group,
        None => {
            append_app_log(
                &app,
                "warn",
                &format!("mesh_file_request_rejected group_id={log_group_id} reason=group_not_found status=404"),
            );
            return respond_sync_json(request, 404, serde_json::json!({"error":"group not found"}));
        }
    };
    let bearer = request_header(&request, "Authorization").unwrap_or_default();
    let ai_authorized = ai_file_token_matches(group, &key, &bearer);
    let device_authorized = group_file_device_request_authorized(&request, &store, group, &key);
    handle_file_request_with_auth(
        Some(&app),
        request,
        &group_id,
        group,
        ai_authorized,
        device_authorized,
    )
}

pub(super) fn handle_file_request_with_auth(
    app: Option<&AppHandle>,
    request: Request,
    group_id: &str,
    group: &MeshShareGroup,
    ai_authorized: bool,
    device_authorized: bool,
) -> Result<(), String> {
    let started = Instant::now();
    let log_group_id = safe_log_label(group_id);
    let method = request.method().clone();
    let url = request.url().to_string();
    let log_path = safe_log_label(url.split('?').next().unwrap_or(&url));
    let query =
        url::form_urlencoded::parse(url.split_once('?').map(|(_, q)| q).unwrap_or("").as_bytes())
            .into_owned()
            .collect::<BTreeMap<_, _>>();
    if group.group_id != group_id
        || request_header(&request, MESH_GROUP_ID_HEADER)
            .is_some_and(|header_group_id| header_group_id != group_id)
    {
        if let Some(app) = app {
            append_app_log(
                app,
                "warn",
                &format!(
                    "mesh_file_request_denied group_id={log_group_id} reason=group_identifier_mismatch"
                ),
            );
        }
        return respond_sync_json(
            request,
            403,
            serde_json::json!({"error":"group identifier does not match the request"}),
        );
    }
    if !request_is_local_or_in_group(&request, group) {
        if let Some(app) = app {
            append_app_log(
                app,
                "warn",
                &format!(
                    "mesh_file_request_denied group_id={log_group_id} reason=source_outside_group"
                ),
            );
        }
        return respond_sync_json(
            request,
            403,
            serde_json::json!({"error":"file access is only available locally or through this share group"}),
        );
    }
    if !ai_authorized && !device_authorized {
        if let Some(app) = app {
            append_app_log(app, "warn", &format!("mesh_file_request_denied group_id={log_group_id} path={log_path} ai_authorized={ai_authorized} device_authorized={device_authorized}"));
        }
        return respond_sync_json(
            request,
            403,
            serde_json::json!({"error":"file access is not authorized"}),
        );
    }
    let path = url.split('?').next().unwrap_or(&url);
    let mut response_status = 200;
    let result = if method == tiny_http::Method::Get && path == format!("{MESH_FILES_PATH}/shares")
    {
        respond_sync_json(
            request,
            200,
            serde_json::json!({"shares":group.file_shares.iter().map(|share| MeshFileShareView { id:share.id.clone(), name:share.name.clone(), kind:share.kind.clone(), added_at:share.added_at.clone() }).collect::<Vec<_>>()}),
        )
    } else if method == tiny_http::Method::Get && path == MESH_FILES_PATH {
        let share_id = query.get("shareId").map(String::as_str).unwrap_or("");
        let relative = query.get("path").map(String::as_str).unwrap_or("");
        match list_shared_files(group, share_id, relative) {
            Ok(files) => respond_sync_json(request, 200, serde_json::json!({"files":files})),
            Err(error) => {
                response_status = 422;
                respond_sync_json(request, 422, serde_json::json!({"error":error}))
            }
        }
    } else if method == tiny_http::Method::Get && path == format!("{MESH_FILES_PATH}/download") {
        let share_id = query.get("shareId").map(String::as_str).unwrap_or("");
        let relative = query.get("path").map(String::as_str).unwrap_or("");
        match safe_shared_path(group, share_id, relative) {
            Ok(file_path) if file_path.is_file() => {
                let name = file_path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("download");
                match (File::open(&file_path), fs::metadata(&file_path)) {
                    (Ok(file), Ok(metadata)) => {
                        let content_type =
                            Header::from_bytes("Content-Type", "application/octet-stream")
                                .map_err(|_| "failed to build content-type header")?;
                        let content_disposition = Header::from_bytes(
                            "Content-Disposition",
                            format!("attachment; filename*=UTF-8''{}", urlencoding::encode(name)),
                        )
                        .map_err(|_| "failed to build download header")?;
                        let length = usize::try_from(metadata.len()).map_err(display_err)?;
                        request
                            .respond(Response::new(
                                StatusCode(200),
                                vec![content_type, content_disposition],
                                file,
                                Some(length),
                                None,
                            ))
                            .map_err(display_err)
                    }
                    (Err(error), _) | (_, Err(error)) => {
                        response_status = 500;
                        respond_sync_json(
                            request,
                            500,
                            serde_json::json!({"error":error.to_string()}),
                        )
                    }
                }
            }
            Ok(_) => {
                response_status = 400;
                respond_sync_json(
                    request,
                    400,
                    serde_json::json!({"error":"a file path is required"}),
                )
            }
            Err(error) => {
                response_status = 422;
                respond_sync_json(request, 422, serde_json::json!({"error":error}))
            }
        }
    } else {
        response_status = 404;
        respond_sync_json(
            request,
            404,
            serde_json::json!({"error":"file route not found"}),
        )
    };
    if let Some(app) = app {
        append_app_log(app, if response_status < 400 && result.is_ok() {"info"} else {"warn"}, &format!("mesh_file_request group_id={log_group_id} method={} path={log_path} status={response_status} elapsed_ms={} response={}", method, started.elapsed().as_millis(), if result.is_ok() {"sent"} else {"failed"}));
    }
    result
}

pub(crate) fn handle_mcp_request(app: AppHandle, request: Request) -> Result<(), String> {
    let request_group = request.url().split_once('?').and_then(|(_, query)| {
        url::form_urlencoded::parse(query.as_bytes())
            .find(|(key, _)| key == "groupId")
            .map(|(_, value)| value.into_owned())
    });
    let Some(request_group) = request_group else {
        append_app_log(
            &app,
            "warn",
            "mesh_mcp_request_rejected reason=missing_group_id status=400",
        );
        return respond_sync_json(
            request,
            400,
            serde_json::json!({"error":"groupId query parameter is required"}),
        );
    };
    let log_group = safe_log_label(&request_group);
    let bearer = request_header(&request, "Authorization").unwrap_or_default();
    let auth_store = load_store(&app).map_err(|error| {
        append_app_log(
            &app,
            "error",
            &format!("mesh_mcp_request_failed group_id={log_group} stage=load_store error={error}"),
        );
        error
    })?;
    let auth_key = load_master_key(&app).map_err(|error| {
        append_app_log(
            &app,
            "error",
            &format!(
                "mesh_mcp_request_failed group_id={log_group} stage=load_master_key error={error}"
            ),
        );
        error
    })?;
    let Some(auth_group) = auth_store
        .settings
        .mesh
        .groups
        .iter()
        .find(|group| group.group_id == request_group)
    else {
        append_app_log(
            &app,
            "warn",
            &format!(
                "mesh_mcp_request_rejected group_id={log_group} reason=group_not_found status=404"
            ),
        );
        return respond_sync_json(
            request,
            404,
            serde_json::json!({"error":"mesh group not found"}),
        );
    };
    if !request_is_local_or_in_group(&request, auth_group) {
        append_app_log(
            &app,
            "warn",
            &format!("mesh_mcp_denied group_id={log_group} reason=source_outside_group"),
        );
        return respond_sync_json(
            request,
            403,
            serde_json::json!({"error":"MCP is only available locally or through this share group"}),
        );
    }
    let authorized = ai_file_token_matches(auth_group, &auth_key, &bearer);
    handle_mcp_request_with_auth(Some(&app), request, &request_group, auth_group, authorized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{sleep, timeout};

    #[test]
    fn log_labels_remove_control_characters_and_are_bounded() {
        let label = safe_log_label("group\r\nid\twith spaces");
        assert_eq!(label, "group__id_with_spaces");
        assert_eq!(safe_log_label(&"x".repeat(100)).len(), 80);
    }

    #[test]
    fn mcp_tool_log_categories_are_specific_without_copying_error_details() {
        assert_eq!(mcp_tool_error_category("unknown tool"), "unknown_tool");
        assert_eq!(
            mcp_tool_error_category("invalid relative file path"),
            "path_rejected"
        );
        assert_eq!(
            mcp_tool_error_category("file exceeds the 1 MiB MCP read limit"),
            "size_limit"
        );
        assert_eq!(
            mcp_tool_error_category("invalid UTF-8 in /private/user/path.txt"),
            "tool_execution_failed"
        );
    }

    fn free_local_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn overlay_test_group(listen_port: u16, socks5_port: u16) -> MeshShareGroup {
        MeshShareGroup {
            group_id: "file-sharing-overlay-test".to_string(),
            name: "file sharing overlay integration".to_string(),
            encrypted_network_secret: None,
            nodes: Vec::new(),
            authorized_devices: Vec::new(),
            enabled: true,
            auto_start: false,
            sync_scope: MeshSyncScope::default(),
            virtual_cidr: "10.221.19.0/24".to_string(),
            listen_port,
            socks5_port,
            runtime: MeshGroupRuntimeState::default(),
            credential_grants: Vec::new(),
            encrypted_local_device_credential: None,
            file_shares: Vec::new(),
            file_access_devices: Vec::new(),
            encrypted_ai_file_token: None,
            ai_file_token_revoked: false,
            legacy_compat: false,
        }
    }

    struct InterruptedReader {
        sent_partial_data: bool,
    }

    impl Read for InterruptedReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.sent_partial_data {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "simulated interrupted transfer",
                ));
            }
            self.sent_partial_data = true;
            let partial = b"incomplete";
            buffer[..partial.len()].copy_from_slice(partial);
            Ok(partial.len())
        }
    }

    #[test]
    fn interrupted_stream_download_preserves_existing_file_and_cleans_temporary_file() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("existing-download.bin");
        fs::write(&destination, b"previous complete file").unwrap();
        let mut interrupted = InterruptedReader {
            sent_partial_data: false,
        };

        assert!(stream_download_to(&mut interrupted, &destination).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"previous complete file");
        assert_eq!(
            stream_download_to(
                &mut std::io::Cursor::new(b"new complete file"),
                &destination
            )
            .unwrap(),
            b"new complete file".len() as u64
        );
        assert_eq!(fs::read(&destination).unwrap(), b"new complete file");
        assert_eq!(
            fs::read_dir(temp.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".existing-download.bin."))
                .count(),
            0
        );
    }

    #[test]
    fn share_mutations_validate_paths_reject_duplicates_and_remove_by_id() {
        let temp = tempfile::tempdir().unwrap();
        let file_path = temp.path().join("notes.txt");
        let directory_path = temp.path().join("reports");
        fs::write(&file_path, b"share me").unwrap();
        fs::create_dir(&directory_path).unwrap();
        let mut group = overlay_test_group(11010, 22333);

        let file_id =
            add_file_share_to_group(&mut group, &file_path.to_string_lossy(), "file").unwrap();
        assert_eq!(group.file_shares[0].kind, "file");
        assert_eq!(group.file_shares[0].name, "notes.txt");
        assert!(
            add_file_share_to_group(&mut group, &file_path.to_string_lossy(), "file",).is_err()
        );
        assert!(
            add_file_share_to_group(&mut group, &directory_path.to_string_lossy(), "file",)
                .is_err()
        );
        let folder_id =
            add_file_share_to_group(&mut group, &directory_path.to_string_lossy(), "directory")
                .unwrap();
        assert_eq!(group.file_shares.len(), 2);
        remove_file_share_from_group(&mut group, &file_id).unwrap();
        assert_eq!(group.file_shares.len(), 1);
        assert_eq!(group.file_shares[0].id, folder_id);
        assert!(remove_file_share_from_group(&mut group, "missing-share").is_err());
        assert_eq!(group.file_shares.len(), 1);
    }

    #[test]
    fn device_file_access_can_be_granted_and_revoked_only_for_trusted_peers() {
        let mut group = overlay_test_group(11010, 22333);
        group.authorized_devices = vec![
            MeshDevice {
                id: "trusted-device".to_string(),
                name: "trusted".to_string(),
                address: None,
                last_seen_at: None,
                trusted: true,
                sync_scope: MeshSyncScope::default(),
                allowed_sync_scope: None,
                auto_account_sync: false,
                encrypted_routing_api_key: None,
                encrypted_mesh_credential: None,
                credential_fingerprint: None,
                revoked_at: None,
            },
            MeshDevice {
                id: "untrusted-device".to_string(),
                name: "untrusted".to_string(),
                address: None,
                last_seen_at: None,
                trusted: false,
                sync_scope: MeshSyncScope::default(),
                allowed_sync_scope: None,
                auto_account_sync: false,
                encrypted_routing_api_key: None,
                encrypted_mesh_credential: None,
                credential_fingerprint: None,
                revoked_at: None,
            },
        ];

        update_file_device_access(&mut group, "local-device", "trusted-device", true).unwrap();
        update_file_device_access(&mut group, "local-device", "trusted-device", true).unwrap();
        assert_eq!(group.file_access_devices, vec!["trusted-device"]);
        let trusted_device = group
            .authorized_devices
            .iter_mut()
            .find(|device| device.id == "trusted-device")
            .unwrap();
        trusted_device.trusted = false;
        trusted_device.revoked_at = Some(now_string());
        assert!(revoke_file_device_access(&mut group, "trusted-device"));
        assert!(group.file_access_devices.is_empty());
        let trusted_device = group
            .authorized_devices
            .iter_mut()
            .find(|device| device.id == "trusted-device")
            .unwrap();
        trusted_device.trusted = true;
        trusted_device.revoked_at = None;
        // Re-trusting the mesh device does not restore its former file grant.
        assert!(group.file_access_devices.is_empty());
        update_file_device_access(&mut group, "local-device", "trusted-device", true).unwrap();
        assert_eq!(group.file_access_devices, vec!["trusted-device"]);
        update_file_device_access(&mut group, "local-device", "trusted-device", false).unwrap();
        assert!(group.file_access_devices.is_empty());
        assert!(
            update_file_device_access(&mut group, "local-device", "local-device", true).is_err()
        );
        assert!(
            update_file_device_access(&mut group, "local-device", "missing-device", true).is_err()
        );
        assert!(
            update_file_device_access(&mut group, "local-device", "untrusted-device", true)
                .is_err()
        );
        assert!(group.file_access_devices.is_empty());
    }

    #[test]
    fn ai_file_access_generates_rotates_and_revokes_encrypted_tokens() {
        let key = [31u8; 32];
        let mut group = overlay_test_group(11010, 22333);
        let first_token = update_ai_file_access(&mut group, true, &key)
            .unwrap()
            .unwrap();
        assert!(!group.ai_file_token_revoked);
        assert!(ai_file_token_matches(
            &group,
            &key,
            &format!("Bearer {first_token}")
        ));
        assert!(!serde_json::to_string(&group)
            .unwrap()
            .contains(&first_token));

        let second_token = update_ai_file_access(&mut group, true, &key)
            .unwrap()
            .unwrap();
        assert_ne!(first_token, second_token);
        assert!(!ai_file_token_matches(
            &group,
            &key,
            &format!("Bearer {first_token}")
        ));
        assert!(ai_file_token_matches(
            &group,
            &key,
            &format!("Bearer {second_token}")
        ));

        assert!(update_ai_file_access(&mut group, false, &key)
            .unwrap()
            .is_none());
        assert!(group.ai_file_token_revoked);
        assert!(!ai_file_token_matches(
            &group,
            &key,
            &format!("Bearer {second_token}")
        ));
    }

    #[tokio::test]
    async fn two_embedded_easytier_nodes_transfer_a_file_over_the_overlay() {
        // The integration nodes share one host; disable NIC-bound socket selection
        // so the local peer uses a deterministic loopback tunnel.
        let peer_host = "127.0.0.1";
        let mut server_group = overlay_test_group(free_local_port(), free_local_port());
        let client_group = overlay_test_group(free_local_port(), free_local_port());
        let group_id = server_group.group_id.clone();
        let device_id = "overlay-client-device".to_string();
        let device_credential = "overlay-test-device-credential";
        let ai_token = "overlay-test-ai-token";
        let key = [19u8; 32];
        let temp = tempfile::tempdir().unwrap();
        let shared_folder = temp.path().join("overlay-shared-folder");
        fs::create_dir(&shared_folder).unwrap();
        fs::write(
            shared_folder.join("overlay.txt"),
            b"downloaded through two EasyTier nodes",
        )
        .unwrap();
        server_group.authorized_devices.push(MeshDevice {
            id: device_id.clone(),
            name: "overlay client".to_string(),
            address: None,
            last_seen_at: None,
            trusted: true,
            sync_scope: MeshSyncScope::default(),
            allowed_sync_scope: None,
            auto_account_sync: false,
            encrypted_routing_api_key: None,
            encrypted_mesh_credential: Some(
                encrypt_secret(device_credential.as_bytes(), &key).unwrap(),
            ),
            credential_fingerprint: None,
            revoked_at: None,
        });
        server_group.file_access_devices.push(device_id.clone());
        server_group.file_shares.push(MeshFileShare {
            id: "overlay-share".to_string(),
            name: "overlay-share".to_string(),
            path: shared_folder.to_string_lossy().into_owned(),
            kind: "directory".to_string(),
            added_at: now_string(),
        });
        server_group.encrypted_ai_file_token =
            Some(encrypt_secret(ai_token.as_bytes(), &key).unwrap());
        let manager = NetworkInstanceManager::new();
        let server_config =
            build_easytier_group_config(&server_group, "overlay-test-secret", &[]).unwrap();
        let mut server_flags = server_config.get_flags();
        server_flags.bind_device = false;
        server_config.set_flags(server_flags);
        server_config.set_inst_name(format!("file-share-test-server-{}", uuid::Uuid::new_v4()));
        server_config.set_ipv4(Some("10.221.19.1/24".parse().unwrap()));
        let server_id = manager
            .run_network_instance(server_config, false, ConfigFileControl::STATIC_CONFIG)
            .unwrap();

        let client_config = build_easytier_group_config(
            &client_group,
            "overlay-test-secret",
            &[format!("tcp://{peer_host}:{}", server_group.listen_port)],
        )
        .unwrap();
        let mut client_flags = client_config.get_flags();
        client_flags.bind_device = false;
        client_config.set_flags(client_flags);
        client_config.set_inst_name(format!("file-share-test-client-{}", uuid::Uuid::new_v4()));
        client_config.set_ipv4(Some("10.221.19.2/24".parse().unwrap()));
        let client_id = manager
            .run_network_instance(client_config, false, ConfigFileControl::STATIC_CONFIG)
            .unwrap();

        let connected = timeout(Duration::from_secs(20), async {
            loop {
                if manager
                    .collect_network_infos()
                    .await
                    .ok()
                    .and_then(|infos| infos.get(&client_id).map(|info| !info.peers.is_empty()))
                    .unwrap_or(false)
                {
                    break;
                }
                sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .is_ok();
        let infos = manager.collect_network_infos().await.unwrap_or_default();
        if !connected {
            manager
                .delete_network_instance(vec![server_id, client_id])
                .unwrap();
            panic!(
                "EasyTier peers failed to connect; server={:?}, client={:?}",
                infos.get(&server_id).map(|info| &info.events),
                infos.get(&client_id).map(|info| &info.events),
            );
        }

        let api_port = free_local_port();
        let listener = std::net::TcpListener::bind(format!("0.0.0.0:{api_port}")).unwrap();
        let api_server = Arc::new(tiny_http::Server::from_listener(listener, None).unwrap());
        let server_store = {
            let mut store = AppStore::default();
            store.settings.mesh.groups.push(server_group.clone());
            store
        };
        let request_server = api_server.clone();
        let request_group = server_group.clone();
        let request_group_id = group_id.clone();
        let request_thread = std::thread::spawn(move || {
            for _ in 0..6 {
                let request = match request_server.recv_timeout(Duration::from_secs(20)) {
                    Ok(Some(request)) => request,
                    _ => return,
                };
                if request.url().split('?').next() == Some("/mcp") {
                    let authorized = request.remote_addr().is_some_and(|address| {
                        ip_is_local_or_in_group(address.ip(), &request_group.virtual_cidr)
                    });
                    let _ = handle_mcp_request_with_auth(
                        None,
                        request,
                        &request_group_id,
                        &request_group,
                        authorized,
                    );
                    continue;
                }
                let authorized = group_file_device_request_authorized(
                    &request,
                    &server_store,
                    &request_group,
                    &key,
                );
                let _ = handle_file_request_with_auth(
                    None,
                    request,
                    &request_group_id,
                    &request_group,
                    false,
                    authorized,
                );
            }
        });

        let proxy_url = format!("socks5h://127.0.0.1:{}", client_group.socks5_port);
        let download_destination = temp.path().join("overlay-download.txt");
        let transfer_group_id = group_id.clone();
        let transfer_device_id = device_id.clone();
        let transfer_result = tokio::task::spawn_blocking(move || -> Result<(), String> {
            let base_url = format!("http://10.221.19.1:{api_port}");
            let shares_client = build_remote_file_http_client(Some(&proxy_url))?;
            let shares_response = remote_file_request(
                &shares_client,
                &base_url,
                &transfer_group_id,
                device_credential,
                &transfer_device_id,
                &format!("{MESH_FILES_PATH}/shares"),
                None,
                None,
            )
            .header("connection", "close")
            .send()
            .map_err(|error| format!("overlay shares request failed: {error:#?}"))?;
            if !shares_response.status().is_success() {
                return Err(format!(
                    "overlay share listing returned HTTP {}",
                    shares_response.status()
                ));
            }
            #[derive(Deserialize)]
            struct Shares {
                shares: Vec<MeshFileShareView>,
            }
            let shares = shares_response
                .json::<Shares>()
                .map_err(display_err)?
                .shares;
            let share = shares
                .first()
                .ok_or_else(|| "overlay share list was empty".to_string())?;

            let files_client = build_remote_file_http_client(Some(&proxy_url))?;
            let files_response = remote_file_request(
                &files_client,
                &base_url,
                &transfer_group_id,
                device_credential,
                &transfer_device_id,
                MESH_FILES_PATH,
                Some(&share.id),
                Some(""),
            )
            .header("connection", "close")
            .send()
            .map_err(|error| format!("overlay file-list request failed: {error:#?}"))?;
            if !files_response.status().is_success() {
                return Err(format!(
                    "overlay file listing returned HTTP {}",
                    files_response.status()
                ));
            }
            #[derive(Deserialize)]
            struct Files {
                files: Vec<MeshFileEntry>,
            }
            let files = files_response.json::<Files>().map_err(display_err)?.files;
            let file = files
                .first()
                .ok_or_else(|| "overlay file list was empty".to_string())?;

            let download_client = build_remote_file_http_client(Some(&proxy_url))?;
            let mut download_response = remote_file_request(
                &download_client,
                &base_url,
                &transfer_group_id,
                device_credential,
                &transfer_device_id,
                &format!("{MESH_FILES_PATH}/download"),
                Some(&share.id),
                Some(&file.id),
            )
            .timeout(MESH_FILE_DOWNLOAD_TIMEOUT)
            .header("connection", "close")
            .send()
            .map_err(|error| format!("overlay download request failed: {error:#?}"))?;
            if !download_response.status().is_success() {
                return Err(format!(
                    "overlay file download returned HTTP {}",
                    download_response.status()
                ));
            }
            stream_download_to(&mut download_response, &download_destination)?;
            let downloaded = fs::read(&download_destination).map_err(display_err)?;
            if downloaded != b"downloaded through two EasyTier nodes" {
                return Err("overlay download contents did not match the shared file".to_string());
            }

            let mcp_url = format!("{base_url}/mcp?groupId={transfer_group_id}");
            for (id, method, params) in [
                (1, "initialize", serde_json::json!({})),
                (2, "tools/list", serde_json::json!({})),
                (3, "tools/call", serde_json::json!({
                    "name":"read_shared_file",
                    "arguments":{"shareId":share.id,"path":"overlay.txt"}
                })),
            ] {
                let mcp_client = build_remote_file_http_client(Some(&proxy_url))?;
                let response = mcp_client
                    .post(&mcp_url)
                    .bearer_auth(ai_token)
                    .json(&serde_json::json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
                    .header("connection", "close")
                    .send()
                    .map_err(|error| format!("overlay MCP {method} request failed: {error:#?}"))?;
                if !response.status().is_success() {
                    return Err(format!("overlay MCP {method} returned HTTP {}", response.status()));
                }
                let body = response.text().map_err(display_err)?;
                match method {
                    "initialize" if !body.contains("2025-03-26") => return Err("overlay MCP initialize response did not advertise the protocol version".to_string()),
                    "tools/list" if !body.contains("read_shared_file") => return Err("overlay MCP tools/list did not expose read_shared_file".to_string()),
                    "tools/call" if !body.contains("downloaded through two EasyTier nodes") => return Err("overlay MCP read_shared_file returned unexpected content".to_string()),
                    _ => {}
                }
            }
            Ok(())
        })
        .await
        .map_err(display_err)
        .and_then(|result| result);
        api_server.unblock();
        let _ = request_thread.join();
        manager
            .delete_network_instance(vec![server_id, client_id])
            .unwrap();
        transfer_result.unwrap();
    }
}

pub(super) fn handle_mcp_request_with_auth(
    app: Option<&AppHandle>,
    mut request: Request,
    request_group: &str,
    group: &MeshShareGroup,
    authorized: bool,
) -> Result<(), String> {
    let started = Instant::now();
    let log_group = safe_log_label(request_group);
    if !authorized {
        if let Some(app) = app {
            append_app_log(
                app,
                "warn",
                &format!("mesh_mcp_denied group_id={log_group} reason=invalid_or_revoked_token elapsed_ms={}", started.elapsed().as_millis()),
            );
        }
        return respond_sync_json(
            request,
            401,
            serde_json::json!({"error":"AI file access is not authorized"}),
        );
    }
    let mut body = Vec::new();
    request
        .as_reader()
        .take(64 * 1024 + 1)
        .read_to_end(&mut body)
        .map_err(|error| {
            let error = display_err(error);
            if let Some(app) = app {
                append_app_log(
                    app,
                    "warn",
                    &format!("mesh_mcp_request_failed group_id={log_group} stage=read_body elapsed_ms={} error={error}", started.elapsed().as_millis()),
                );
            }
            error
        })?;
    if body.len() > 64 * 1024 {
        if let Some(app) = app {
            append_app_log(
                app,
                "warn",
                &format!(
                    "mesh_mcp_denied group_id={log_group} reason=request_too_large bytes={} elapsed_ms={}",
                    body.len(),
                    started.elapsed().as_millis()
                ),
            );
        }
        return respond_sync_json(
            request,
            413,
            serde_json::json!({"error":"MCP request exceeds 64 KiB"}),
        );
    }
    let rpc: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(error) => {
            if let Some(app) = app {
                append_app_log(
                    app,
                    "warn",
                    &format!(
                        "mesh_mcp_request_invalid group_id={log_group} reason=json_parse_error category={:?} line={} column={} elapsed_ms={}",
                        error.classify(),
                        error.line(),
                        error.column(),
                        started.elapsed().as_millis()
                    ),
                );
            }
            return respond_sync_json(
                request,
                400,
                serde_json::json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":error.to_string()}}),
            );
        }
    };
    let id = rpc.get("id").cloned().unwrap_or(Value::Null);
    let method = rpc
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut tool_failure = None;
    let result = match method {
        "initialize" => {
            serde_json::json!({"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"codex-account-switcher-files","version":"1.0.0"}})
        }
        "notifications/initialized" => {
            if let Some(app) = app {
                append_app_log(app, "info", &format!("mesh_mcp_request group_id={log_group} method=notifications/initialized result=accepted elapsed_ms={}", started.elapsed().as_millis()));
            }
            return request
                .respond(Response::empty(StatusCode(202)))
                .map_err(display_err);
        }
        "tools/list" => serde_json::json!({"tools":[
            {"name":"list_shared_items","description":"List file and folder shares in this group","inputSchema":{"type":"object","properties":{}}},
            {"name":"list_shared_files","description":"List files in a shared folder","inputSchema":{"type":"object","properties":{"shareId":{"type":"string"},"path":{"type":"string"}},"required":["shareId"]}},
            {"name":"search_shared_files","description":"Search file names recursively in a shared folder","inputSchema":{"type":"object","properties":{"shareId":{"type":"string"},"query":{"type":"string"}},"required":["shareId","query"]}},
            {"name":"read_shared_file","description":"Read a UTF-8 text file from a share (max 1 MiB)","inputSchema":{"type":"object","properties":{"shareId":{"type":"string"},"path":{"type":"string"}},"required":["shareId","path"]}}
        ]}),
        "tools/call" => {
            let params = rpc.get("params").cloned().unwrap_or_default();
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let args = params.get("arguments").cloned().unwrap_or_default();
            let group_mismatch = args
                .get("groupId")
                .and_then(Value::as_str)
                .is_some_and(|argument_group| argument_group != request_group);
            if group_mismatch {
                tool_failure = Some("group_mismatch");
                serde_json::json!({"isError":true,"content":[{"type":"text","text":"groupId must match the authorized MCP endpoint"}]})
            } else {
                let outcome = call_mcp_file_tool(group, name, &args);
                match outcome {
                    Ok(text) => serde_json::json!({"content":[{"type":"text","text":text}]}),
                    Err(error) => {
                        tool_failure = Some(mcp_tool_error_category(&error));
                        serde_json::json!({"isError":true,"content":[{"type":"text","text":error}]})
                    }
                }
            }
        }
        _ => {
            if let Some(app) = app {
                append_app_log(app, "warn", &format!("mesh_mcp_request group_id={log_group} method={} result=method_not_found elapsed_ms={}", safe_log_label(method), started.elapsed().as_millis()));
            }
            return respond_sync_json(
                request,
                200,
                serde_json::json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"method not found"}}),
            );
        }
    };
    let outcome = if result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "tool_error"
    } else {
        "ok"
    };
    let tool_name = rpc
        .get("params")
        .and_then(|params| params.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("-");
    if let Some(app) = app {
        append_app_log(app, if outcome == "ok" { "info" } else { "warn" }, &format!("mesh_mcp_request group_id={log_group} method={} tool={} result={outcome} failure_reason={} elapsed_ms={}", safe_log_label(method), safe_log_label(tool_name), tool_failure.unwrap_or("-"), started.elapsed().as_millis()));
    }
    let response = if id.is_null() {
        serde_json::json!({"jsonrpc":"2.0","result":result})
    } else {
        serde_json::json!({"jsonrpc":"2.0","id":id,"result":result})
    };
    respond_sync_json(request, 200, response)
}

fn mcp_tool_error_category(error: &str) -> &'static str {
    if error.contains("1 MiB") || error.contains("entry limit") {
        "size_limit"
    } else if error.contains("invalid relative") || error.contains("escapes") {
        "path_rejected"
    } else if error.contains("not found") || error.contains("unavailable") {
        "share_or_path_unavailable"
    } else if error.contains("not a file") || error.contains("file path is required") {
        "not_a_file"
    } else if error.contains("unknown tool") {
        "unknown_tool"
    } else {
        "tool_execution_failed"
    }
}
