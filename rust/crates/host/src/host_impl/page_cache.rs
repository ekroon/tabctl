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

const MAX_HTML_CHARS_PER_ENTRY: usize = 1_500_000;
const MAX_ENTRIES: usize = 250;
const MAX_TOTAL_HTML_BYTES: usize = 128 * 1024 * 1024;

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
        if incognito
            || truncated_html
            || html.is_empty()
            || html.chars().count() > MAX_HTML_CHARS_PER_ENTRY
        {
            return;
        }

        let profile = profile_key(profile);
        let url_key = url_key(url);
        let canonical_url_key = canonical_url_key(url);
        let key = CacheKey {
            profile: profile.clone(),
            tab_id,
            url_key: url_key.clone(),
        };
        self.entries.insert(
            key,
            PageCacheEntry {
                profile,
                tab_id,
                url_key,
                canonical_url_key,
                title: title.map(str::to_string),
                html: html.to_string(),
                source_html_chars,
                source_text_chars,
                document_ready_state: document_ready_state.map(str::to_string),
                captured_at,
            },
        );
        self.dirty = true;
        self.enforce_caps();
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
            let file_path = cache_file_path(path, &entry);
            let tmp_path = file_path.with_extension(format!(
                "json.tmp.{}.{}",
                std::process::id(),
                crate::host_impl::protocol::now_ms()
            ));
            let bytes = serde_json::to_vec_pretty(&entry)
                .map_err(|err| format!("serialize page cache: {err}"))?;
            write_private_file(&tmp_path, &bytes).map_err(|err| {
                let _ = fs::remove_file(&tmp_path);
                err
            })?;
            fs::rename(&tmp_path, &file_path).map_err(|err| {
                let _ = fs::remove_file(&tmp_path);
                format!("replace page cache: {err}")
            })?;
            set_private_file_permissions(&file_path)?;
        }
        self.dirty = false;
        Ok(())
    }

    fn enforce_caps(&mut self) {
        let mut entries: Vec<_> = self.entries.values().cloned().collect();
        entries.sort_by_key(|entry| entry.captured_at);

        let mut keep = HashSet::new();
        let mut total_bytes = 0usize;
        for entry in entries.into_iter().rev() {
            let html_bytes = entry.html.len();
            if keep.len() >= MAX_ENTRIES
                || total_bytes.saturating_add(html_bytes) > MAX_TOTAL_HTML_BYTES
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
    dir.join(format!(
        "tab-{}-{}.json",
        entry.tab_id,
        cache_file_hash(&entry.profile, entry.tab_id, &entry.url_key)
    ))
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
        .map_err(|err| format!("write page cache: {err}"))?;
    file.sync_all()
        .map_err(|err| format!("sync page cache: {err}"))
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
        for i in 0..230 {
            store(
                &mut byte_cache,
                None,
                i,
                &format!("https://bytes.example/{i}"),
                &html,
                i,
            );
        }
        let total: usize = byte_cache
            .entries
            .values()
            .map(|entry| entry.html.len())
            .sum();
        assert!(total <= MAX_TOTAL_HTML_BYTES);
        assert!(byte_cache
            .lookup_open_tab(None, 0, "https://bytes.example/0")
            .is_none());
        assert!(byte_cache
            .lookup_open_tab(None, 229, "https://bytes.example/229")
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
}
