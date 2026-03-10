use super::*;

pub(super) fn run_extension_fetch(matches: &ArgMatches, sub: &ArgMatches) -> Result<(), String> {
    let source = resolve_extension_release_source(
        sub.get_one::<String>("version").map(|v| v.as_str()),
        sub.get_one::<String>("repo").map(|v| v.as_str()),
        sub.get_one::<String>("asset").map(|v| v.as_str()),
        sub.get_one::<String>("out").map(PathBuf::from),
    )?;
    let payload = download_extension_asset(&source)?;
    if matches.get_flag("json") {
        if !matches.get_flag("no-pretty") {
            println!(
                "{}",
                serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?
            );
        } else {
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|e| e.to_string())?
            );
        }
    } else if let Some(path) = payload.get("path").and_then(Value::as_str) {
        println!("{path}");
    }
    Ok(())
}

pub(super) fn resolve_extension_release_source(
    version_input: Option<&str>,
    repo_input: Option<&str>,
    asset_input: Option<&str>,
    output_path_input: Option<PathBuf>,
) -> Result<ExtensionReleaseSource, String> {
    let version_input = version_input.unwrap_or(env!("CARGO_PKG_VERSION"));
    let tag = if version_input.starts_with('v') {
        version_input.to_string()
    } else {
        format!("v{version_input}")
    };
    let version = tag.trim_start_matches('v');
    let repo = repo_input.unwrap_or("ekroon/tabctl");
    let asset = asset_input.unwrap_or("tabctl-extension.zip");
    let output_path = if let Some(path) = output_path_input {
        path
    } else {
        PathBuf::from(resolve_data_dir(None)?)
            .join(EXTENSION_RELEASES_DIR_NAME)
            .join(version)
            .join(asset)
    };
    let url = format!("https://github.com/{repo}/releases/download/{tag}/{asset}");
    Ok(ExtensionReleaseSource {
        repo: repo.to_string(),
        tag,
        asset: asset.to_string(),
        path: output_path,
        url,
    })
}

pub(super) fn download_extension_asset(source: &ExtensionReleaseSource) -> Result<Value, String> {
    if let Some(parent) = source.path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create output directory: {e}"))?;
    }
    let status = ProcessCommand::new("curl")
        .arg("--fail")
        .arg("--location")
        .arg("--silent")
        .arg("--output")
        .arg(&source.path)
        .arg(&source.url)
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("Failed to execute curl: {e}"))?;
    if !status.success() {
        return Err(format!(
            "Failed to download extension asset from {}",
            source.url
        ));
    }
    Ok(source.to_json())
}

pub(super) fn extension_active_dir(data_dir: &str) -> PathBuf {
    PathBuf::from(data_dir).join(EXTENSION_ACTIVE_DIR_NAME)
}

pub(super) fn extension_versions_dir(data_dir: &str) -> PathBuf {
    PathBuf::from(data_dir).join(EXTENSION_VERSIONS_DIR_NAME)
}

pub(super) fn extension_version_dir(data_dir: &str, version: &str) -> PathBuf {
    extension_versions_dir(data_dir).join(version)
}

pub(super) fn extension_release_checksum_path(archive_path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.sha256", archive_path.display()))
}

pub(super) fn normalize_base_version(version: &str) -> &str {
    version
        .split_once('-')
        .map(|(base, _)| base)
        .or_else(|| version.split_once('+').map(|(base, _)| base))
        .unwrap_or(version)
}

/// Strips `-dev.{sha}[.dirty]` build suffix but preserves pre-release tags.
/// e.g. "0.6.0-alpha.10-dev.f4ad4314" → "0.6.0-alpha.10"
///      "0.6.0-alpha.10-dev.f4ad4314.dirty" → "0.6.0-alpha.10"
///      "0.6.0-alpha.10" → "0.6.0-alpha.10"
///      "0.6.0" → "0.6.0"
pub(super) fn strip_dev_suffix(version: &str) -> &str {
    if let Some(idx) = version.find("-dev.") {
        &version[..idx]
    } else {
        version
    }
}

pub(super) fn parse_base_triplet(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = normalize_base_version(version)
        .split('.')
        .map(|segment| segment.parse::<u64>().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

pub(super) fn compare_base_versions(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    let a_parts = parse_base_triplet(a)?;
    let b_parts = parse_base_triplet(b)?;
    let triplet_ord = a_parts.cmp(&b_parts);
    if triplet_ord != std::cmp::Ordering::Equal {
        return Some(triplet_ord);
    }
    // Triplets equal — compare pre-release segments (semver: no pre-release > has pre-release)
    let a_pre = extract_prerelease(a);
    let b_pre = extract_prerelease(b);
    Some(compare_prerelease(a_pre, b_pre))
}

fn extract_prerelease(version: &str) -> Option<&str> {
    let after_triplet = version.split_once('-').map(|(_, rest)| rest)?;
    // strip build metadata (+...)
    Some(
        after_triplet
            .split_once('+')
            .map_or(after_triplet, |(pre, _)| pre),
    )
}

fn compare_prerelease(a: Option<&str>, b: Option<&str>) -> std::cmp::Ordering {
    match (a, b) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Greater, // no pre-release > has pre-release
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(a), Some(b)) => {
            let a_parts = a.split('.');
            let b_parts = b.split('.');
            for (ap, bp) in a_parts.zip(b_parts) {
                let ord = match (ap.parse::<u64>(), bp.parse::<u64>()) {
                    (Ok(an), Ok(bn)) => an.cmp(&bn),
                    _ => ap.cmp(bp),
                };
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
            }
            a.len().cmp(&b.len())
        }
    }
}

