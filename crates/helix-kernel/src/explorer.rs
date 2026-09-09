//! Paginated explorer snapshots and workspace-scoped file operations.
use helix_core::error::AppError;
use helix_fs::{FileEntry, FileSystemService};
use helix_ipc::IpcDispatcher;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use ts_rs::TS;

#[derive(Clone, Default, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct ExplorerPageRequest {
    pub root: String,
    pub path: String,
    pub filter: String,
    pub offset: usize,
    pub snapshot: Option<String>,
}
#[derive(Clone, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct ExplorerPageResponse {
    pub entries: Vec<FileEntry>,
    pub total: usize,
    pub snapshot: String,
    pub unreadable_paths: Vec<String>,
}
#[derive(Clone, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct ExplorerMutationRequest {
    pub root: String,
    pub operation: String,
    pub paths: Vec<String>,
    pub destination: Option<String>,
}
type Snapshot = (Vec<FileEntry>, Vec<String>);

#[cfg(test)]
mod tests {
    use super::*;
    use helix_fs::{FsConfig, testutil::TempDir};
    use helix_ipc::IpcRequest;
    use helix_log::{LogLevel, Logger};

    fn operation(
        dir: &TempDir,
        operation: &str,
        paths: &[&str],
        destination: Option<&str>,
    ) -> Result<(), AppError> {
        mutate(ExplorerMutationRequest {
            root: dir.path().to_string_lossy().into_owned(),
            operation: operation.into(),
            paths: paths
                .iter()
                .map(|p| dir.path().join(p).to_string_lossy().into_owned())
                .collect(),
            destination: destination.map(|p| dir.path().join(p).to_string_lossy().into_owned()),
        })
    }

    #[test]
    fn crud_and_multi_target_operations_preserve_contents() {
        let dir = TempDir::new("explorer-crud");
        operation(&dir, "newFolder", &[], Some("src")).unwrap();
        operation(&dir, "newFile", &[], Some("src/a.txt")).unwrap();
        dir.write("src/a.txt", "hello");
        operation(&dir, "rename", &["src/a.txt"], Some("src/b.txt")).unwrap();
        operation(&dir, "duplicate", &["src/b.txt"], None).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("src/b.txt copy 1")).unwrap(),
            "hello"
        );
        dir.mkdir("dest");
        operation(
            &dir,
            "copy",
            &["src/b.txt", "src/b.txt copy 1"],
            Some("dest"),
        )
        .unwrap();
        operation(&dir, "delete", &["src/b.txt", "src/b.txt copy 1"], None).unwrap();
        assert!(dir.path().join("dest/b.txt").exists());
        dir.mkdir("moved");
        operation(
            &dir,
            "move",
            &["dest/b.txt", "dest/b.txt copy 1"],
            Some("moved"),
        )
        .unwrap();
        assert!(!dir.path().join("dest/b.txt").exists());
        operation(&dir, "delete", &["moved", "moved/b.txt"], None).unwrap();
        assert!(!dir.path().join("moved").exists());
    }

    #[test]
    fn rejects_root_traversal_collisions_and_recursive_moves() {
        let dir = TempDir::new("explorer-guards");
        dir.write("a.txt", "a");
        dir.write("b.txt", "b");
        dir.mkdir("src/child");
        assert!(operation(&dir, "newFile", &[], Some("a.txt")).is_err());
        assert!(operation(&dir, "rename", &["a.txt"], Some("b.txt")).is_err());
        assert!(operation(&dir, "move", &["src"], Some("src/child")).is_err());
        assert!(operation(&dir, "delete", &[""], None).is_err());
        assert!(operation(&dir, "newFile", &[], Some("../escape")).is_err());
        assert_eq!(fs::read_to_string(dir.path().join("b.txt")).unwrap(), "b");
    }

    #[cfg(unix)]
    #[test]
    fn cannot_mutate_through_a_symlink_outside_the_root() {
        let dir = TempDir::new("explorer-link");
        let outside = TempDir::new("explorer-outside");
        outside.write("keep", "safe");
        std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
        assert!(operation(&dir, "delete", &["link/keep"], None).is_err());
        assert!(operation(&dir, "newFile", &[], Some("link/new")).is_err());
        operation(&dir, "delete", &["link"], None).unwrap();
        assert!(outside.path().join("keep").exists());
    }

    #[tokio::test]
    async fn serves_bounded_stable_pages_and_matching_ancestors_without_git() {
        let dir = TempDir::new("explorer-pages");
        for n in 0..1002 {
            dir.write(&format!("file-{n:04}.txt"), "");
        }
        dir.write("src/ui/component.tsx", "");
        dir.write("src/other.txt", "");
        let mut dispatcher = IpcDispatcher::new();
        register(
            &mut dispatcher,
            Arc::new(FileSystemService::new(
                FsConfig::default(),
                Arc::new(Logger::in_memory(LogLevel::Trace)),
            )),
        );
        let request = |offset, snapshot: Option<String>, filter: &str| serde_json::json!({ "root":dir.path(), "path":dir.path(), "offset":offset, "snapshot":snapshot, "filter":filter });
        let first = dispatcher
            .dispatch(IpcRequest::new("explorer.page", "p1", request(0, None, "")))
            .await
            .result
            .unwrap();
        assert_eq!(first["entries"].as_array().unwrap().len(), 500);
        let second = dispatcher
            .dispatch(IpcRequest::new(
                "explorer.page",
                "p2",
                request(500, Some(first["snapshot"].as_str().unwrap().into()), ""),
            ))
            .await
            .result
            .unwrap();
        assert_eq!(second["entries"].as_array().unwrap().len(), 500);
        assert_ne!(first["entries"][0]["path"], second["entries"][0]["path"]);
        let filtered = dispatcher
            .dispatch(IpcRequest::new(
                "explorer.page",
                "p3",
                request(0, None, "component"),
            ))
            .await
            .result
            .unwrap();
        assert_eq!(filtered["total"], 3);
    }
}

