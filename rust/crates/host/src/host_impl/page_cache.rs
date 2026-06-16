//! Per-tab HTML snapshot cache.
//!
//! The cache is strictly profile-isolated and stores successful, non-incognito,
//! non-truncated HTML snapshots as one private file per `(profile, tab_id, url)`
//! entry under the active profile data directory. Full URLs, including fragments,
//! are the exact cache key; each entry also records a canonical URL with only the
//! fragment stripped. Lookups prefer the exact tab and URL, then the latest
//! duplicate/restored tab for the exact URL, then the latest canonical URL match.
//! Callers prune after each browser snapshot, and corrupted cache files are ignored.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

const MAX_HTML_CHARS_PER_ENTRY: usize = 10 * 1024 * 1024;
const MAX_ENTRIES: usize = 250;
const MAX_TOTAL_HTML_BYTES: usize = 512 * 1024 * 1024;
const MAX_OPEN_TAB_CLEANUP_FILE_READS: usize = 16;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CacheKey {
    profile: String,
    tab_id: i64,
    url_key: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct OpenTabCacheKey {
    pub(crate) tab_id: i64,
    pub(crate) url: String,
}

impl OpenTabCacheKey {
    pub(crate) fn new(tab_id: i64, url: impl Into<String>) -> Self {
        Self {
            tab_id,
            url: url.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PageCacheEntry {
    pub(crate) profile: String,
    pub(crate) tab_id: i64,
    pub(crate) url_key: String,
    #[serde(default)]
    pub(crate) canonical_url_key: String,
    pub(crate) title: Option<String>,
    pub(crate) html: String,
    pub(crate) source_html_chars: i64,
    pub(crate) source_text_chars: i64,
    pub(crate) document_ready_state: Option<String>,
    pub(crate) captured_at: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageCacheFileMetadata {
    profile: String,
    tab_id: i64,
    url_key: String,
    #[serde(default)]
    canonical_url_key: String,
}

#[derive(Debug)]
struct PageCacheFileSummary {
    path: PathBuf,
    key: CacheKey,
    captured_at: i64,
    html_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageCacheMatchMode {
    Exact,
    DuplicateExact,
    Canonical,
}

impl PageCacheMatchMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::DuplicateExact => "duplicateExact",
            Self::Canonical => "canonical",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PageCacheLookup {
    pub(crate) entry: PageCacheEntry,
    pub(crate) match_mode: PageCacheMatchMode,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct PageCache {
    entries: HashMap<CacheKey, PageCacheEntry>,
    dirty: bool,
    stale_files: HashSet<PathBuf>,
}

impl PageCache {
    pub(crate) fn load(path: &Path) -> Self {
        let Ok(entries) = fs::read_dir(path) else {
            return Self::default();
        };

        let mut cache = Self::default();
        for dir_entry in entries.filter_map(Result::ok) {
            let file_path = dir_entry.path();
            if file_path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Ok(bytes) = fs::read(&file_path) else {
                cache.stale_files.insert(file_path);
                cache.dirty = true;
                continue;
            };
            let Ok(mut entry) = serde_json::from_slice::<PageCacheEntry>(&bytes) else {
                cache.stale_files.insert(file_path);
                cache.dirty = true;
                continue;
            };
            if entry.html.is_empty() || entry.html.chars().count() > MAX_HTML_CHARS_PER_ENTRY {
                cache.stale_files.insert(file_path);
                cache.dirty = true;
                continue;
            };
            let canonical_url_key = canonical_url_key(&entry.url_key);
            if entry.canonical_url_key != canonical_url_key {
                entry.canonical_url_key = canonical_url_key;
                cache.dirty = true;
            }
            cache.entries.insert(
                CacheKey {
                    profile: entry.profile.clone(),
                    tab_id: entry.tab_id,
                    url_key: entry.url_key.clone(),
                },
                entry,
            );
        }
        cache.enforce_caps();
        cache
    }

    pub(crate) fn lookup_open_tab(
        &self,
        profile: Option<&str>,
        tab_id: i64,
        url: &str,
    ) -> Option<PageCacheLookup> {
        let profile = profile_key(profile);
        let url_key = url_key(url);
        if let Some(entry) = self.entries.get(&CacheKey {
            profile: profile.clone(),
            tab_id,
            url_key: url_key.clone(),
        }) {
            return Some(PageCacheLookup {
                entry: entry.clone(),
                match_mode: PageCacheMatchMode::Exact,
            });
        }

        if let Some(entry) = self
            .entries
            .values()
            .filter(|entry| entry.profile == profile && entry.url_key == url_key)
            .max_by_key(|entry| entry.captured_at)
        {
            return Some(PageCacheLookup {
                entry: entry.clone(),
                match_mode: PageCacheMatchMode::DuplicateExact,
            });
        }

        let canonical_url_key = canonical_url_key(url);
        self.entries
            .values()
            .filter(|entry| {
                entry.profile == profile && entry.canonical_url_key == canonical_url_key
            })
            .max_by_key(|entry| entry.captured_at)
            .map(|entry| PageCacheLookup {
                entry: entry.clone(),
                match_mode: PageCacheMatchMode::Canonical,
            })
    }

    pub(crate) fn exact_file_available(
        path: &Path,
        profile: Option<&str>,
        tab_id: i64,
        url: &str,
    ) -> Result<bool, String> {
        let profile = profile_key(profile);
        let url_key = url_key(url);
        let file_path = cache_file_path_for_key(path, &profile, tab_id, &url_key);
        let Some(entry) = read_exact_file_entry(&file_path, &profile, tab_id, &url_key)? else {
            return Ok(false);
        };
        Ok(is_usable_entry(&entry))
    }

    #[cfg(test)]
    fn lookup_exact_open_tab(
        &self,
        profile: Option<&str>,
        tab_id: i64,
        url: &str,
    ) -> Option<PageCacheEntry> {
        self.entries
            .get(&CacheKey {
                profile: profile_key(profile),
                tab_id,
                url_key: url_key(url),
            })
            .cloned()
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn store_success(
        &mut self,
        profile: Option<&str>,
        tab_id: i64,
        url: &str,
        title: Option<&str>,
        html: &str,
        source_html_chars: i64,
        source_text_chars: i64,
        document_ready_state: Option<&str>,
        truncated_html: bool,
        incognito: bool,
        captured_at: i64,
    ) {
        let Some(entry) = build_success_entry(
            profile,
            tab_id,
            url,
            title,
            html,
            source_html_chars,
            source_text_chars,
            document_ready_state,
            truncated_html,
            incognito,
            captured_at,
        ) else {
            return;
        };

        let key = CacheKey {
            profile: entry.profile.clone(),
            tab_id: entry.tab_id,
            url_key: entry.url_key.clone(),
        };
        self.entries.insert(key, entry);
        self.dirty = true;
        self.enforce_caps();
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn store_success_file(
        path: &Path,
        profile: Option<&str>,
        tab_id: i64,
        url: &str,
        title: Option<&str>,
        html: &str,
        source_html_chars: i64,
        source_text_chars: i64,
        document_ready_state: Option<&str>,
        truncated_html: bool,
        incognito: bool,
        captured_at: i64,
        open_tabs: Option<&[OpenTabCacheKey]>,
    ) -> Result<bool, String> {
        let Some(entry) = build_success_entry(
            profile,
            tab_id,
            url,
            title,
            html,
            source_html_chars,
            source_text_chars,
            document_ready_state,
            truncated_html,
            incognito,
            captured_at,
        ) else {
            return Ok(false);
        };

        fs::create_dir_all(path).map_err(|err| format!("create page cache directory: {err}"))?;
        set_private_dir_permissions(path)?;
        write_cache_entry(path, &entry)?;
        remove_replaced_files_for_current_tab(path, &entry)?;
        if let Some(open_tabs) = open_tabs {
            remove_stale_files_to_open_tabs(path, &entry.profile, open_tabs)?;
        }
        enforce_file_caps(path)?;
        Self::exact_file_available(path, Some(&entry.profile), entry.tab_id, &entry.url_key)
    }

    pub(crate) fn prune_to_open_tabs(
        &mut self,
        profile: Option<&str>,
        open_tabs: &[OpenTabCacheKey],
    ) {
        let profile = profile_key(profile);
        let open_exact_urls: HashSet<String> =
            open_tabs.iter().map(|tab| url_key(&tab.url)).collect();
        let open_canonical_urls: HashSet<String> = open_tabs
            .iter()
            .map(|tab| canonical_url_key(&tab.url))
            .collect();
        let before = self.entries.len();
        self.entries.retain(|key, entry| {
            key.profile != profile
                || open_exact_urls.contains(&entry.url_key)
                || open_canonical_urls.contains(&entry.canonical_url_key)
        });
        if self.entries.len() != before {
            self.dirty = true;
        }
    }

    pub(crate) fn save_if_dirty(&mut self, path: &Path) -> Result<(), String> {
        if !self.dirty {
            return Ok(());
        }
        fs::create_dir_all(path).map_err(|err| format!("create page cache directory: {err}"))?;
        set_private_dir_permissions(path)?;

        for stale_file in self.stale_files.drain() {
            let _ = fs::remove_file(stale_file);
        }

        let expected_files: HashSet<PathBuf> = self
            .entries
            .values()
            .map(|entry| cache_file_path(path, entry))
            .collect();
        if let Ok(existing) = fs::read_dir(path) {
            for file_path in existing
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
            {
                if !expected_files.contains(&file_path) {
                    let _ = fs::remove_file(file_path);
                }
            }
        }

        let mut entries: Vec<_> = self.entries.values().cloned().collect();
        entries.sort_by_key(|entry| (entry.captured_at, entry.profile.clone(), entry.tab_id));
        for entry in entries {
            write_cache_entry(path, &entry)?;
        }
        self.dirty = false;
        Ok(())
    }

    fn enforce_caps(&mut self) {
        self.enforce_caps_with_limits(MAX_ENTRIES, MAX_TOTAL_HTML_BYTES);
    }

    fn enforce_caps_with_limits(&mut self, max_entries: usize, max_total_html_bytes: usize) {
        let mut entries: Vec<_> = self.entries.values().cloned().collect();
        entries.sort_by_key(|entry| entry.captured_at);

        let mut keep = HashSet::new();
        let mut total_bytes = 0usize;
        for entry in entries.into_iter().rev() {
            let html_bytes = entry.html.len();
            if keep.len() >= max_entries
                || total_bytes.saturating_add(html_bytes) > max_total_html_bytes
            {
                self.dirty = true;
                continue;
            }

            total_bytes += html_bytes;
            keep.insert(CacheKey {
                profile: entry.profile,
                tab_id: entry.tab_id,
                url_key: entry.url_key,
            });
        }
        let before = self.entries.len();
        self.entries.retain(|key, _| keep.contains(key));
        if self.entries.len() != before {
            self.dirty = true;
        }
    }
}

fn cache_file_path(dir: &Path, entry: &PageCacheEntry) -> PathBuf {
    cache_file_path_for_key(dir, &entry.profile, entry.tab_id, &entry.url_key)
}

fn cache_file_path_for_key(dir: &Path, profile: &str, tab_id: i64, url_key: &str) -> PathBuf {
    dir.join(format!(
        "tab-{}-{}.json",
        tab_id,
        cache_file_hash(profile, tab_id, url_key)
    ))
}

#[allow(clippy::too_many_arguments)]
fn build_success_entry(
    profile: Option<&str>,
    tab_id: i64,
    url: &str,
    title: Option<&str>,
    html: &str,
    source_html_chars: i64,
    source_text_chars: i64,
    document_ready_state: Option<&str>,
    truncated_html: bool,
    incognito: bool,
    captured_at: i64,
) -> Option<PageCacheEntry> {
    if incognito
        || truncated_html
        || html.is_empty()
        || html.chars().count() > MAX_HTML_CHARS_PER_ENTRY
    {
        return None;
    }

    let profile = profile_key(profile);
    let url_key = url_key(url);
    Some(PageCacheEntry {
        profile,
        tab_id,
        canonical_url_key: canonical_url_key(url),
        url_key,
        title: title.map(str::to_string),
        html: html.to_string(),
        source_html_chars,
        source_text_chars,
        document_ready_state: document_ready_state.map(str::to_string),
        captured_at,
    })
}

fn read_exact_file_entry(
    file_path: &Path,
    profile: &str,
    tab_id: i64,
    url_key: &str,
) -> Result<Option<PageCacheEntry>, String> {
    let bytes = match fs::read(file_path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("read page cache: {err}")),
    };
    let Ok(entry) = serde_json::from_slice::<PageCacheEntry>(&bytes) else {
        let _ = fs::remove_file(file_path);
        return Ok(None);
    };
    if entry.profile != profile
        || entry.tab_id != tab_id
        || entry.url_key != url_key
        || !is_usable_entry(&entry)
    {
        let _ = fs::remove_file(file_path);
        return Ok(None);
    }
    Ok(Some(entry))
}

fn is_usable_entry(entry: &PageCacheEntry) -> bool {
    !entry.html.is_empty() && entry.html.chars().count() <= MAX_HTML_CHARS_PER_ENTRY
}

fn write_cache_entry(dir: &Path, entry: &PageCacheEntry) -> Result<(), String> {
    let file_path = cache_file_path(dir, entry);
    let tmp_path = file_path.with_extension(format!(
        "json.tmp.{}.{}",
        std::process::id(),
        crate::host_impl::protocol::now_ms()
    ));
    let bytes =
        serde_json::to_vec_pretty(entry).map_err(|err| format!("serialize page cache: {err}"))?;
    write_private_file(&tmp_path, &bytes).map_err(|err| {
        let _ = fs::remove_file(&tmp_path);
        err
    })?;
    fs::rename(&tmp_path, &file_path).map_err(|err| {
        let _ = fs::remove_file(&tmp_path);
        format!("replace page cache: {err}")
    })?;
    set_private_file_permissions(&file_path)
}

fn remove_stale_files_to_open_tabs(
    dir: &Path,
    profile: &str,
    open_tabs: &[OpenTabCacheKey],
) -> Result<(), String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(format!("read page cache directory: {err}")),
    };
    let open_exact_urls: HashSet<String> = open_tabs.iter().map(|tab| url_key(&tab.url)).collect();
    let open_canonical_urls: HashSet<String> = open_tabs
        .iter()
        .map(|tab| canonical_url_key(&tab.url))
        .collect();

    let mut files_read = 0usize;
    for dir_entry in entries.filter_map(Result::ok) {
        if files_read >= MAX_OPEN_TAB_CLEANUP_FILE_READS {
            break;
        }
        let file_path = dir_entry.path();
        if file_path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let bytes = match fs::read(&file_path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(format!("read page cache: {err}")),
        };
        files_read += 1;
        let Ok(entry) = serde_json::from_slice::<PageCacheFileMetadata>(&bytes) else {
            continue;
        };
        if entry.profile != profile {
            continue;
        }
        let canonical_url_key = if entry.canonical_url_key.is_empty() {
            canonical_url_key(&entry.url_key)
        } else {
            entry.canonical_url_key.clone()
        };
        if !open_exact_urls.contains(&entry.url_key)
            && !open_canonical_urls.contains(&canonical_url_key)
        {
            let _ = fs::remove_file(&file_path);
        }
    }
    Ok(())
}

fn remove_replaced_files_for_current_tab(
    dir: &Path,
    current: &PageCacheEntry,
) -> Result<(), String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(format!("read page cache directory: {err}")),
    };
    let current_path = cache_file_path(dir, current);
    let tab_prefix = format!("tab-{}-", current.tab_id);

    for dir_entry in entries.filter_map(Result::ok) {
        let file_path = dir_entry.path();
        if file_path == current_path
            || file_path.extension().and_then(|ext| ext.to_str()) != Some("json")
            || !dir_entry
                .file_name()
                .to_string_lossy()
                .starts_with(&tab_prefix)
        {
            continue;
        }

        let bytes = match fs::read(&file_path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(format!("read page cache: {err}")),
        };
        let Ok(entry) = serde_json::from_slice::<PageCacheFileMetadata>(&bytes) else {
            continue;
        };
        if entry.profile != current.profile || entry.tab_id != current.tab_id {
            continue;
        }
        let canonical_url_key = if entry.canonical_url_key.is_empty() {
            canonical_url_key(&entry.url_key)
        } else {
            entry.canonical_url_key
        };
        if entry.url_key != current.url_key && canonical_url_key != current.canonical_url_key {
            let _ = fs::remove_file(&file_path);
        }
    }
    Ok(())
}

fn enforce_file_caps(dir: &Path) -> Result<(), String> {
    enforce_file_caps_with_limits(dir, MAX_ENTRIES, MAX_TOTAL_HTML_BYTES)
}

fn enforce_file_caps_with_limits(
    dir: &Path,
    max_entries: usize,
    max_total_html_bytes: usize,
) -> Result<(), String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(format!("read page cache directory: {err}")),
    };

    let mut file_paths = Vec::new();
    let mut total_file_bytes = 0usize;
    for dir_entry in entries.filter_map(Result::ok) {
        let file_path = dir_entry.path();
        if file_path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if let Ok(metadata) = dir_entry.metadata() {
            total_file_bytes = total_file_bytes.saturating_add(metadata.len() as usize);
        }
        file_paths.push(file_path);
    }

    if file_paths.len() <= max_entries && total_file_bytes <= max_total_html_bytes {
        return Ok(());
    }

    let mut summaries = Vec::new();
    for file_path in file_paths {
        match read_cache_file_summary(&file_path)? {
            Some(summary) => summaries.push(summary),
            None => {
                let _ = fs::remove_file(file_path);
            }
        }
    }

    summaries.sort_by_key(|summary| summary.captured_at);
    let mut keep = HashSet::new();
    let mut total_html_bytes = 0usize;
    for summary in summaries.iter().rev() {
        if keep.len() >= max_entries
            || total_html_bytes.saturating_add(summary.html_bytes) > max_total_html_bytes
        {
            continue;
        }
        total_html_bytes += summary.html_bytes;
        keep.insert(summary.key.clone());
    }

    for summary in summaries {
        if !keep.contains(&summary.key) {
            let _ = fs::remove_file(summary.path);
        }
    }
    Ok(())
}

fn read_cache_file_summary(file_path: &Path) -> Result<Option<PageCacheFileSummary>, String> {
    let bytes = match fs::read(file_path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("read page cache: {err}")),
    };
    let Ok(entry) = serde_json::from_slice::<PageCacheEntry>(&bytes) else {
        return Ok(None);
    };
    if !is_usable_entry(&entry) {
        return Ok(None);
    }
    Ok(Some(PageCacheFileSummary {
        path: file_path.to_path_buf(),
        key: CacheKey {
            profile: entry.profile,
            tab_id: entry.tab_id,
            url_key: entry.url_key,
        },
        captured_at: entry.captured_at,
        html_bytes: entry.html.len(),
    }))
}

fn cache_file_hash(profile: &str, tab_id: i64, url_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(profile.as_bytes());
    hasher.update([0]);
    hasher.update(tab_id.to_string().as_bytes());
    hasher.update([0]);
    hasher.update(url_key.as_bytes());
    let digest = hasher.finalize();
    digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn profile_key(profile: Option<&str>) -> String {
    profile.unwrap_or_default().to_string()
}

fn url_key(url: &str) -> String {
    url.to_string()
}

fn canonical_url_key(url: &str) -> String {
    url.split_once('#')
        .map(|(without_fragment, _)| without_fragment)
        .unwrap_or(url)
        .to_string()
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    #[cfg(unix)]
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|err| format!("create private page cache temp file: {err}"))?;

    #[cfg(not(unix))]
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|err| format!("create page cache temp file: {err}"))?;

    file.write_all(bytes)
        .map_err(|err| format!("write page cache: {err}"))
}