pub(super) fn chromium_extension_id_from_digest(digest: &[u8]) -> String {
    digest
        .iter()
        .take(16)
        .flat_map(|byte| [((byte >> 4) & 0x0f) + b'a', (byte & 0x0f) + b'a'])
        .map(char::from)
        .collect::<String>()
}

pub(super) fn derive_extension_id_from_extension_path(path: &Path) -> Result<String, String> {
    let canonical = fs::canonicalize(path).map_err(|e| {
        format!(
            "Failed to resolve extension path {} for ID derivation: {e}",
            path.display()
        )
    })?;
    let normalized = canonical.to_string_lossy().replace('\\', "/");
    #[cfg(windows)]
    let normalized = {
        let mut normalized = normalized;
        if normalized.len() > 1 && normalized.as_bytes()[1] == b':' {
            if let Some(first) = normalized.chars().next() {
                normalized.replace_range(0..1, &first.to_ascii_lowercase().to_string());
            }
        }
        normalized
    };
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let digest = hasher.finalize();
    Ok(chromium_extension_id_from_digest(digest.as_ref()))
}

pub(super) fn read_extension_manifest_version(extension_dir: &Path) -> Option<String> {
    let manifest_path = extension_dir.join("manifest.json");
    let raw = fs::read_to_string(manifest_path).ok()?;
    let parsed: Value = serde_json::from_str(&raw).ok()?;
    parsed
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub(super) fn read_installed_extension_version(data_dir: &str) -> Option<String> {
    let active_dir = extension_active_dir(data_dir);
    let marker_path = active_dir.join(EXTENSION_VERSION_MARKER_FILE);
    if let Ok(content) = fs::read_to_string(marker_path) {
        let version = content.trim();
        if !version.is_empty() {
            return Some(version.to_string());
        }
    }
    read_extension_manifest_version(&active_dir)
}

pub(super) fn download_extension_checksum(
    source: &ExtensionReleaseSource,
) -> Result<PathBuf, String> {
    let checksum_path = extension_release_checksum_path(&source.path);
    if let Some(parent) = checksum_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create checksum output directory: {e}"))?;
    }
    let checksum_url = format!("{}.sha256", source.url);
    let status = ProcessCommand::new("curl")
        .arg("--fail")
        .arg("--location")
        .arg("--silent")
        .arg("--output")
        .arg(&checksum_path)
        .arg(&checksum_url)
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("Failed to execute curl: {e}"))?;
    if !status.success() {
        return Err(format!(
            "Failed to download extension checksum from {}",
            checksum_url
        ));
    }
    Ok(checksum_path)
}

pub(super) fn read_expected_sha256(checksum_path: &Path) -> Result<String, String> {
    let content = fs::read_to_string(checksum_path).map_err(|e| {
        format!(
            "Failed to read checksum file {}: {e}",
            checksum_path.display()
        )
    })?;
    let expected = content
        .split_whitespace()
        .next()
        .ok_or_else(|| format!("Checksum file {} is empty", checksum_path.display()))?
        .trim()
        .to_lowercase();
    if expected.len() != 64 || !expected.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "Checksum file {} does not contain a valid SHA256 digest",
            checksum_path.display()
        ));
    }
    Ok(expected)
}

pub(super) fn file_sha256(path: &Path) -> Result<String, String> {
    let mut file =
        fs::File::open(path).map_err(|e| format!("Failed to open {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    Ok(digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>())
}

pub(super) fn verify_extension_asset_checksum(
    source: &ExtensionReleaseSource,
    checksum_path: &Path,
) -> Result<(), String> {
    let expected = read_expected_sha256(checksum_path)?;
    let actual = file_sha256(&source.path)?;
    if expected != actual {
        return Err(format!(
            "Extension checksum mismatch for {} (expected {}, got {})",
            source.path.display(),
            expected,
            actual
        ));
    }
    Ok(())
}

pub(super) fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|e| format!("Failed to create {}: {e}", destination.display()))?;
    let entries = fs::read_dir(source)
        .map_err(|e| format!("Failed to read directory {}: {e}", source.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read directory entry: {e}"))?;
        let file_type = entry
            .file_type()
            .map_err(|e| format!("Failed to read file type: {e}"))?;
        let src_path = entry.path();
        let dest_path = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dest_path)?;
        } else if file_type.is_file() || file_type.is_symlink() {
            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
            }
            fs::copy(&src_path, &dest_path).map_err(|e| {
                format!(
                    "Failed to copy {} -> {}: {e}",
                    src_path.display(),
                    dest_path.display()
                )
            })?;
        }
    }
    Ok(())
}