fn failure(message: impl ToString) -> AppError {
    AppError::permanent("EXPLORER_FAILED", message.to_string())
}

// Resolve the parent, not the leaf: deleting a symlink must delete the link,
// while symlinked ancestors must never permit an escape from the root.
fn scoped(root: &Path, path: &str, allow_root: bool) -> Result<PathBuf, AppError> {
    let path = Path::new(path);
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(failure("An absolute workspace path is required"));
    }
    if path == root && allow_root {
        return Ok(root.to_path_buf());
    }
    let parent = path
        .parent()
        .ok_or_else(|| failure("Missing parent"))?
        .canonicalize()
        .map_err(failure)?;
    let resolved = parent.join(path.file_name().ok_or_else(|| failure("Missing name"))?);
    if !resolved.starts_with(root) || (resolved == root && !allow_root) {
        return Err(failure(
            "The operation must stay inside the workspace and cannot change its root",
        ));
    }
    Ok(resolved)
}

fn copy_entry(source: &Path, target: &Path) -> Result<(), AppError> {
    let metadata = fs::symlink_metadata(source).map_err(failure)?;
    if metadata.is_symlink() {
        return Err(failure("Copying symbolic links is not supported"));
    }
    if metadata.is_dir() {
        fs::create_dir(target).map_err(failure)?;
        for entry in fs::read_dir(source).map_err(failure)? {
            let entry = entry.map_err(failure)?;
            copy_entry(&entry.path(), &target.join(entry.file_name()))?;
        }
    } else {
        let mut input = fs::File::open(source).map_err(failure)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)
            .map_err(failure)?;
        std::io::copy(&mut input, &mut output).map_err(failure)?;
        fs::set_permissions(target, metadata.permissions()).map_err(failure)?;
    }
    Ok(())
}

fn mutate(req: ExplorerMutationRequest) -> Result<(), AppError> {
    let root = Path::new(&req.root).canonicalize().map_err(failure)?;
    let mut paths = req
        .paths
        .iter()
        .map(|p| scoped(&root, p, req.operation == "reveal"))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    paths.dedup();
    let all = paths.clone();
    paths.retain(|p| !all.iter().any(|other| other != p && p.starts_with(other)));
    let destination = req
        .destination
        .as_deref()
        .map(|p| scoped(&root, p, true))
        .transpose()?;
    match req.operation.as_str() {
        "newFile" | "newFolder" => {
            let target = destination.ok_or_else(|| failure("Missing destination"))?;
            if target == root {
                return Err(failure("Cannot replace workspace root"));
            }
            if req.operation == "newFolder" {
                fs::create_dir(target).map_err(failure)?;
            } else {
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(target)
                    .map_err(failure)?;
            }
        }
        "delete" => {
            for path in paths {
                let metadata = fs::symlink_metadata(&path).map_err(failure)?;
                if metadata.is_dir() && !metadata.is_symlink() {
                    fs::remove_dir_all(path).map_err(failure)?;
                } else {
                    fs::remove_file(path).map_err(failure)?;
                }
            }
        }
        "rename" | "move" | "copy" | "duplicate" => {
            if req.operation == "rename" && paths.len() != 1 {
                return Err(failure("Rename needs one source"));
            }
            let mut pairs = Vec::new();
            for source in paths {
                let target = if req.operation == "duplicate" {
                    let name = source.file_name().unwrap().to_string_lossy();
                    let mut n = 1;
                    loop {
                        let candidate = source.with_file_name(format!("{name} copy {n}"));
                        if fs::symlink_metadata(&candidate).is_err() {
                            break candidate;
                        }
                        n += 1;
                    }
                } else {
                    let dest = destination
                        .as_ref()
                        .ok_or_else(|| failure("Missing destination"))?;
                    if req.operation == "rename" {
                        dest.clone()
                    } else {
                        if fs::symlink_metadata(dest).map_err(failure)?.is_symlink() {
                            return Err(failure("Cannot move into a symbolic link"));
                        }
                        dest.join(source.file_name().unwrap())
                    }
                };
                if target.starts_with(&source)
                    || fs::symlink_metadata(&target).is_ok()
                    || pairs.iter().any(|(_, t)| t == &target)
                {
                    return Err(failure(
                        "Destination already exists or is inside its source",
                    ));
                }
                pairs.push((source, target));
            }
            for (source, target) in pairs {
                if req.operation == "copy" || req.operation == "duplicate" {
                    copy_entry(&source, &target)?;
                } else {
                    fs::rename(source, target).map_err(failure)?;
                }
            }
        }
        "reveal" => {
            let path = paths.first().ok_or_else(|| failure("Missing path"))?;
            #[cfg(target_os = "macos")]
            let status = std::process::Command::new("open")
                .arg("-R")
                .arg(path)
                .status();
            #[cfg(target_os = "windows")]
            let status = std::process::Command::new("explorer.exe")
                .arg(format!("/select,{}", path.display()))
                .status();
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            let status = std::process::Command::new("xdg-open")
                .arg(path.parent().unwrap())
                .status();
            if !status.map_err(failure)?.success() {
                return Err(failure("File manager could not be opened"));
            }
        }
        _ => return Err(failure("Unknown explorer operation")),
    }
    Ok(())
}

