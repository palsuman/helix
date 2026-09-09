//! Indexed workspace search (Task 4.5, REQ-SEARCH-001).

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::UNIX_EPOCH;

use globset::Glob;
use helix_core::error::AppError;
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::exclude::{ExclusionConfig, Exclusions};

pub const SEARCH_HISTORY_LIMIT: usize = 50;
pub const DEFAULT_INDEX_CAP_BYTES: usize = 200 * 1024 * 1024;
pub const MAX_INDEXED_FILE_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SearchQuery {
    pub root: String,
    pub query: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub max_results: usize,
    #[serde(default)]
    pub regex: bool,
    #[serde(default)]
    pub include_glob: Option<String>,
    #[serde(default)]
    pub exclude_glob: Option<String>,
    #[serde(default = "default_context_lines")]
    pub context_lines: u8,
    #[serde(default = "default_respect_gitignore")]
    pub respect_gitignore: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SearchMatch {
    pub path: String,
    pub line: u32,
    pub column: u32,
    pub text: String,
    pub context_before: Vec<String>,
    pub context_after: Vec<String>,
    pub version_hash: String,
}

/// A path-index query used by Quick Open. Content is deliberately absent: the
/// path index is the single source for this surface and file contents never
/// cross IPC just to render a picker row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct QuickOpenQuery {
    pub root: String,
    pub query: String,
    pub max_results: usize,
    #[serde(default)]
    pub recent_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct QuickOpenMatch {
    pub path: String,
    pub relative_path: String,
    pub file_name: String,
    pub score: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct ReplaceRequest {
    pub search: SearchQuery,
    pub replacement: String,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub preview_only: bool,
    #[serde(default)]
    pub cancel_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct ReplaceFileResult {
    pub path: String,
    pub before: String,
    pub after: String,
    pub match_count: u32,
    pub skipped: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct ReplaceResponse {
    pub operation_id: String,
    pub files: Vec<ReplaceFileResult>,
    pub preview: bool,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct UndoRequest {
    pub operation_id: String,
}

fn default_context_lines() -> u8 {
    0
}
fn default_respect_gitignore() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SearchStats {
    pub indexed_files: usize,
    pub indexed_bytes: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IndexedFile {
    path: String,
    text: String,
    bytes: usize,
    modified_ms: u128,
    hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedIndex {
    files: Vec<IndexedFile>,
    history: VecDeque<SearchQuery>,
}

pub struct SearchIndex {
    root: PathBuf,
    exclusions: Exclusions,
    files: HashMap<String, IndexedFile>,
    path_trigrams: HashMap<String, Vec<String>>,
    content_trigrams: HashMap<String, Vec<String>>,
    history: VecDeque<SearchQuery>,
    cap_bytes: usize,
    indexed_bytes: usize,
    truncated: bool,
}

impl SearchIndex {
    pub fn new(root: impl Into<PathBuf>, exclusions: ExclusionConfig) -> Self {
        let root = root.into();
        Self {
            exclusions: Exclusions::build(&root, &exclusions),
            root,
            files: HashMap::new(),
            path_trigrams: HashMap::new(),
            content_trigrams: HashMap::new(),
            history: VecDeque::new(),
            cap_bytes: DEFAULT_INDEX_CAP_BYTES,
            indexed_bytes: 0,
            truncated: false,
        }
    }

    pub fn with_cap_bytes(mut self, cap_bytes: usize) -> Self {
        self.cap_bytes = cap_bytes;
        self
    }

    pub fn build(&mut self) -> io::Result<SearchStats> {
        self.walk(self.root.clone())?;
        Ok(self.stats())
    }

    /// Populate only path data for the Quick Open fallback. Unlike the full
    /// content index this never reads file bodies, so an unbuilt index does not
    /// make the picker wait for content indexing.
    fn build_paths(&mut self) -> io::Result<()> {
        self.walk_paths(self.root.clone())
    }

    pub fn update_path(&mut self, path: &Path) -> io::Result<()> {
        let key = path.to_string_lossy().into_owned();
        self.remove_path(&key);
        if path.is_file() && !self.exclusions.is_excluded(path, false) {
            self.index_file(path)?;
        }
        Ok(())
    }

    pub fn remove_path(&mut self, path: &str) {
        if let Some(file) = self.files.remove(path) {
            self.indexed_bytes = self.indexed_bytes.saturating_sub(file.bytes);
            remove_refs(&mut self.path_trigrams, path, path);
            remove_refs(&mut self.content_trigrams, &file.text, path);
        }
    }

    pub fn search(&mut self, query: &SearchQuery) -> Vec<SearchMatch> {
        if query.query.is_empty() {
            return Vec::new();
        }
        self.history.retain(|item| item != query);
        self.history.push_front(query.clone());
        self.history.truncate(SEARCH_HISTORY_LIMIT);

        let pattern = if query.regex {
            RegexBuilder::new(&query.query)
                .case_insensitive(!query.case_sensitive)
                .build()
                .ok()
        } else {
            let source = regex::escape(&query.query);
            let source = if query.whole_word {
                format!(r"\b(?:{source})\b")
            } else {
                source
            };
            RegexBuilder::new(&source)
                .case_insensitive(!query.case_sensitive)
                .build()
                .ok()
        };
        let Some(pattern) = pattern else {
            return Vec::new();
        };
        let mut results = Vec::new();
        for file in self.files.values() {
            if !query_path_allowed(&self.root, &file.path, query) {
                continue;
            }
            let lines: Vec<&str> = file.text.lines().collect();
            for (line_index, line) in lines.iter().enumerate() {
                for found in pattern.find_iter(line) {
                    let start = found.start();
                    let end = found.end();
                    if !query.whole_word || word_boundary(line, start, end) {
                        let before = lines
                            .iter()
                            .skip(line_index.saturating_sub(query.context_lines as usize))
                            .take(
                                line_index
                                    - line_index.saturating_sub(query.context_lines as usize),
                            )
                            .map(|value| (*value).to_string())
                            .collect();
                        let after = lines
                            .iter()
                            .skip(line_index + 1)
                            .take(query.context_lines as usize)
                            .map(|value| (*value).to_string())
                            .collect();
                        results.push(SearchMatch {
                            path: file.path.clone(),
                            line: line_index as u32 + 1,
                            column: start as u32 + 1,
                            text: (*line).to_string(),
                            context_before: before,
                            context_after: after,
                            version_hash: file.hash.clone(),
                        });
                        if results.len() >= query.max_results.max(1) {
                            return results;
                        }
                    }
                }
            }
        }
        results
    }

    /// Rank files from the path trigram index. Exact filename matches form the
    /// first tier, path-segment matches the second, and subsequence matches the
    /// third. Recency only reorders within a tier, preserving match quality.
    pub fn quick_open(&self, query: &QuickOpenQuery) -> Vec<QuickOpenMatch> {
        let needle = query.query.trim().to_lowercase();
        let recent = query
            .recent_files
            .iter()
            .enumerate()
            .map(|(index, path)| (path.as_str(), (query.recent_files.len() - index) as i64))
            .collect::<HashMap<_, _>>();

        let mut candidates: Vec<&str> = if needle.chars().count() >= 3 {
            let keys = trigrams(&needle);
            // Start from the rarest trigram. Counting every posting for every
            // key makes a common prefix such as "component" O(keys × files).
            let indexed = keys
                .iter()
                .filter_map(|key| self.path_trigrams.get(key))
                .min_by_key(|paths| paths.len())
                .map(|paths| {
                    paths
                        .iter()
                        .filter(|path| {
                            let normalized = path.to_lowercase();
                            keys.iter().all(|key| normalized.contains(key))
                        })
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            // A trigram intersection cannot represent every subsequence query.
            // Falling back to the index's file keys retains fuzzy semantics.
            if indexed.is_empty() {
                self.files.keys().map(String::as_str).collect()
            } else {
                indexed
            }
        } else {
            self.files.keys().map(String::as_str).collect()
        };
        candidates.sort_unstable();
        candidates.dedup();

        let mut matches = candidates
            .into_iter()
            .filter_map(|path| {
                let relative = Path::new(path)
                    .strip_prefix(&self.root)
                    .unwrap_or_else(|_| Path::new(path))
                    .to_string_lossy()
                    .replace('\\', "/");
                let file_name = Path::new(path)
                    .file_name()
                    .map(|value| value.to_string_lossy().into_owned())
                    .unwrap_or_else(|| relative.clone());
                path_score(&file_name, &relative, &needle).map(|base| QuickOpenMatch {
                    score: base + recent.get(path).copied().unwrap_or_default().min(99),
                    path: path.to_string(),
                    relative_path: relative,
                    file_name,
                })
            })
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.relative_path.len().cmp(&right.relative_path.len()))
                .then_with(|| left.relative_path.cmp(&right.relative_path))
        });
        matches.truncate(query.max_results.max(1));
        matches
    }

    pub fn history(&self) -> Vec<SearchQuery> {
        self.history.iter().cloned().collect()
    }

    pub fn stats(&self) -> SearchStats {
        SearchStats {
            indexed_files: self.files.len(),
            indexed_bytes: self.indexed_bytes,
            truncated: self.truncated,
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let data = PersistedIndex {
            files: self.files.values().cloned().collect(),
            history: self.history.clone(),
        };
        fs::write(path, serde_json::to_vec(&data).map_err(io::Error::other)?)
    }

    pub fn load(&mut self, path: &Path) -> io::Result<()> {
        let data: PersistedIndex =
            serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)?;
        self.files.clear();
        self.path_trigrams.clear();
        self.content_trigrams.clear();
        self.indexed_bytes = 0;
        self.history = data.history;
        for file in data.files {
            let path = PathBuf::from(&file.path);
            if path.exists() && modified_ms(&path) == file.modified_ms {
                self.insert(file);
            }
        }
        Ok(())
    }

    fn walk(&mut self, directory: PathBuf) -> io::Result<()> {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if self.exclusions.is_excluded(&path, path.is_dir()) {
                continue;
            }
            if path.is_dir() {
                self.walk(path)?;
            } else {
                self.index_file(&path)?;
            }
        }
        Ok(())
    }

    fn walk_paths(&mut self, directory: PathBuf) -> io::Result<()> {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if self.exclusions.is_excluded(&path, path.is_dir()) {
                continue;
            }
            if path.is_dir() {
                self.walk_paths(path)?;
            } else {
                let key = path.to_string_lossy().into_owned();
                add_refs(&mut self.path_trigrams, &key, &key);
                self.files.insert(
                    key.clone(),
                    IndexedFile {
                        path: key,
                        text: String::new(),
                        bytes: 0,
                        modified_ms: 0,
                        hash: String::new(),
                    },
                );
            }
        }
        Ok(())
    }

    fn index_file(&mut self, path: &Path) -> io::Result<()> {
        let metadata = fs::metadata(path)?;
        if metadata.len() > MAX_INDEXED_FILE_BYTES
            || self.indexed_bytes + metadata.len() as usize > self.cap_bytes
        {
            self.truncated = true;
            return Ok(());
        }
        let bytes = fs::read(path)?;
        if bytes.contains(&0) {
            return Ok(());
        }
        self.insert(IndexedFile {
            path: path.to_string_lossy().into_owned(),
            text: String::from_utf8_lossy(&bytes).into_owned(),
            bytes: bytes.len(),
            modified_ms: modified_ms(path),
            hash: crate::hash::hash_bytes(&bytes).to_string(),
        });
        Ok(())
    }

    fn insert(&mut self, file: IndexedFile) {
        self.indexed_bytes += file.bytes;
        add_refs(&mut self.path_trigrams, &file.path, &file.path);
        add_refs(&mut self.content_trigrams, &file.text, &file.path);
        self.files.insert(file.path.clone(), file);
    }
}

#[derive(Clone, Default)]
pub struct SearchService {
    indexes: Arc<RwLock<HashMap<String, SearchIndex>>>,
    undo: Arc<Mutex<HashMap<String, Vec<(String, String, String)>>>>,
    cancelled: Arc<Mutex<std::collections::HashSet<String>>>,
    building: Arc<Mutex<std::collections::HashSet<String>>>,
}

impl SearchService {
    pub fn new() -> Self {
        Self {
            indexes: Arc::new(RwLock::new(HashMap::new())),
            undo: Arc::new(Mutex::new(HashMap::new())),
            cancelled: Arc::new(Mutex::new(std::collections::HashSet::new())),
            building: Arc::new(Mutex::new(std::collections::HashSet::new())),
        }
    }

    pub fn search(&self, query: &SearchQuery) -> io::Result<Vec<SearchMatch>> {
        let mut indexes = self.indexes.write().unwrap();
        let index = indexes
            .entry(query.root.clone())
            .or_insert_with(|| SearchIndex::new(&query.root, ExclusionConfig::default()));
        if index.stats().indexed_files == 0 {
            index.build()?;
        }
        Ok(index.search(query))
    }

    pub fn quick_open(&self, query: &QuickOpenQuery) -> io::Result<(Vec<QuickOpenMatch>, bool)> {
        if let Some(index) = self.indexes.read().unwrap().get(&query.root)
            && index.stats().indexed_files > 0
        {
            return Ok((index.quick_open(query), false));
        }

        let mut fallback = SearchIndex::new(&query.root, ExclusionConfig::default());
        fallback.build_paths()?;
        let matches = fallback.quick_open(query);

        let should_start = self.building.lock().unwrap().insert(query.root.clone());
        if should_start {
            let root = query.root.clone();
            let indexes = self.indexes.clone();
            let building = self.building.clone();
            std::thread::spawn(move || {
                let mut index = SearchIndex::new(&root, ExclusionConfig::default());
                if index.build().is_ok() {
                    indexes.write().unwrap().insert(root.clone(), index);
                }
                building.lock().unwrap().remove(&root);
            });
        }
        Ok((matches, true))
    }

    pub fn cancel(&self, id: &str) {
        self.cancelled.lock().unwrap().insert(id.to_string());
    }

    pub fn replace(
        &self,
        fs_service: &crate::service::FileSystemService,
        request: &ReplaceRequest,
    ) -> Result<ReplaceResponse, AppError> {
        let operation_id = format!("replace-{}", modified_ms(Path::new(&request.search.root)));
        let matches = self
            .search(&request.search)
            .map_err(|error| AppError::permanent("SEARCH_FAILED", error.to_string()))?;
        let mut files = Vec::new();
        let mut undo = Vec::new();
        let selected = request
            .paths
            .iter()
            .collect::<std::collections::HashSet<_>>();
        for path in matches
            .iter()
            .map(|item| item.path.clone())
            .collect::<std::collections::HashSet<_>>()
        {
            if !selected.is_empty() && !selected.contains(&path) {
                continue;
            }
            if request
                .cancel_id
                .as_ref()
                .is_some_and(|id| self.cancelled.lock().unwrap().contains(id))
            {
                return Ok(ReplaceResponse {
                    operation_id,
                    files,
                    preview: request.preview_only,
                    cancelled: true,
                });
            }
            let content = match fs_service.read(&path) {
                Ok(value) => value,
                Err(error) => {
                    files.push(ReplaceFileResult {
                        path,
                        before: String::new(),
                        after: String::new(),
                        match_count: 0,
                        skipped: true,
                        error: Some(error.message),
                    });
                    continue;
                }
            };
            let before = content.text.unwrap_or_default();
            let after = replace_text(&before, &request.search, &request.replacement)
                .map_err(|error| AppError::permanent("SEARCH_REGEX_INVALID", error))?;
            let count = matches.iter().filter(|item| item.path == path).count() as u32;
            let changed = before != after;
            if !request.preview_only && changed {
                let outcome = fs_service.write(
                    &path,
                    crate::service::WriteOptions {
                        text: after.clone(),
                        expected_hash: Some(content.hash.clone()),
                        ..crate::service::WriteOptions::new(after.clone())
                    },
                );
                if let Err(error) = outcome {
                    files.push(ReplaceFileResult {
                        path,
                        before,
                        after: String::new(),
                        match_count: count,
                        skipped: true,
                        error: Some(error.message),
                    });
                    continue;
                }
                undo.push((path.clone(), before.clone(), content.hash));
            }
            files.push(ReplaceFileResult {
                path,
                before,
                after,
                match_count: count,
                skipped: !changed,
                error: None,
            });
        }
        if !request.preview_only {
            self.undo.lock().unwrap().insert(operation_id.clone(), undo);
        }
        Ok(ReplaceResponse {
            operation_id,
            files,
            preview: request.preview_only,
            cancelled: false,
        })
    }

    pub fn undo(
        &self,
        fs_service: &crate::service::FileSystemService,
        request: &UndoRequest,
    ) -> Result<ReplaceResponse, AppError> {
        let entries = self
            .undo
            .lock()
            .unwrap()
            .remove(&request.operation_id)
            .unwrap_or_default();
        let mut files = Vec::new();
        for (path, before, expected_hash) in entries {
            let current = fs_service.read(&path)?;
            if current.hash != expected_hash {
                files.push(ReplaceFileResult {
                    path,
                    before,
                    after: String::new(),
                    match_count: 0,
                    skipped: true,
                    error: Some("file changed since replacement".into()),
                });
                continue;
            }
            fs_service.write(
                &path,
                crate::service::WriteOptions {
                    text: before.clone(),
                    expected_hash: Some(current.hash),
                    ..crate::service::WriteOptions::new(before.clone())
                },
            )?;
            files.push(ReplaceFileResult {
                path,
                before: String::new(),
                after: before,
                match_count: 0,
                skipped: false,
                error: None,
            });
        }
        Ok(ReplaceResponse {
            operation_id: request.operation_id.clone(),
            files,
            preview: false,
            cancelled: false,
        })
    }

    pub fn stats(&self, root: &str) -> Option<SearchStats> {
        self.indexes
            .read()
            .unwrap()
            .get(root)
            .map(SearchIndex::stats)
    }
}

fn replace_text(text: &str, query: &SearchQuery, replacement: &str) -> Result<String, String> {
    let source = if query.regex {
        query.query.clone()
    } else {
        regex::escape(&query.query)
    };
    let source = if query.whole_word {
        format!(r"\b(?:{source})\b")
    } else {
        source
    };
    let regex = RegexBuilder::new(&source)
        .case_insensitive(!query.case_sensitive)
        .build()
        .map_err(|error| error.to_string())?;
    Ok(regex.replace_all(text, replacement).into_owned())
}

fn modified_ms(path: &Path) -> u128 {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn word_boundary(line: &str, start: usize, end: usize) -> bool {
    let is_word = |character: char| character.is_alphanumeric() || character == '_';
    !line[..start].chars().next_back().is_some_and(is_word)
        && !line[end..].chars().next().is_some_and(is_word)
}

fn trigrams(value: &str) -> Vec<String> {
    let chars: Vec<char> = value.to_lowercase().chars().collect();
    if chars.len() < 3 {
        return vec![chars.into_iter().collect()];
    }
    chars
        .windows(3)
        .map(|window| window.iter().collect())
        .collect()
}

fn path_score(file_name: &str, relative_path: &str, needle: &str) -> Option<i64> {
    if needle.is_empty() {
        return Some(1_000_000);
    }
    let name = file_name.to_lowercase();
    let path = relative_path.to_lowercase();
    if name == needle {
        return Some(3_000_000);
    }
    let best_segment = path
        .split('/')
        .filter_map(|segment| segment.find(needle).map(|offset| (segment, offset)))
        .max_by_key(|(segment, offset)| (i64::from(*offset == 0), -(segment.len() as i64)))
        .map(|(segment, offset)| 2_000_000 - (offset as i64 * 100) - segment.len() as i64);
    if best_segment.is_some() {
        return best_segment;
    }

    let mut position = 0usize;
    let mut first = None;
    let mut last = 0usize;
    for character in needle.chars() {
        let found = path[position..].find(character)? + position;
        first.get_or_insert(found);
        last = found;
        position = found + character.len_utf8();
    }
    let span = last.saturating_sub(first.unwrap_or_default());
    Some(1_000_000 - span as i64 * 100 - path.len() as i64)
}

fn add_refs(index: &mut HashMap<String, Vec<String>>, value: &str, path: &str) {
    for key in trigrams(value) {
        index.entry(key).or_default().push(path.to_string());
    }
}

fn remove_refs(index: &mut HashMap<String, Vec<String>>, value: &str, path: &str) {
    for key in trigrams(value) {
        if let Some(values) = index.get_mut(&key) {
            values.retain(|candidate| candidate != path);
            if values.is_empty() {
                index.remove(&key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::time::{Duration, Instant};

    #[test]
    fn indexes_searches_and_excludes_generated_directories() {
        let dir = TempDir::new("search");
        dir.write("src/main.rs", "fn main() {\n let value = 1;\n}");
        dir.write("node_modules/ignored.js", "value");
        let mut index = SearchIndex::new(dir.path(), ExclusionConfig::default());
        index.build().unwrap();
        let results = index.search(&SearchQuery {
            root: dir.path().display().to_string(),
            query: "value".into(),
            case_sensitive: false,
            whole_word: true,
            max_results: 20,
            regex: false,
            include_glob: None,
            exclude_glob: None,
            context_lines: 0,
            respect_gitignore: true,
        });
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].line, 2);
    }

    #[test]
    fn persisted_index_drops_files_changed_since_snapshot() {
        let dir = TempDir::new("search-persist");
        let file = dir.write("main.rs", "alpha");
        let state = dir.path().join("index.json");
        let mut index = SearchIndex::new(dir.path(), ExclusionConfig::permissive());
        index.build().unwrap();
        index.save(&state).unwrap();
        std::fs::write(file, "beta").unwrap();
        let mut restored = SearchIndex::new(dir.path(), ExclusionConfig::permissive());
        restored.load(&state).unwrap();
        assert!(
            restored
                .search(&SearchQuery {
                    root: dir.path().display().to_string(),
                    query: "alpha".into(),
                    case_sensitive: false,
                    whole_word: false,
                    max_results: 20,
                    regex: false,
                    include_glob: None,
                    exclude_glob: None,
                    context_lines: 0,
                    respect_gitignore: true,
                })
                .is_empty()
        );
    }

    #[test]
    fn replacement_supports_regex_and_whole_words() {
        let query = SearchQuery {
            root: "/tmp".into(),
            query: "item\\d+".into(),
            case_sensitive: false,
            whole_word: false,
            max_results: 20,
            regex: true,
            include_glob: None,
            exclude_glob: None,
            context_lines: 0,
            respect_gitignore: true,
        };
        assert_eq!(
            replace_text("item1 item22", &query, "value").unwrap(),
            "value value"
        );
        let whole_word = SearchQuery {
            query: "cat".into(),
            regex: false,
            whole_word: true,
            ..query
        };
        assert_eq!(
            replace_text("cat cater", &whole_word, "dog").unwrap(),
            "dog cater"
        );
    }

    #[test]
    fn quick_open_prefers_filename_then_segment_then_fuzzy_and_honours_recency() {
        let dir = TempDir::new("quick-open-rank");
        let exact = dir.write("nested/main.rs", "exact");
        dir.write("main.rs/other.txt", "segment");
        dir.write("src/m_a_i_n_notes.txt", "fuzzy");
        let recent = dir.write("recent/main.rs", "recent");
        let mut index = SearchIndex::new(dir.path(), ExclusionConfig::permissive());
        index.build().unwrap();

        let matches = index.quick_open(&QuickOpenQuery {
            root: dir.path().display().to_string(),
            query: "main.rs".into(),
            max_results: 20,
            recent_files: vec![recent.display().to_string()],
        });

        assert_eq!(matches[0].path, recent.display().to_string());
        assert_eq!(matches[1].path, exact.display().to_string());
        assert!(matches[0].score >= 3_000_000);
        assert!(
            matches
                .iter()
                .any(|item| item.relative_path == "main.rs/other.txt")
        );
    }

    #[test]
    fn first_quick_open_request_reports_the_directory_scan_fallback() {
        let dir = TempDir::new("quick-open-fallback");
        dir.write("src/application.ts", "export {};");
        let service = SearchService::new();
        let query = QuickOpenQuery {
            root: dir.path().display().to_string(),
            query: "app".into(),
            max_results: 10,
            recent_files: vec![],
        };
        let (first, indexing) = service.quick_open(&query).unwrap();
        assert_eq!(first[0].relative_path, "src/application.ts");
        assert!(indexing);
        let started = Instant::now();
        loop {
            let (_, subsequent_indexing) = service.quick_open(&query).unwrap();
            if !subsequent_indexing {
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(2));
            std::thread::yield_now();
        }
    }

    #[test]
    fn quick_open_queries_a_hundred_thousand_path_index_within_fifty_ms() {
        let mut index = SearchIndex::new("/workspace", ExclusionConfig::permissive());
        for number in 0..100_000 {
            let path = format!("/workspace/src/component_{number:05}.ts");
            index.insert(IndexedFile {
                path,
                text: String::new(),
                bytes: 0,
                modified_ms: 0,
                hash: String::new(),
            });
        }
        let started = Instant::now();
        let matches = index.quick_open(&QuickOpenQuery {
            root: "/workspace".into(),
            query: "99999".into(),
            max_results: 100,
            recent_files: vec![],
        });
        assert!(!matches.is_empty());
        assert!(
            started.elapsed() < Duration::from_millis(50),
            "100k-file Quick Open took {:?}",
            started.elapsed()
        );
    }
}

fn query_path_allowed(root: &Path, path: &str, query: &SearchQuery) -> bool {
    let relative = Path::new(path)
        .strip_prefix(root)
        .unwrap_or_else(|_| Path::new(path));
    if let Some(pattern) = &query.include_glob {
        if Glob::new(pattern)
            .map(|glob| glob.compile_matcher().is_match(relative))
            .unwrap_or(false)
            == false
        {
            return false;
        }
    }
    if let Some(pattern) = &query.exclude_glob {
        if Glob::new(pattern)
            .map(|glob| glob.compile_matcher().is_match(relative))
            .unwrap_or(false)
        {
            return false;
        }
    }
    true
}