pub(super) fn extract_extension_archive_to_version(
    source: &ExtensionReleaseSource,
    version_dir: &Path,
) -> Result<(), String> {
    let parent = version_dir
        .parent()
        .ok_or_else(|| "Invalid extension version directory path".to_string())?;
    fs::create_dir_all(parent).map_err(|e| {
        format!(
            "Failed to create extension versions directory {}: {e}",
            parent.display()
        )
    })?;
    let staging = parent.join(format!(
        ".staging-{}-{}",
        version_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("extension"),
        now_ms()
    ));
    if staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    fs::create_dir_all(&staging).map_err(|e| {
        format!(
            "Failed to create staging directory {}: {e}",
            staging.display()
        )
    })?;

    let file = fs::File::open(&source.path).map_err(|e| {
        format!(
            "Failed to open extension archive {}: {e}",
            source.path.display()
        )
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        format!(
            "Failed to read extension archive {}: {e}",
            source.path.display()
        )
    })?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read zip entry at index {i}: {e}"))?;
        let entry_path = entry
            .enclosed_name()
            .ok_or_else(|| format!("Unsafe zip entry path: {}", entry.name()))?
            .to_path_buf();
        let out_path = staging.join(entry_path);
        if entry.is_dir() {
            fs::create_dir_all(&out_path)
                .map_err(|e| format!("Failed to create {}: {e}", out_path.display()))?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
            }
            let mut out_file = fs::File::create(&out_path)
                .map_err(|e| format!("Failed to create {}: {e}", out_path.display()))?;
            std::io::copy(&mut entry, &mut out_file)
                .map_err(|e| format!("Failed to extract {}: {e}", out_path.display()))?;
        }
    }

    let rooted_manifest = staging.join("extension").join("manifest.json");
    let extracted_root = if rooted_manifest.exists() {
        staging.join("extension")
    } else if staging.join("manifest.json").exists() {
        staging.clone()
    } else {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!(
            "Extension archive {} does not contain manifest.json",
            source.path.display()
        ));
    };

    if version_dir.exists() {
        fs::remove_dir_all(version_dir).map_err(|e| {
            format!(
                "Failed to remove previous version directory {}: {e}",
                version_dir.display()
            )
        })?;
    }

    if extracted_root == staging {
        fs::rename(&staging, version_dir).map_err(|e| {
            format!(
                "Failed to move {} to {}: {e}",
                staging.display(),
                version_dir.display()
            )
        })?;
    } else {
        fs::rename(&extracted_root, version_dir).map_err(|e| {
            format!(
                "Failed to move {} to {}: {e}",
                extracted_root.display(),
                version_dir.display()
            )
        })?;
        let _ = fs::remove_dir_all(&staging);
    }

    if !version_dir.join("manifest.json").exists() {
        return Err(format!(
            "Extracted extension at {} is missing manifest.json",
            version_dir.display()
        ));
    }

    Ok(())
}