pub fn register(dispatcher: &mut IpcDispatcher, service: Arc<FileSystemService>) {
    let cache = Arc::new(Mutex::new(HashMap::<String, Snapshot>::new()));
    let pages = cache.clone();
    dispatcher.register("explorer.page", move |req: ExplorerPageRequest, _ctx| {
        let service = service.clone();
        let cache = pages.clone();
        async move {
            tokio::task::spawn_blocking(move || {
                let root = Path::new(&req.root).canonicalize().map_err(failure)?;
                let path = scoped(&root, &req.path, true)?;
                if !path.canonicalize().map_err(failure)?.starts_with(&root) {
                    return Err(failure("Directory escapes workspace"));
                }
                let mut cache = cache.lock().unwrap();
                let scope = format!("{}:{}:{}:", root.display(), path.display(), req.filter);
                let key = req
                    .snapshot
                    .unwrap_or_else(|| format!("{scope}{:?}", std::time::SystemTime::now()));
                if !key.starts_with(&scope) {
                    return Err(failure("Snapshot belongs to another directory or filter"));
                }
                if !cache.contains_key(&key) {
                    if req.offset != 0 {
                        return Err(failure("Listing expired; refresh the explorer"));
                    }
                    let listing = service.list(&path, !req.filter.is_empty())?;
                    let mut entries = listing.entries;
                    if !req.filter.is_empty() {
                        let filter = req.filter.to_lowercase();
                        let matches: Vec<_> = entries
                            .iter()
                            .filter(|e| e.name.to_lowercase().contains(&filter))
                            .map(|e| PathBuf::from(&e.path))
                            .collect();
                        let mut keep = std::collections::HashSet::new();
                        for matched in matches {
                            for parent in matched.ancestors() {
                                if parent == path {
                                    break;
                                }
                                keep.insert(parent.to_path_buf());
                            }
                        }
                        entries.retain(|e| keep.contains(Path::new(&e.path)));
                    }
                    entries.sort_by(|a, b| {
                        b.is_dir
                            .cmp(&a.is_dir)
                            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                            .then_with(|| a.path.cmp(&b.path))
                    });
                    if cache.len() >= 32 {
                        cache.clear();
                    }
                    cache.insert(key.clone(), (entries, listing.unreadable_paths));
                }
                let (entries, unreadable) = &cache[&key];
                Ok(ExplorerPageResponse {
                    entries: entries.iter().skip(req.offset).take(500).cloned().collect(),
                    total: entries.len(),
                    snapshot: key,
                    unreadable_paths: unreadable.clone(),
                })
            })
            .await
            .map_err(failure)?
        }
    });
    dispatcher.register(
        "explorer.mutate",
        move |req: ExplorerMutationRequest, _ctx| {
            let cache = cache.clone();
            async move {
                let result = tokio::task::spawn_blocking(move || {
                    let mut snapshots = cache.lock().unwrap();
                    let result = mutate(req);
                    snapshots.clear();
                    result
                })
                .await
                .map_err(failure)?;
                result.map(|()| serde_json::json!({}))
            }
        },
    );
}