fn set_private_dir_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|err| format!("set page cache directory permissions: {err}"))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn set_private_file_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|err| format!("set page cache file permissions: {err}"))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    fn cache_path(name: &str) -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("page-cache-tests")
            .join(format!("{}-{}-{}", name, std::process::id(), id));
        let _ = fs::remove_dir_all(&path);
        path
    }

    fn store(
        cache: &mut PageCache,
        profile: Option<&str>,
        tab_id: i64,
        url: &str,
        html: &str,
        captured_at: i64,
    ) {
        cache.store_success(
            profile,
            tab_id,
            url,
            Some("title"),
            html,
            html.chars().count() as i64,
            4,
            Some("complete"),
            false,
            false,
            captured_at,
        );
    }

    #[test]
    fn write_read_same_profile_tab_url() {
        let path = cache_path("roundtrip");
        let mut cache = PageCache::load(&path);
        store(
            &mut cache,
            Some("work"),
            7,
            "https://example.com/a?x=1",
            "<html>ok</html>",
            10,
        );
        cache.save_if_dirty(&path).expect("save cache");

        let loaded = PageCache::load(&path);
        let entry = loaded
            .lookup_open_tab(Some("work"), 7, "https://example.com/a?x=1")
            .expect("cached entry");
        assert_eq!(entry.match_mode, PageCacheMatchMode::Exact);
        assert_eq!(entry.entry.html, "<html>ok</html>");
        assert_eq!(entry.entry.title.as_deref(), Some("title"));
    }

    #[cfg(unix)]
    #[test]
    fn saved_cache_file_and_directory_are_private() {
        let path = cache_path("permissions");
        let mut cache = PageCache::default();
        store(&mut cache, Some("work"), 1, "https://example.com", "ok", 1);
        cache.save_if_dirty(&path).expect("save cache");

        let files: Vec<_> = fs::read_dir(&path)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        assert_eq!(files.len(), 1);
        let file_mode = fs::metadata(&files[0]).unwrap().permissions().mode() & 0o777;
        let dir_mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o600);
        assert_eq!(dir_mode, 0o700);
    }

    #[test]
    fn saved_cache_uses_one_bounded_private_file_per_entry() {
        let path = cache_path("file-layout");
        let mut cache = PageCache::default();
        store(
            &mut cache,
            Some("work-secret"),
            7,
            "https://example.com/private/path?token=secret#one",
            "one",
            1,
        );
        store(
            &mut cache,
            Some("work-secret"),
            7,
            "https://example.com/private/path?token=secret#two",
            "two",
            2,
        );
        cache.save_if_dirty(&path).expect("save cache");

        let mut file_names: Vec<_> = fs::read_dir(&path)
            .expect("read cache dir")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        file_names.sort();

        assert_eq!(file_names.len(), 2);
        for name in file_names {
            assert!(name.starts_with("tab-7-"), "{name}");
            assert!(name.ends_with(".json"), "{name}");
            assert!(name.len() <= 40, "{name}");
            assert!(!name.contains("work-secret"), "{name}");
            assert!(!name.contains("example.com"), "{name}");
            assert!(!name.contains("secret"), "{name}");
            assert!(!name.contains("private"), "{name}");
        }
    }

    #[test]
    fn profile_isolation() {
        let mut cache = PageCache::default();
        store(
            &mut cache,
            Some("work"),
            1,
            "https://example.com",
            "work",
            1,
        );
        store(
            &mut cache,
            Some("home"),
            1,
            "https://example.com",
            "home",
            2,
        );
        assert_eq!(
            cache
                .lookup_open_tab(Some("work"), 1, "https://example.com")
                .unwrap()
                .entry
                .html,
            "work"
        );
        assert_eq!(
            cache
                .lookup_open_tab(Some("home"), 1, "https://example.com")
                .unwrap()
                .entry
                .html,
            "home"
        );
        assert!(cache
            .lookup_open_tab(None, 1, "https://example.com")
            .is_none());
    }

    #[test]
    fn query_string_separation() {
        let mut cache = PageCache::default();
        store(
            &mut cache,
            None,
            1,
            "https://example.com/item?id=1",
            "one",
            1,
        );
        store(
            &mut cache,
            None,
            1,
            "https://example.com/item?id=2",
            "two",
            2,
        );
        assert_eq!(
            cache
                .lookup_open_tab(None, 1, "https://example.com/item?id=1")
                .unwrap()
                .entry
                .html,
            "one"
        );
        assert_eq!(
            cache
                .lookup_open_tab(None, 1, "https://example.com/item?id=2")
                .unwrap()
                .entry
                .html,
            "two"
        );
        assert!(cache
            .lookup_open_tab(None, 99, "https://example.com/item?id=3#other")
            .is_none());
    }

    #[test]
    fn exact_match_wins_over_canonical_match() {
        let mut cache = PageCache::default();
        store(
            &mut cache,
            None,
            1,
            "https://example.com/page?q=1#a",
            "a",
            1,
        );
        store(
            &mut cache,
            None,
            1,
            "https://example.com/page?q=1#b",
            "b",
            2,
        );
        let lookup = cache
            .lookup_open_tab(None, 1, "https://example.com/page?q=1#a")
            .unwrap();
        assert_eq!(lookup.match_mode, PageCacheMatchMode::Exact);
        assert_eq!(lookup.entry.html, "a");
    }

    #[test]
    fn exact_match_wins_over_newer_duplicate_exact_match() {
        let mut cache = PageCache::default();
        store(
            &mut cache,
            Some("work"),
            1,
            "https://example.com/page?q=1",
            "exact",
            10,
        );
        store(
            &mut cache,
            Some("work"),
            2,
            "https://example.com/page?q=1",
            "newer duplicate",
            20,
        );

        let lookup = cache
            .lookup_open_tab(Some("work"), 1, "https://example.com/page?q=1")
            .unwrap();
        assert_eq!(lookup.match_mode, PageCacheMatchMode::Exact);
        assert_eq!(lookup.entry.tab_id, 1);
        assert_eq!(lookup.entry.html, "exact");
    }

    #[test]
    fn canonical_fallback_matches_fragment_drift() {
        let mut cache = PageCache::default();
        store(
            &mut cache,
            None,
            1,
            "https://example.com/page?q=1#a",
            "a",
            1,
        );
        let lookup = cache
            .lookup_open_tab(None, 1, "https://example.com/page?q=1#b")
            .unwrap();
        assert_eq!(lookup.match_mode, PageCacheMatchMode::Canonical);
        assert_eq!(lookup.entry.html, "a");
        assert_eq!(
            cache
                .lookup_open_tab(None, 1, "https://example.com/page?q=1#a")
                .unwrap()
                .entry
                .html,
            "a"
        );
    }

    #[test]
    fn duplicate_urls_use_latest_matching_url_cache() {
        let mut cache = PageCache::default();
        store(
            &mut cache,
            Some("work"),
            1,
            "https://example.com/page?q=1",
            "old",
            10,
        );
        store(
            &mut cache,
            Some("work"),
            2,
            "https://example.com/page?q=1",
            "new",
            20,
        );

        let lookup = cache
            .lookup_open_tab(Some("work"), 99, "https://example.com/page?q=1")
            .expect("latest URL cache should survive tab id changes");
        assert_eq!(lookup.match_mode, PageCacheMatchMode::DuplicateExact);
        assert_eq!(lookup.entry.tab_id, 2);
        assert_eq!(lookup.entry.html, "new");
    }

    #[test]
    fn rejects_disallowed_writes() {
        let mut cache = PageCache::default();
        cache.store_success(
            None,
            1,
            "https://a",
            None,
            "secret",
            6,
            1,
            None,
            false,
            true,
            1,
        );
        cache.store_success(None, 2, "https://a", None, "", 0, 0, None, false, false, 1);
        cache.store_success(
            None,
            3,
            "https://a",
            None,
            "abc",
            3,
            1,
            None,
            true,
            false,
            1,
        );
        let oversized = "x".repeat(MAX_HTML_CHARS_PER_ENTRY + 1);
        cache.store_success(
            None,
            4,
            "https://a",
            None,
            &oversized,
            oversized.len() as i64,
            1,
            None,
            false,
            false,
            1,
        );
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn prune_closed_tabs_and_url_changed_tabs() {
        let mut cache = PageCache::default();
        store(
            &mut cache,
            Some("p"),
            1,
            "https://example.com/a?x=1",
            "keep",
            1,
        );
        store(
            &mut cache,
            Some("p"),
            2,
            "https://example.com/b",
            "closed",
            2,
        );
        store(
            &mut cache,
            Some("p"),
            3,
            "https://example.com/old",
            "changed",
            3,
        );
        store(
            &mut cache,
            Some("p"),
            4,
            "https://example.com/fragment#old",
            "canonical",
            4,
        );
        store(
            &mut cache,
            Some("p"),
            5,
            "https://example.com/unrelated#old",
            "unrelated",
            5,
        );
        store(
            &mut cache,
            Some("other"),
            2,
            "https://example.com/b",
            "other",
            4,
        );

        cache.prune_to_open_tabs(
            Some("p"),
            &[
                OpenTabCacheKey::new(1, "https://example.com/a?x=1"),
                OpenTabCacheKey::new(3, "https://example.com/new"),
                OpenTabCacheKey::new(4, "https://example.com/fragment#new"),
            ],
        );

        assert!(cache
            .lookup_open_tab(Some("p"), 1, "https://example.com/a?x=1")
            .is_some());
        assert!(cache
            .lookup_open_tab(Some("p"), 2, "https://example.com/b")
            .is_none());
        assert!(cache
            .lookup_exact_open_tab(Some("p"), 3, "https://example.com/old")
            .is_none());
        assert!(cache
            .lookup_exact_open_tab(Some("p"), 4, "https://example.com/fragment#old")
            .is_some());
        assert!(cache
            .lookup_exact_open_tab(Some("p"), 5, "https://example.com/unrelated#old")
            .is_none());
        assert!(cache
            .lookup_open_tab(Some("other"), 2, "https://example.com/b")
            .is_some());
    }

    #[test]
    fn evicts_oldest_for_max_entries_and_total_bytes() {
        let mut cache = PageCache::default();
        for i in 0..260 {
            store(
                &mut cache,
                None,
                i,
                &format!("https://example.com/{i}"),
                "x",
                i,
            );
        }
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        assert!(cache
            .lookup_open_tab(None, 0, "https://example.com/0")
            .is_none());
        assert!(cache
            .lookup_open_tab(None, 259, "https://example.com/259")
            .is_some());

        let mut byte_cache = PageCache::default();
        let html = "x".repeat(600_000);
        for i in 0..10 {
            store(
                &mut byte_cache,
                None,
                i,
                &format!("https://bytes.example/{i}"),
                &html,
                i,
            );
        }
        byte_cache.enforce_caps_with_limits(MAX_ENTRIES, 2_000_000);
        let total: usize = byte_cache
            .entries
            .values()
            .map(|entry| entry.html.len())
            .sum();
        assert!(total <= 2_000_000);
        assert!(byte_cache
            .lookup_open_tab(None, 0, "https://bytes.example/0")
            .is_none());
        assert!(byte_cache
            .lookup_open_tab(None, 9, "https://bytes.example/9")
            .is_some());
    }

    #[test]
    fn corrupted_cache_load_degrades_gracefully() {
        let path = cache_path("corrupt");
        fs::create_dir_all(&path).expect("create dir");
        fs::write(path.join("bad.json"), b"not-json").expect("write corrupt cache");
        let cache = PageCache::load(&path);
        assert!(cache.entries.is_empty());
        assert!(cache.dirty);
    }

    #[test]
    fn load_drops_invalid_entries_and_marks_dirty() {
        let path = cache_path("invalid-load");
        fs::create_dir_all(&path).expect("create dir");
        fs::write(
            path.join("entries.json"),
            serde_json::json!({
                "profile": "p",
                "tabId": 1,
                "urlKey": "https://example.com/empty",
                "title": null,
                "html": "",
                "sourceHtmlChars": 0,
                "sourceTextChars": 0,
                "documentReadyState": null,
                "capturedAt": 1
            })
            .to_string(),
        )
        .expect("write cache");
        fs::write(
            path.join("ok.json"),
            serde_json::json!({
                "profile": "p",
                "tabId": 2,
                "urlKey": "https://example.com/ok",
                "title": "Ok",
                "html": "<p>ok</p>",
                "sourceHtmlChars": 9,
                "sourceTextChars": 2,
                "documentReadyState": "complete",
                "capturedAt": 2
            })
            .to_string(),
        )
        .expect("write cache");

        let cache = PageCache::load(&path);
        assert!(cache.dirty);
        assert!(cache
            .lookup_open_tab(Some("p"), 1, "https://example.com/empty")
            .is_none());
        assert!(cache
            .lookup_open_tab(Some("p"), 2, "https://example.com/ok")
            .is_some());
    }

    #[test]
    fn atomic_save_creates_parent_dir_and_loads_again() {
        let path = cache_path("atomic");
        let mut cache = PageCache::default();
        store(
            &mut cache,
            None,
            42,
            "https://example.com/save",
            "saved",
            42,
        );
        cache.save_if_dirty(&path).expect("save");
        assert!(path.exists());
        assert_eq!(
            PageCache::load(&path)
                .lookup_open_tab(None, 42, "https://example.com/save")
                .unwrap()
                .entry
                .html,
            "saved"
        );
        let files: Vec<_> = fs::read_dir(&path)
            .expect("read cache dir")
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
            .collect();
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn fast_store_writes_single_exact_file_without_rewriting_unrelated_entries() {
        let path = cache_path("fast-store");
        let mut cache = PageCache::default();
        store(
            &mut cache,
            Some("other"),
            1,
            "https://example.com/other",
            "other",
            1,
        );
        cache.save_if_dirty(&path).expect("seed cache");
        let unrelated_path = fs::read_dir(&path)
            .expect("read cache dir")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
            .expect("unrelated cache file");
        let before = fs::metadata(&unrelated_path)
            .expect("metadata before")
            .modified()
            .expect("modified before");

        PageCache::store_success_file(
            &path,
            Some("work"),
            42,
            "https://example.com/page",
            Some("Page"),
            "<html>fast</html>",
            17,
            4,
            Some("complete"),
            false,
            false,
            2,
            None,
        )
        .expect("fast store");

        assert_eq!(
            fs::metadata(&unrelated_path)
                .expect("metadata after")
                .modified()
                .expect("modified after"),
            before
        );
        assert!(PageCache::exact_file_available(
            &path,
            Some("work"),
            42,
            "https://example.com/page"
        )
        .expect("exact status"));
        assert_eq!(
            fs::read_dir(&path)
                .expect("read cache dir")
                .filter_map(Result::ok)
                .filter(
                    |entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json")
                )
                .count(),
            2
        );
    }

    #[test]
    fn fast_store_enforces_max_entries_without_rewriting_retained_files() {
        let path = cache_path("fast-store-entry-cap");
        let mut cache = PageCache::default();
        for i in 0..MAX_ENTRIES {
            store(
                &mut cache,
                Some("work"),
                i as i64,
                &format!("https://example.com/{i}"),
                "seed",
                i as i64,
            );
        }
        cache.save_if_dirty(&path).expect("seed cache");
        let retained_path = cache_file_path_for_key(
            &path,
            "work",
            (MAX_ENTRIES - 1) as i64,
            &format!("https://example.com/{}", MAX_ENTRIES - 1),
        );
        let before = fs::metadata(&retained_path)
            .expect("metadata before")
            .modified()
            .expect("modified before");
        std::thread::sleep(Duration::from_millis(20));

        PageCache::store_success_file(
            &path,
            Some("work"),
            MAX_ENTRIES as i64,
            "https://example.com/new",
            Some("Page"),
            "<html>new</html>",
            16,
            3,
            Some("complete"),
            false,
            false,
            MAX_ENTRIES as i64,
            None,
        )
        .expect("fast store");

        assert_eq!(
            fs::read_dir(&path)
                .expect("read cache dir")
                .filter_map(Result::ok)
                .filter(
                    |entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json")
                )
                .count(),
            MAX_ENTRIES
        );
        assert!(
            !PageCache::exact_file_available(&path, Some("work"), 0, "https://example.com/0")
                .expect("oldest evicted")
        );
        assert!(PageCache::exact_file_available(
            &path,
            Some("work"),
            MAX_ENTRIES as i64,
            "https://example.com/new"
        )
        .expect("new retained"));
        assert_eq!(
            fs::metadata(&retained_path)
                .expect("metadata after")
                .modified()
                .expect("modified after"),
            before
        );
    }

    #[test]
    fn exact_file_status_deletes_known_stale_exact_file_only() {
        let path = cache_path("exact-stale");
        fs::create_dir_all(&path).expect("create dir");
        let exact_path = cache_file_path_for_key(&path, "work", 42, "https://example.com/page");
        let unrelated_path = cache_file_path_for_key(&path, "work", 43, "https://example.com/page");
        fs::write(&exact_path, b"not-json").expect("write corrupt exact cache");
        fs::write(&unrelated_path, b"not-json").expect("write corrupt unrelated cache");

        assert!(!PageCache::exact_file_available(
            &path,
            Some("work"),
            42,
            "https://example.com/page"
        )
        .expect("exact status"));
        assert!(!exact_path.exists());
        assert!(unrelated_path.exists());
    }

    #[test]
    fn fast_store_removes_stale_files_for_closed_tabs_from_open_snapshot() {
        let path = cache_path("fast-closed-tab-cleanup");
        let mut cache = PageCache::default();
        store(
            &mut cache,
            Some("work"),
            10,
            "https://example.com/keep",
            "keep",
            1,
        );
        store(
            &mut cache,
            Some("work"),
            11,
            "https://example.com/closed",
            "closed",
            2,
        );
        store(
            &mut cache,
            Some("other"),
            11,
            "https://example.com/closed",
            "other",
            3,
        );
        cache.save_if_dirty(&path).expect("seed cache");

        PageCache::store_success_file(
            &path,
            Some("work"),
            12,
            "https://example.com/current",
            Some("Page"),
            "<html>current</html>",
            20,
            3,
            Some("complete"),
            false,
            false,
            4,
            Some(&[
                OpenTabCacheKey::new(10, "https://example.com/keep"),
                OpenTabCacheKey::new(12, "https://example.com/current"),
            ]),
        )
        .expect("fast store");

        let loaded = PageCache::load(&path);
        assert!(loaded
            .lookup_exact_open_tab(Some("work"), 10, "https://example.com/keep")
            .is_some());
        assert!(loaded
            .lookup_exact_open_tab(Some("work"), 11, "https://example.com/closed")
            .is_none());
        assert!(loaded
            .lookup_exact_open_tab(Some("work"), 12, "https://example.com/current")
            .is_some());
        assert!(loaded
            .lookup_exact_open_tab(Some("other"), 11, "https://example.com/closed")
            .is_some());
    }

    #[test]
    fn fast_store_removes_replaced_url_cache_and_keeps_new_entry() {
        let path = cache_path("fast-replaced-url-cleanup");
        let mut cache = PageCache::default();
        store(
            &mut cache,
            Some("work"),
            42,
            "https://example.com/old#section",
            "old",
            1,
        );
        cache.save_if_dirty(&path).expect("seed cache");

        PageCache::store_success_file(
            &path,
            Some("work"),
            42,
            "https://example.com/new#section",
            Some("Page"),
            "<html>new</html>",
            16,
            3,
            Some("complete"),
            false,
            false,
            2,
            Some(&[OpenTabCacheKey::new(42, "https://example.com/new#section")]),
        )
        .expect("fast store");

        let loaded = PageCache::load(&path);
        assert!(loaded
            .lookup_exact_open_tab(Some("work"), 42, "https://example.com/old#section")
            .is_none());
        assert!(loaded
            .lookup_exact_open_tab(Some("work"), 42, "https://example.com/new#section")
            .is_some());
        assert!(PageCache::exact_file_available(
            &path,
            Some("work"),
            42,
            "https://example.com/new#section"
        )
        .expect("exact status"));
    }

    #[test]
    fn fast_store_does_not_rewrite_retained_profile_files() {
        let path = cache_path("fast-retained-no-rewrite");
        let mut cache = PageCache::default();
        store(
            &mut cache,
            Some("work"),
            10,
            "https://example.com/retained",
            "retained",
            1,
        );
        cache.save_if_dirty(&path).expect("seed cache");
        let retained_path =
            cache_file_path_for_key(&path, "work", 10, "https://example.com/retained");
        let before = fs::metadata(&retained_path)
            .expect("metadata before")
            .modified()
            .expect("modified before");
        std::thread::sleep(Duration::from_millis(20));

        PageCache::store_success_file(
            &path,
            Some("work"),
            12,
            "https://example.com/current",
            Some("Page"),
            "<html>current</html>",
            20,
            3,
            Some("complete"),
            false,
            false,
            2,
            Some(&[
                OpenTabCacheKey::new(10, "https://example.com/retained"),
                OpenTabCacheKey::new(12, "https://example.com/current"),
            ]),
        )
        .expect("fast store");

        assert_eq!(
            fs::metadata(&retained_path)
                .expect("metadata after")
                .modified()
                .expect("modified after"),
            before
        );
        let loaded = PageCache::load(&path);
        assert!(loaded
            .lookup_exact_open_tab(Some("work"), 10, "https://example.com/retained")
            .is_some());
    }

    #[test]
    fn exact_file_status_remains_exact_not_canonical_or_duplicate() {
        let path = cache_path("exact-status-exact-only");
        PageCache::store_success_file(
            &path,
            Some("work"),
            42,
            "https://example.com/page#section",
            Some("Page"),
            "<html>exact</html>",
            18,
            3,
            Some("complete"),
            false,
            false,
            1,
            None,
        )
        .expect("store exact");

        assert!(PageCache::exact_file_available(
            &path,
            Some("work"),
            42,
            "https://example.com/page#section"
        )
        .expect("exact hit"));
        assert!(!PageCache::exact_file_available(
            &path,
            Some("work"),
            42,
            "https://example.com/page#other"
        )
        .expect("canonical miss"));
        assert!(!PageCache::exact_file_available(
            &path,
            Some("work"),
            7,
            "https://example.com/page#section"
        )
        .expect("duplicate-tab miss"));
    }
}