pub(super) fn activate_extension_version(
    data_dir: &str,
    version_dir: &Path,
    target_version: &str,
) -> Result<String, String> {
    let active_dir = extension_active_dir(data_dir);
    let parent = active_dir
        .parent()
        .ok_or_else(|| "Invalid extension active path".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
    let staging = parent.join(format!(".extension-active-{}", now_ms()));
    if staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    copy_dir_recursive(version_dir, &staging)?;
    fs::write(
        staging.join(EXTENSION_VERSION_MARKER_FILE),
        format!("{target_version}\n"),
    )
    .map_err(|e| format!("Failed to write extension version marker: {e}"))?;
    if active_dir.exists() {
        fs::remove_dir_all(&active_dir)
            .map_err(|e| format!("Failed to replace {}: {e}", active_dir.display()))?;
    }
    fs::rename(&staging, &active_dir).map_err(|e| {
        format!(
            "Failed to activate extension {} -> {}: {e}",
            staging.display(),
            active_dir.display()
        )
    })?;
    Ok(active_dir.display().to_string())
}

pub(super) fn sync_extension_release(
    source: &ExtensionReleaseSource,
    allow_download: bool,
) -> Result<ExtensionSyncResult, String> {
    let data_dir = resolve_data_dir(None)?;
    let target_version = source.tag.trim_start_matches('v').to_string();
    let installed_version = read_installed_extension_version(&data_dir);
    let version_dir = extension_version_dir(&data_dir, &target_version);

    let active_path = extension_active_dir(&data_dir);
    if installed_version.as_deref() == Some(target_version.as_str()) && active_path.exists() {
        return Ok(ExtensionSyncResult {
            updated: false,
            target_version,
            installed_version,
            active_path: active_path.display().to_string(),
        });
    }

    if !source.path.exists() {
        if !allow_download {
            return Err(format!(
                "Extension release asset not found at {}",
                source.path.display()
            ));
        }
        download_extension_asset(source)?;
    }

    let checksum_path = extension_release_checksum_path(&source.path);
    if !checksum_path.exists() {
        if !allow_download {
            return Err(format!(
                "Extension checksum file not found at {}",
                checksum_path.display()
            ));
        }
        download_extension_checksum(source)?;
    }
    verify_extension_asset_checksum(source, &checksum_path)?;

    if !version_dir.join("manifest.json").exists() {
        extract_extension_archive_to_version(source, &version_dir)?;
    }
    let active_path = activate_extension_version(&data_dir, &version_dir, &target_version)?;
    Ok(ExtensionSyncResult {
        updated: installed_version.as_deref() != Some(target_version.as_str()),
        target_version,
        installed_version,
        active_path,
    })
}

pub(super) fn sync_extension_unpacked_dir(
    source_dir: &Path,
) -> Result<ExtensionSyncResult, String> {
    let canonical_source = fs::canonicalize(source_dir).map_err(|e| {
        format!(
            "Failed to resolve local extension directory {}: {e}",
            source_dir.display()
        )
    })?;
    if !canonical_source.is_dir() {
        return Err(format!(
            "Local extension source {} is not a directory",
            canonical_source.display()
        ));
    }
    if !canonical_source.join("manifest.json").exists() {
        return Err(format!(
            "Local extension source {} is missing manifest.json",
            canonical_source.display()
        ));
    }

    let data_dir = resolve_data_dir(None)?;
    let target_version = env!("CARGO_PKG_VERSION").to_string();
    let installed_version = read_installed_extension_version(&data_dir);
    let version_dir = extension_version_dir(&data_dir, &target_version);
    if version_dir.exists() {
        fs::remove_dir_all(&version_dir).map_err(|e| {
            format!(
                "Failed to remove previous version directory {}: {e}",
                version_dir.display()
            )
        })?;
    }
    copy_dir_recursive(&canonical_source, &version_dir)?;
    if !version_dir.join("manifest.json").exists() {
        return Err(format!(
            "Copied extension at {} is missing manifest.json",
            version_dir.display()
        ));
    }
    let active_path = activate_extension_version(&data_dir, &version_dir, &target_version)?;
    Ok(ExtensionSyncResult {
        updated: true,
        target_version,
        installed_version,
        active_path,
    })
}

pub(super) fn should_runtime_auto_sync(action: &str) -> bool {
    !matches!(action, "reload")
}

pub(super) fn effective_auto_sync_mode() -> AutoSyncMode {
    let Some(raw) = std::env::var("TABCTL_AUTO_SYNC_MODE").ok() else {
        return AutoSyncMode::Auto;
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "off" | "0" | "false" | "disabled" => AutoSyncMode::Off,
        "release-like" | "release_like" | "release" | "force" | "on" | "1" | "true" => {
            AutoSyncMode::ReleaseLike
        }
        _ => AutoSyncMode::Auto,
    }
}

pub(super) fn maybe_runtime_extension_auto_sync(
    action: &str,
    profile: Option<&str>,
    prefetched_ping: Option<&ResponseEnvelope>,
) {
    if !should_runtime_auto_sync(action) {
        return;
    }
    let mode = effective_auto_sync_mode();
    if mode == AutoSyncMode::Off {
        return;
    }
    if mode == AutoSyncMode::Auto {
        if std::env::var("TABCTL_VERSION_MODE").ok().as_deref() == Some("dev") {
            return;
        }
        if std::env::var("TABCTL_SETUP_FETCH_EXTENSION")
            .ok()
            .as_deref()
            == Some("0")
        {
            return;
        }
    }

    // Check if the host wrapper needs repair (binary path changed after CLI upgrade)
    let wrapper_repaired = maybe_auto_repair_wrapper(profile);

    let owned_ping;
    let ping = if let Some(pre) = prefetched_ping {
        pre
    } else {
        owned_ping = match send_request("ping", Value::Object(Map::new()), profile, false) {
            Ok(response) if response.ok => response,
            _ => return,
        };
        &owned_ping
    };

    let Some(data) = ping.data.as_ref().and_then(Value::as_object) else {
        return;
    };

    let host_base = data.get("hostBaseVersion").and_then(Value::as_str);
    let extension_version = data
        .get("version")
        .and_then(Value::as_str)
        .map(strip_dev_suffix);
    let (Some(host_base), Some(ext_release)) = (host_base, extension_version) else {
        return;
    };

    if ext_release == host_base && !wrapper_repaired {
        return;
    }

    let cli_base = env!("CARGO_PKG_VERSION");
    let sync_target = if wrapper_repaired {
        cli_base
    } else {
        host_base
    };

    if compare_base_versions(ext_release, sync_target) == Some(std::cmp::Ordering::Greater) {
        eprintln!(
            "[tabctl] extension auto-sync skipped (installed extension {} is newer than tabctl {})",
            ext_release, sync_target
        );
        return;
    }

    let source = match resolve_extension_release_source(Some(sync_target), None, None, None) {
        Ok(source) => source,
        Err(error) => {
            eprintln!(
                "\u{26a0}\u{fe0f} auto-sync failed: {error}. Run 'tabctl upgrade' to sync manually."
            );
            return;
        }
    };

    eprint!("\u{2699}\u{fe0f} auto-syncing extension to {sync_target}...");
    let sync = match sync_extension_release(&source, true) {
        Ok(sync) => sync,
        Err(error) => {
            eprintln!(
                " failed\n\u{26a0}\u{fe0f} auto-sync failed: {error}. Run 'tabctl upgrade' to sync manually."
            );
            return;
        }
    };

    if !sync.updated && !wrapper_repaired {
        eprintln!(" already up to date");
        return;
    }

    let _ = send_request("reload", Value::Object(Map::new()), profile, false);
    eprintln!(" done");
}

/// Check if the host wrapper's binary path is stale and repair it.
/// Returns `true` if the wrapper was updated.
fn maybe_auto_repair_wrapper(profile: Option<&str>) -> bool {
    let profile_name = match resolve_effective_profile(profile) {
        Some(name) => name,
        None => return false,
    };
    let config_dir = match resolve_config_dir() {
        Ok(dir) => dir,
        Err(_) => return false,
    };
    let profiles_path = PathBuf::from(&config_dir).join("profiles.json");
    let content = match fs::read_to_string(&profiles_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let registry = match serde_json::from_str::<ProfileRegistry>(&content) {
        Ok(r) => r,
        Err(_) => return false,
    };
    let Some(entry) = registry.profiles.get(&profile_name) else {
        return false;
    };

    let current_binary = resolve_tabctl_binary_path();
    if entry.node_path == current_binary {
        return false;
    }

    eprintln!("\u{2699}\u{fe0f} upgrading host wrapper for profile \"{profile_name}\"...");
    match attempt_profile_repair(&profile_name, entry) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("\u{26a0}\u{fe0f} wrapper repair failed: {e}");
            false
        }
    }
}

pub(super) fn run_setup(matches: &ArgMatches, sub: &ArgMatches) -> Result<(), String> {
    let browser = sub
        .get_one::<String>("browser")
        .map(|v| v.as_str())
        .ok_or_else(|| "Missing --browser".to_string())?;
    if browser != "edge" && browser != "chrome" {
        return Err("Missing or invalid --browser (edge|chrome)".to_string());
    }
    let local_extension_dir = resolve_setup_extension_dir_override(sub)?;
    let release_source = if local_extension_dir.is_none() {
        Some(resolve_setup_release_source(sub)?)
    } else {
        None
    };
    let mut setup_warnings = Vec::new();
    let skip_extension_download = should_skip_extension_download(sub);
    let extension_asset = if let Some(local_dir) = local_extension_dir.as_ref() {
        json!({
            "downloaded": false,
            "reason": "local-source",
            "source": {
                "type": "local-directory",
                "path": local_dir.display().to_string()
            }
        })
    } else {
        let release_source = release_source
            .as_ref()
            .ok_or_else(|| "Missing release source".to_string())?;
        if skip_extension_download {
            json!({
                "downloaded": false,
                "reason": "skipped",
                "source": release_source.to_json()
            })
        } else {
            match download_extension_asset(release_source) {
                Ok(payload) => json!({ "downloaded": true, "asset": payload }),
                Err(error_message) => {
                    let warning = json!({
                        "code": "extension_download_failed",
                        "message": error_message,
                        "url": release_source.url.clone(),
                    });
                    setup_warnings.push(warning.clone());
                    json!({
                        "downloaded": false,
                        "reason": "download-failed",
                        "source": release_source.to_json(),
                        "fallback": {
                            "path": release_source.path.display().to_string()
                        },
                        "warning": warning
                    })
                }
            }
        }
    };
    let data_dir = resolve_data_dir(None)?;
    let runtime_env = if cfg!(windows) {
        "native-win32"
    } else if cfg!(target_os = "macos") {
        "native-darwin"
    } else {
        "native-linux"
    };
    let wrapper_path = resolve_tabctl_binary_path();
    let explicit_extension_id = sub.get_one::<String>("extension-id").cloned();
    let mut sync_active_path = None::<String>;
    let extension_sync = if let Some(local_dir) = local_extension_dir.as_ref() {
        match sync_extension_unpacked_dir(local_dir) {
            Ok(result) => {
                sync_active_path = Some(result.active_path.clone());
                json!({
                    "attempted": true,
                    "ok": true,
                    "updated": result.updated,
                    "targetVersion": result.target_version,
                    "installedVersion": result.installed_version,
                    "activePath": result.active_path
                })
            }
            Err(error_message) => {
                let warning = json!({
                    "code": "extension_sync_failed",
                    "message": error_message
                });
                setup_warnings.push(warning.clone());
                json!({
                    "attempted": true,
                    "ok": false,
                    "error": error_message,
                    "warning": warning
                })
            }
        }
    } else {
        let release_source = release_source
            .as_ref()
            .ok_or_else(|| "Missing release source".to_string())?;
        match sync_extension_release(release_source, !skip_extension_download) {
            Ok(result) => {
                sync_active_path = Some(result.active_path.clone());
                json!({
                    "attempted": true,
                    "ok": true,
                    "updated": result.updated,
                    "targetVersion": result.target_version,
                    "installedVersion": result.installed_version,
                    "activePath": result.active_path
                })
            }
            Err(error_message) => {
                let warning = json!({
                    "code": "extension_sync_failed",
                    "message": error_message
                });
                setup_warnings.push(warning.clone());
                json!({
                    "attempted": true,
                    "ok": false,
                    "error": error_message,
                    "warning": warning
                })
            }
        }
    };

    let mut effective_extension_id = explicit_extension_id.clone();
    let extension_id_source = if effective_extension_id.is_some() {
        "explicit"
    } else {
        let derive_path = sync_active_path
            .map(PathBuf::from)
            .unwrap_or_else(|| extension_active_dir(&data_dir));
        match derive_extension_id_from_extension_path(&derive_path) {
            Ok(derived_id) => {
                effective_extension_id = Some(derived_id);
                "derived"
            }
            Err(error_message) => {
                setup_warnings.push(json!({
                    "code": "extension_id_derive_failed",
                    "message": error_message
                }));
                "missing"
            }
        }
    };

    let mut actual_wrapper_path = wrapper_path.clone();
    let mut actual_manifest_path = data_dir.clone();
    let mut is_default_profile = false;
    let mut profile_registry = None::<Value>;

    #[cfg(windows)]
    let mut registry_key_value = None::<String>;

    if let Some(ref ext_id) = effective_extension_id {
        let profile_name = sub
            .get_one::<String>("name")
            .map(|s| s.as_str())
            .unwrap_or(browser);

        let profile_data_dir = PathBuf::from(&data_dir).join("profiles").join(profile_name);

        let wrapper_file = write_host_wrapper(&wrapper_path, profile_name, &profile_data_dir)?;

        let user_data_dir = sub.get_one::<String>("user-data-dir").map(|s| s.as_str());
        let manifest_path = write_native_manifest(browser, &wrapper_file, ext_id, user_data_dir)?;

        #[cfg(windows)]
        {
            registry_key_value = Some(write_registry_key(browser, &manifest_path)?);
        }

        actual_wrapper_path = wrapper_file.display().to_string();
        actual_manifest_path = manifest_path.display().to_string();

        let registry = register_profile(
            &data_dir,
            profile_name,
            browser,
            ext_id,
            &wrapper_file,
            &manifest_path,
        )?;
        is_default_profile = registry
            .get("default")
            .and_then(|v| v.as_str())
            .map(|d| d == profile_name)
            .unwrap_or(false);
        profile_registry = Some(registry);
    }

    let mut data = json!({
        "profileName": browser,
        "browser": browser,
        "runtimeEnv": runtime_env,
        "dataDir": data_dir,
        "wrapperPath": actual_wrapper_path,
        "manifestPath": actual_manifest_path,
        "hostArgs": ["host"],
        "extensionReleaseAsset": extension_asset,
        "extensionSync": extension_sync,
        "extensionIdSource": extension_id_source,
        "warnings": setup_warnings
    });
    if let Some(id) = effective_extension_id.as_deref() {
        data["extensionId"] = json!(id);
        data["allowedOrigins"] = json!([format!("chrome-extension://{id}/")]);
        data["isDefault"] = json!(is_default_profile);
        if let Some(ref reg) = profile_registry {
            data["profileRegistry"] = reg.clone();
        }
        #[cfg(windows)]
        if let Some(ref rk) = registry_key_value {
            data["registryKey"] = json!(rk);
        }
    }
    let setup_payload = json!({
        "ok": true,
        "action": "setup",
        "data": data
    });
    if matches.get_flag("json") {
        if !matches.get_flag("no-pretty") {
            println!(
                "{}",
                serde_json::to_string_pretty(&setup_payload).map_err(|e| e.to_string())?
            );
        } else {
            println!(
                "{}",
                serde_json::to_string(&setup_payload).map_err(|e| e.to_string())?
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&setup_payload["data"]).map_err(|e| e.to_string())?
        );
    }
    Ok(())
}

/// Resolve the path to the `tabctl` binary.
/// Prefers the current running executable; falls back to searching PATH.
pub(super) fn resolve_tabctl_binary_path() -> String {
    if let Ok(exe) = std::env::current_exe() {
        let path_str = if let Ok(canonical) = dunce::canonicalize(&exe) {
            canonical.display().to_string()
        } else {
            exe.display().to_string()
        };
        return path_str;
    }
    // Fallback: look up "tabctl" in PATH
    let cmd = if cfg!(windows) { "where" } else { "which" };
    if let Ok(output) = ProcessCommand::new(cmd).arg("tabctl").output() {
        if output.status.success() {
            if let Ok(path) = String::from_utf8(output.stdout) {
                let trimmed = path.trim().to_string();
                if !trimmed.is_empty() {
                    return trimmed;
                }
            }
        }
    }
    "tabctl".to_string()
}

#[derive(Clone, Debug)]
pub(super) struct ExtensionReleaseSource {
    pub(super) repo: String,
    pub(super) tag: String,
    pub(super) asset: String,
    pub(super) path: PathBuf,
    pub(super) url: String,
}

impl ExtensionReleaseSource {
    fn to_json(&self) -> Value {
        json!({
            "repo": self.repo,
            "tag": self.tag,
            "asset": self.asset,
            "url": self.url,
            "path": self.path.display().to_string(),
            "version": self.tag.trim_start_matches('v'),
        })
    }
}

pub(super) fn resolve_setup_release_source(
    sub: &ArgMatches,
) -> Result<ExtensionReleaseSource, String> {
    let tag_override = sub
        .get_one::<String>("release-tag")
        .cloned()
        .or_else(|| sub.get_one::<String>("release-version").cloned())
        .or_else(|| std::env::var("TABCTL_RELEASE_TAG").ok());
    resolve_extension_release_source(
        tag_override.as_deref(),
        resolve_setup_release_override(sub, "release-repo", "TABCTL_RELEASE_REPO").as_deref(),
        resolve_setup_release_override(sub, "release-asset", "TABCTL_RELEASE_ASSET").as_deref(),
        None,
    )
}

pub(super) fn resolve_setup_extension_dir_override(
    sub: &ArgMatches,
) -> Result<Option<PathBuf>, String> {
    let local_dir = sub
        .get_one::<String>("extension-dir")
        .cloned()
        .or_else(|| std::env::var("TABCTL_SETUP_EXTENSION_DIR").ok());
    let Some(local_dir) = local_dir else {
        return Ok(None);
    };
    let resolved = fs::canonicalize(&local_dir).map_err(|e| {
        format!(
            "Failed to resolve local extension directory {}: {e}",
            local_dir
        )
    })?;
    if !resolved.is_dir() {
        return Err(format!(
            "Local extension source {} is not a directory",
            resolved.display()
        ));
    }
    if !resolved.join("manifest.json").exists() {
        return Err(format!(
            "Local extension source {} is missing manifest.json",
            resolved.display()
        ));
    }
    Ok(Some(resolved))
}

pub(super) fn resolve_setup_release_override(
    sub: &ArgMatches,
    cli_key: &str,
    env_key: &str,
) -> Option<String> {
    sub.get_one::<String>(cli_key)
        .cloned()
        .or_else(|| std::env::var(env_key).ok())
}

pub(super) fn should_skip_extension_download(sub: &ArgMatches) -> bool {
    sub.get_flag("skip-extension-download")
        || std::env::var("TABCTL_SETUP_FETCH_EXTENSION")
            .ok()
            .as_deref()
            == Some("0")
}

pub(super) fn register_profile(
    data_dir: &str,
    profile_name: &str,
    browser: &str,
    extension_id: &str,
    wrapper_path: &Path,
    _manifest_path: &Path,
) -> Result<Value, String> {
    let config_dir = resolve_config_dir()?;
    let profiles_path = PathBuf::from(&config_dir).join("profiles.json");

    let mut registry = if profiles_path.exists() {
        let contents = fs::read_to_string(&profiles_path)
            .map_err(|e| format!("failed to read profiles.json: {e}"))?;
        serde_json::from_str::<ProfileRegistry>(&contents)
            .map_err(|e| format!("failed to parse profiles.json: {e}"))?
    } else {
        ProfileRegistry {
            default: None,
            profiles: HashMap::new(),
        }
    };

    let browser_enum = match browser {
        "edge" => Browser::Edge,
        "chrome" => Browser::Chrome,
        _ => return Err(format!("unsupported browser: {browser}")),
    };

    let profile_data_dir = PathBuf::from(data_dir).join("profiles").join(profile_name);

    let entry = ProfileEntry {
        browser: browser_enum,
        extension_id: extension_id.to_string(),
        node_path: resolve_tabctl_binary_path(),
        host_path: path_to_platform_string(wrapper_path),
        data_dir: path_to_platform_string(&profile_data_dir),
        user_data_dir: None,
    };

    // First registered profile becomes the default
    if registry.default.is_none() || registry.profiles.is_empty() {
        registry.default = Some(profile_name.to_string());
    }

    registry.profiles.insert(profile_name.to_string(), entry);

    fs::create_dir_all(&config_dir).map_err(|e| format!("failed to create config dir: {e}"))?;

    let content = serde_json::to_string_pretty(&registry).map_err(|e| e.to_string())?;
    fs::write(&profiles_path, content)
        .map_err(|e| format!("failed to write profiles.json: {e}"))?;

    serde_json::to_value(&registry).map_err(|e| e.to_string())
}

pub(super) const HOST_NAME: &str = "com.erwinkroon.tabctl";

pub(super) fn resolve_manifest_dir(browser: &str) -> Result<PathBuf, String> {
    match browser {
        "edge" | "chrome" => {}
        _ => return Err(format!("unsupported browser: {browser}")),
    }

    #[cfg(target_os = "windows")]
    {
        let data_dir = resolve_data_dir(None)?;
        Ok(PathBuf::from(data_dir))
    }

    #[cfg(target_os = "macos")]
    {
        let home =
            dirs::home_dir().ok_or_else(|| "Unable to resolve home directory".to_string())?;
        let subdir = match browser {
            "edge" => "Microsoft Edge",
            _ => "Google/Chrome",
        };
        Ok(home
            .join("Library/Application Support")
            .join(subdir)
            .join("NativeMessagingHosts"))
    }

    #[cfg(target_os = "linux")]
    {
        let home =
            dirs::home_dir().ok_or_else(|| "Unable to resolve home directory".to_string())?;
        let subdir = match browser {
            "edge" => "microsoft-edge",
            _ => "google-chrome",
        };
        Ok(home
            .join(".config")
            .join(subdir)
            .join("NativeMessagingHosts"))
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    Err(format!("unsupported platform"))
}

pub(super) fn write_host_wrapper(
    tabctl_binary_path: &str,
    profile_name: &str,
    wrapper_dir: &Path,
) -> Result<PathBuf, String> {
    fs::create_dir_all(wrapper_dir).map_err(|e| format!("failed to create wrapper dir: {e}"))?;
    let config_dir = resolve_config_dir()?;
    let data_dir = resolve_data_dir(None)?;

    #[cfg(unix)]
    let (filename, content) = {
        let script = format!(
            "#!/usr/bin/env bash\nset -euo pipefail\nexport TABCTL_PROFILE=\"{profile_name}\"\nexport TABCTL_CONFIG_DIR=\"{config_dir}\"\nexport TABCTL_DATA_DIR=\"{data_dir}\"\nexec \"{tabctl_binary_path}\" host\n"
        );
        ("tabctl-host.sh", script)
    };

    #[cfg(windows)]
    let (filename, content) = {
        let script = format!(
            "@echo off\r\nset TABCTL_PROFILE={profile_name}\r\nset TABCTL_CONFIG_DIR={config_dir}\r\nset TABCTL_DATA_DIR={data_dir}\r\n\"{tabctl_binary_path}\" host\r\n"
        );
        ("tabctl-host.cmd", script)
    };

    let wrapper_path = wrapper_dir.join(filename);
    fs::write(&wrapper_path, &content)
        .map_err(|e| format!("failed to write wrapper script: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&wrapper_path, fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("failed to set wrapper permissions: {e}"))?;
    }

    Ok(wrapper_path)
}

#[cfg(windows)]
pub(super) fn write_registry_key(browser: &str, manifest_path: &Path) -> Result<String, String> {
    use winreg::enums::*;
    use winreg::RegKey;

    let subkey = match browser {
        "edge" => format!("Software\\Microsoft\\Edge\\NativeMessagingHosts\\{HOST_NAME}"),
        "chrome" => format!("Software\\Google\\Chrome\\NativeMessagingHosts\\{HOST_NAME}"),
        _ => return Err(format!("Unsupported browser for registry: {browser}")),
    };

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(&subkey)
        .map_err(|e| format!("Failed to create registry key: {e}"))?;

    key.set_value("", &path_to_platform_string(manifest_path))
        .map_err(|e| format!("Failed to set registry value: {e}"))?;

    Ok(format!("HKCU\\{subkey}"))
}

pub(super) fn write_native_manifest(
    browser: &str,
    wrapper_path: &Path,
    extension_id: &str,
    user_data_dir: Option<&str>,
) -> Result<PathBuf, String> {
    let manifest_dir = if let Some(udd) = user_data_dir {
        PathBuf::from(udd).join("NativeMessagingHosts")
    } else {
        resolve_manifest_dir(browser)?
    };
    fs::create_dir_all(&manifest_dir).map_err(|e| format!("failed to create manifest dir: {e}"))?;

    let abs_wrapper =
        dunce::canonicalize(wrapper_path).unwrap_or_else(|_| wrapper_path.to_path_buf());

    let manifest = json!({
        "name": HOST_NAME,
        "description": "tabctl native host",
        "path": path_to_platform_string(&abs_wrapper),
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{extension_id}/")]
    });

    let manifest_path = manifest_dir.join(format!("{HOST_NAME}.json"));
    let content = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    fs::write(&manifest_path, content)
        .map_err(|e| format!("failed to write native manifest: {e}"))?;

    Ok(manifest_path)
}
