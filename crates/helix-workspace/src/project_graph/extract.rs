use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use glob::glob;
use quick_xml::Reader;
use quick_xml::events::Event;
use serde_json::Value as JsonValue;
use toml::Value as TomlValue;
use xxhash_rust::xxh3::xxh3_64;

use crate::identity::{canonical_path, comparison_key};

use super::{
    MonorepoTool, Project, ProjectGraph, ProjectGraphStatus, SourceFingerprint, ToolDetection,
    detect_tools, fingerprint_sources,
};

/// A malformed or unreadable manifest that did not prevent another ecosystem
/// in the same workspace from producing a graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionWarning {
    pub path: PathBuf,
    pub message: String,
}

/// Graph extraction result plus non-fatal parser findings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectGraphExtraction {
    pub graph: ProjectGraph,
    /// Source identities captured with this result. The cache persists these
    /// exact hashes instead of pairing an older graph with later file content.
    pub fingerprints: Vec<SourceFingerprint>,
    pub warnings: Vec<ExtractionWarning>,
}

#[derive(Debug, Clone)]
enum DependencyRef {
    Name(String),
    Root(PathBuf),
}

#[derive(Debug, Clone)]
struct Candidate {
    name: String,
    root: PathBuf,
    dependencies: Vec<DependencyRef>,
    tool: MonorepoTool,
}

#[derive(Default)]
struct GraphBuilder {
    candidates: BTreeMap<String, Candidate>,
    aliases: HashMap<String, BTreeSet<String>>,
    source_files: BTreeSet<PathBuf>,
    warnings: Vec<ExtractionWarning>,
}

impl GraphBuilder {
    fn source(&mut self, path: impl Into<PathBuf>) {
        self.source_files.insert(path.into());
    }

    fn warn(&mut self, path: impl Into<PathBuf>, message: impl Into<String>) {
        self.warnings.push(ExtractionWarning {
            path: path.into(),
            message: message.into(),
        });
    }

    fn add(&mut self, candidate: Candidate) {
        let key = comparison_key(&candidate.root);
        self.aliases
            .entry(key.clone())
            .or_default()
            .insert(candidate.name.clone());
        match self.candidates.get_mut(&key) {
            Some(existing) => {
                let preferred = tool_priority(candidate.tool) < tool_priority(existing.tool);
                if preferred {
                    existing.tool = candidate.tool;
                }
                if preferred || existing.name.is_empty() {
                    existing.name = candidate.name;
                }
                existing.dependencies.extend(candidate.dependencies);
            }
            None => {
                self.candidates.insert(key, candidate);
            }
        }
    }

    fn finish(
        mut self,
        workspace_key: &str,
        roots: &[PathBuf],
        detection: &ToolDetection,
    ) -> ProjectGraphExtraction {
        self.source_files
            .extend(detection.source_files.iter().cloned());

        let extracted_projects = !self.candidates.is_empty();
        if self.candidates.is_empty() {
            for root in roots.iter().filter(|root| root.is_dir()) {
                self.add(Candidate {
                    name: file_name(root, "workspace"),
                    root: canonical_path(root),
                    dependencies: Vec::new(),
                    tool: MonorepoTool::Fallback,
                });
            }
        }

        let candidates: Vec<Candidate> = self.candidates.into_values().collect();
        let mut name_counts = HashMap::<String, usize>::new();
        for candidate in &candidates {
            *name_counts.entry(candidate.name.clone()).or_default() += 1;
        }

        let ids: Vec<String> = candidates
            .iter()
            .map(|candidate| {
                if name_counts.get(&candidate.name) == Some(&1) {
                    candidate.name.clone()
                } else {
                    format!(
                        "{}@{:08x}",
                        candidate.name,
                        xxh3_64(comparison_key(&candidate.root).as_bytes()) as u32
                    )
                }
            })
            .collect();

        let mut by_name = HashMap::<&str, Vec<&str>>::new();
        let mut by_root = HashMap::<String, &str>::new();
        for (candidate, id) in candidates.iter().zip(&ids) {
            let root_key = comparison_key(&candidate.root);
            if let Some(aliases) = self.aliases.get(&root_key) {
                for alias in aliases {
                    by_name.entry(alias).or_default().push(id.as_str());
                }
            }
            by_root.insert(root_key, id.as_str());
        }

        let mut projects: Vec<Project> = candidates
            .iter()
            .zip(&ids)
            .map(|(candidate, id)| {
                let mut dependencies: Vec<String> = candidate
                    .dependencies
                    .iter()
                    .filter_map(|dependency| match dependency {
                        DependencyRef::Name(name) => by_name
                            .get(name.as_str())
                            .filter(|matches| matches.len() == 1)
                            .and_then(|matches| matches.first())
                            .copied(),
                        DependencyRef::Root(root) => by_root.get(&comparison_key(root)).copied(),
                    })
                    .filter(|dependency| *dependency != id)
                    .map(str::to_string)
                    .collect();
                dependencies.sort();
                dependencies.dedup();
                Project {
                    id: id.clone(),
                    name: candidate.name.clone(),
                    root: canonical_path(&candidate.root)
                        .to_string_lossy()
                        .to_string(),
                    dependencies,
                    tool: candidate.tool,
                }
            })
            .collect();
        projects.sort_by(|left, right| left.id.cmp(&right.id));

        let mut tools: Vec<MonorepoTool> = projects.iter().map(|project| project.tool).collect();
        for detected in &detection.tools {
            tools.push(detected.tool);
        }
        tools.sort();
        tools.dedup();
        let status = if extracted_projects {
            ProjectGraphStatus::Fresh
        } else {
            ProjectGraphStatus::Fallback
        };

        let source_files: Vec<String> = self
            .source_files
            .into_iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect();
        let source_paths: Vec<PathBuf> = source_files.iter().map(PathBuf::from).collect();
        ProjectGraphExtraction {
            graph: ProjectGraph {
                workspace_key: workspace_key.to_string(),
                projects,
                tools,
                source_files,
                generated_ms: now_ms(),
                status,
            },
            fingerprints: fingerprint_sources(&source_paths),
            warnings: self.warnings,
        }
    }
}

/// Extract every detected graph and merge polyglot projects by canonical root.
pub fn extract_project_graph(workspace_key: &str, roots: &[PathBuf]) -> ProjectGraphExtraction {
    let canonical_roots: Vec<PathBuf> = roots.iter().map(|root| canonical_path(root)).collect();
    let detection = detect_tools(&canonical_roots);
    let mut builder = GraphBuilder::default();

    let mut node_roots = BTreeSet::new();
    for detected in &detection.tools {
        if is_node_tool(detected.tool) {
            node_roots.insert(detected.root.clone());
        }
    }
    for root in node_roots {
        extract_node(
            &root,
            preferred_node_tool(&detection, &root),
            &detection,
            &mut builder,
        );
    }

    for detected in &detection.tools {
        let result = match detected.tool {
            MonorepoTool::Cargo => extract_cargo(&detected.root, &mut builder),
            MonorepoTool::Go => extract_go(&detected.root, &mut builder),
            MonorepoTool::Maven => extract_maven(&detected.root, &mut builder),
            MonorepoTool::Gradle => extract_gradle(&detected.root, &mut builder),
            MonorepoTool::DotNet => extract_dotnet(&detected.root, &mut builder),
            _ => Ok(()),
        };
        if let Err(message) = result {
            builder.warn(
                detected
                    .config_files
                    .first()
                    .cloned()
                    .unwrap_or_else(|| detected.root.clone()),
                message,
            );
        }
    }

    builder.finish(workspace_key, &canonical_roots, &detection)
}

fn extract_node(
    root: &Path,
    tool: MonorepoTool,
    detection: &ToolDetection,
    builder: &mut GraphBuilder,
) {
    let mut patterns = node_workspace_patterns(root, builder);
    patterns.sort();
    patterns.dedup();
    let package_files = expand_manifest_patterns(root, &patterns, "package.json", builder);

    if detection
        .tools
        .iter()
        .any(|entry| entry.root == root && entry.tool == MonorepoTool::Nx)
    {
        for central in [root.join("workspace.json"), root.join("angular.json")]
            .into_iter()
            .filter(|path| path.is_file())
        {
            extract_nx_workspace(root, &central, builder);
        }
        for project in find_named_files(root, &["project.json"], 16) {
            extract_nx_project(&project, builder);
        }
    }

    for package in package_files {
        extract_package_json(&package, tool, builder);
    }
}

fn node_workspace_patterns(root: &Path, builder: &mut GraphBuilder) -> Vec<String> {
    let mut patterns = Vec::new();
    let package = root.join("package.json");
    if package.is_file() {
        builder.source(package.clone());
        match read_json(&package) {
            Ok(value) => match value.get("workspaces") {
                Some(JsonValue::Array(entries)) => extend_strings(&mut patterns, entries),
                Some(JsonValue::Object(object)) => {
                    if let Some(entries) = object.get("packages").and_then(JsonValue::as_array) {
                        extend_strings(&mut patterns, entries);
                    }
                }
                _ => {}
            },
            Err(message) => builder.warn(package, message),
        }
    }

    let lerna = root.join("lerna.json");
    if lerna.is_file() {
        builder.source(lerna.clone());
        match read_json(&lerna) {
            Ok(value) => {
                if let Some(entries) = value.get("packages").and_then(JsonValue::as_array) {
                    extend_strings(&mut patterns, entries);
                } else {
                    patterns.push("packages/*".to_string());
                }
            }
            Err(message) => builder.warn(lerna, message),
        }
    }

    let pnpm = root.join("pnpm-workspace.yaml");
    if pnpm.is_file() {
        builder.source(pnpm.clone());
        match fs::read_to_string(&pnpm) {
            Ok(body) => patterns.extend(parse_pnpm_patterns(&body)),
            Err(error) => builder.warn(pnpm, error.to_string()),
        }
    }
    patterns
}

fn extract_nx_workspace(root: &Path, path: &Path, builder: &mut GraphBuilder) {
    builder.source(path.to_path_buf());
    let value = match read_json(path) {
        Ok(value) => value,
        Err(message) => {
            builder.warn(path.to_path_buf(), message);
            return;
        }
    };
    let Some(projects) = value.get("projects").and_then(JsonValue::as_object) else {
        return;
    };
    for (name, project) in projects {
        let (project_root, dependencies) = match project {
            JsonValue::String(project_root) => (project_root.as_str(), Vec::new()),
            JsonValue::Object(project) => {
                let Some(project_root) = project.get("root").and_then(JsonValue::as_str) else {
                    continue;
                };
                let dependencies = project
                    .get("implicitDependencies")
                    .and_then(JsonValue::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(JsonValue::as_str)
                    .map(|dependency| DependencyRef::Name(dependency.to_string()))
                    .collect();
                (project_root, dependencies)
            }
            _ => continue,
        };
        builder.add(Candidate {
            name: name.clone(),
            root: canonical_path(&root.join(project_root)),
            dependencies,
            tool: MonorepoTool::Nx,
        });
    }
}

fn extract_package_json(path: &Path, tool: MonorepoTool, builder: &mut GraphBuilder) {
    builder.source(path.to_path_buf());
    let value = match read_json(path) {
        Ok(value) => value,
        Err(message) => {
            builder.warn(path.to_path_buf(), message);
            return;
        }
    };
    let root = path.parent().unwrap_or(path);
    let name = value
        .get("name")
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| file_name(root, "package"));
    let mut dependencies = Vec::new();
    for section in [
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
    ] {
        if let Some(entries) = value.get(section).and_then(JsonValue::as_object) {
            dependencies.extend(entries.keys().cloned().map(DependencyRef::Name));
        }
    }
    builder.add(Candidate {
        name,
        root: canonical_path(root),
        dependencies,
        tool,
    });
}

fn extract_nx_project(path: &Path, builder: &mut GraphBuilder) {
    builder.source(path.to_path_buf());
    let value = match read_json(path) {
        Ok(value) => value,
        Err(message) => {
            builder.warn(path.to_path_buf(), message);
            return;
        }
    };
    let root = path.parent().unwrap_or(path);
    let name = value
        .get("name")
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| file_name(root, "nx-project"));
    let dependencies = value
        .get("implicitDependencies")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .map(|name| DependencyRef::Name(name.to_string()))
        .collect();
    builder.add(Candidate {
        name,
        root: canonical_path(root),
        dependencies,
        tool: MonorepoTool::Nx,
    });
}

fn extract_cargo(root: &Path, builder: &mut GraphBuilder) -> Result<(), String> {
    let workspace_manifest = root.join("Cargo.toml");
    builder.source(workspace_manifest.clone());
    let value = read_toml(&workspace_manifest)?;
    let workspace = value
        .get("workspace")
        .and_then(TomlValue::as_table)
        .ok_or_else(|| "Cargo.toml has no [workspace] table".to_string())?;
    let patterns = toml_strings(workspace.get("members"));
    let excludes: HashSet<String> = expand_manifest_patterns(
        root,
        &toml_strings(workspace.get("exclude")),
        "Cargo.toml",
        builder,
    )
    .into_iter()
    .map(|path| comparison_key(path.parent().unwrap_or(&path)))
    .collect();
    let mut manifests = expand_manifest_patterns(root, &patterns, "Cargo.toml", builder);
    if value.get("package").is_some() {
        manifests.push(workspace_manifest);
    }
    manifests.sort();
    manifests.dedup();
    for manifest in manifests {
        let project_root = manifest.parent().unwrap_or(&manifest);
        if excludes.contains(&comparison_key(project_root)) {
            continue;
        }
        builder.source(manifest.clone());
        let value = match read_toml(&manifest) {
            Ok(value) => value,
            Err(message) => {
                builder.warn(manifest, message);
                continue;
            }
        };
        let Some(package) = value.get("package").and_then(TomlValue::as_table) else {
            continue;
        };
        let name = package
            .get("name")
            .and_then(TomlValue::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| file_name(project_root, "cargo-package"));
        let mut dependencies = Vec::new();
        collect_cargo_dependencies(&value, &mut dependencies);
        builder.add(Candidate {
            name,
            root: canonical_path(project_root),
            dependencies,
            tool: MonorepoTool::Cargo,
        });
    }
    Ok(())
}

fn collect_cargo_dependencies(value: &TomlValue, dependencies: &mut Vec<DependencyRef>) {
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(entries) = value.get(section).and_then(TomlValue::as_table) {
            for (name, spec) in entries {
                let package = spec
                    .as_table()
                    .and_then(|table| table.get("package"))
                    .and_then(TomlValue::as_str)
                    .unwrap_or(name);
                dependencies.push(DependencyRef::Name(package.to_string()));
            }
        }
    }
    if let Some(targets) = value.get("target").and_then(TomlValue::as_table) {
        for target in targets.values().filter_map(TomlValue::as_table) {
            for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
                if let Some(entries) = target.get(section).and_then(TomlValue::as_table) {
                    dependencies.extend(entries.keys().cloned().map(DependencyRef::Name));
                }
            }
        }
    }
}

fn extract_go(root: &Path, builder: &mut GraphBuilder) -> Result<(), String> {
    let work = root.join("go.work");
    builder.source(work.clone());
    let body = fs::read_to_string(&work).map_err(|error| error.to_string())?;
    for module_root in parse_go_work_uses(&body) {
        let module_root = canonical_path(&root.join(module_root));
        let manifest = module_root.join("go.mod");
        builder.source(manifest.clone());
        let body = match fs::read_to_string(&manifest) {
            Ok(body) => body,
            Err(error) => {
                builder.warn(manifest, error.to_string());
                continue;
            }
        };
        let (name, requirements) = parse_go_mod(&body);
        builder.add(Candidate {
            name: name.unwrap_or_else(|| file_name(&module_root, "go-module")),
            root: module_root,
            dependencies: requirements.into_iter().map(DependencyRef::Name).collect(),
            tool: MonorepoTool::Go,
        });
    }
    Ok(())
}

fn extract_maven(root: &Path, builder: &mut GraphBuilder) -> Result<(), String> {
    let root_pom = root.join("pom.xml");
    let mut pending = vec![root_pom];
    let mut seen = HashSet::new();
    while let Some(pom) = pending.pop() {
        let key = comparison_key(&pom);
        if !seen.insert(key) {
            continue;
        }
        builder.source(pom.clone());
        let model = match parse_maven_pom(&pom) {
            Ok(model) => model,
            Err(message) => {
                builder.warn(pom, message);
                continue;
            }
        };
        let project_root = pom.parent().unwrap_or(&pom);
        if let Some(name) = model.artifact_id {
            builder.add(Candidate {
                name,
                root: canonical_path(project_root),
                dependencies: model
                    .dependencies
                    .into_iter()
                    .map(DependencyRef::Name)
                    .collect(),
                tool: MonorepoTool::Maven,
            });
        }
        for module in model.modules {
            pending.push(project_root.join(module).join("pom.xml"));
        }
    }
    Ok(())
}

fn extract_gradle(root: &Path, builder: &mut GraphBuilder) -> Result<(), String> {
    let settings = [
        root.join("settings.gradle"),
        root.join("settings.gradle.kts"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .ok_or_else(|| "Gradle settings file is missing".to_string())?;
    builder.source(settings.clone());
    let body = fs::read_to_string(&settings).map_err(|error| error.to_string())?;
    let project_directories = parse_gradle_project_directories(&body);
    for project_path in parse_gradle_includes(&body) {
        let name = project_path.trim_start_matches(':').to_string();
        let relative = project_directories
            .get(&name)
            .cloned()
            .unwrap_or_else(|| name.replace(':', "/"));
        let project_root = canonical_path(&root.join(relative));
        let mut dependencies = Vec::new();
        for build in [
            project_root.join("build.gradle"),
            project_root.join("build.gradle.kts"),
        ] {
            if !build.is_file() {
                continue;
            }
            builder.source(build.clone());
            if let Ok(body) = fs::read_to_string(&build) {
                dependencies.extend(
                    parse_gradle_project_dependencies(&body)
                        .into_iter()
                        .map(DependencyRef::Name),
                );
            }
        }
        builder.add(Candidate {
            name,
            root: project_root,
            dependencies,
            tool: MonorepoTool::Gradle,
        });
    }
    Ok(())
}

fn extract_dotnet(root: &Path, builder: &mut GraphBuilder) -> Result<(), String> {
    let solutions: Vec<PathBuf> = fs::read_dir(root)
        .map_err(|error| error.to_string())?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("sln"))
        })
        .collect();
    for solution in solutions {
        builder.source(solution.clone());
        let body = fs::read_to_string(&solution).map_err(|error| error.to_string())?;
        for (name, project_file) in parse_solution_projects(&body) {
            let project_file = canonical_path(&root.join(project_file.replace('\\', "/")));
            builder.source(project_file.clone());
            let dependencies = parse_project_references(&project_file)
                .unwrap_or_else(|message| {
                    builder.warn(project_file.clone(), message);
                    Vec::new()
                })
                .into_iter()
                .map(|reference| {
                    DependencyRef::Root(reference.parent().map(canonical_path).unwrap_or(reference))
                })
                .collect();
            builder.add(Candidate {
                name,
                root: project_file
                    .parent()
                    .map(canonical_path)
                    .unwrap_or_else(|| canonical_path(root)),
                dependencies,
                tool: MonorepoTool::DotNet,
            });
        }
    }
    Ok(())
}

fn expand_manifest_patterns(
    root: &Path,
    patterns: &[String],
    manifest: &str,
    builder: &mut GraphBuilder,
) -> Vec<PathBuf> {
    let positive: Vec<&String> = patterns
        .iter()
        .filter(|pattern| !pattern.starts_with('!'))
        .collect();
    let excluded = expand_patterns(
        root,
        patterns
            .iter()
            .filter_map(|pattern| pattern.strip_prefix('!')),
        manifest,
        builder,
    );
    expand_patterns(
        root,
        positive.into_iter().map(String::as_str),
        manifest,
        builder,
    )
    .into_iter()
    .filter(|path| !excluded.contains(path))
    .collect()
}

fn expand_patterns<'a>(
    root: &Path,
    patterns: impl IntoIterator<Item = &'a str>,
    manifest: &str,
    builder: &mut GraphBuilder,
) -> BTreeSet<PathBuf> {
    let mut matches = BTreeSet::new();
    for pattern in patterns {
        let mut path = root.join(pattern);
        if path.file_name().and_then(|name| name.to_str()) != Some(manifest) {
            path.push(manifest);
        }
        let pattern = path.to_string_lossy();
        match glob(&pattern) {
            Ok(paths) => {
                for path in paths.flatten().filter(|path| path.is_file()) {
                    if !path.components().any(|part| {
                        matches!(
                            part.as_os_str().to_str(),
                            Some("node_modules" | ".git" | "target")
                        )
                    }) {
                        matches.insert(canonical_path(&path));
                    }
                }
            }
            Err(error) => builder.warn(path, error.to_string()),
        }
    }
    matches
}

fn find_named_files(root: &Path, names: &[&str], max_depth: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    while let Some((directory, depth)) = pending.pop() {
        if depth > max_depth {
            continue;
        }
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| names.contains(&name))
            {
                found.push(path);
            } else if path.is_dir()
                && !matches!(
                    path.file_name().and_then(|name| name.to_str()),
                    Some("node_modules" | ".git" | "target" | "build" | ".gradle")
                )
            {
                pending.push((path, depth + 1));
            }
        }
    }
    found.sort();
    found
}

fn parse_pnpm_patterns(body: &str) -> Vec<String> {
    let mut patterns = Vec::new();
    let mut in_packages = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed == "packages:" {
            in_packages = true;
            continue;
        }
        if in_packages && !line.starts_with([' ', '\t']) && !trimmed.is_empty() {
            break;
        }
        if in_packages && let Some(value) = trimmed.strip_prefix('-') {
            patterns.push(unquote(value.trim()).to_string());
        }
    }
    patterns
}

fn parse_go_work_uses(body: &str) -> Vec<PathBuf> {
    let mut uses = Vec::new();
    let mut block = false;
    for line in body.lines() {
        let line = line.split("//").next().unwrap_or_default().trim();
        if line == "use (" {
            block = true;
        } else if block && line == ")" {
            block = false;
        } else if block && !line.is_empty() {
            uses.push(PathBuf::from(
                line.split_whitespace().next().unwrap_or(line),
            ));
        } else if let Some(path) = line.strip_prefix("use ") {
            uses.push(PathBuf::from(
                path.split_whitespace().next().unwrap_or(path),
            ));
        }
    }
    uses
}

fn parse_go_mod(body: &str) -> (Option<String>, Vec<String>) {
    let mut module = None;
    let mut requirements = Vec::new();
    let mut block = false;
    for line in body.lines() {
        let line = line.split("//").next().unwrap_or_default().trim();
        if let Some(name) = line.strip_prefix("module ") {
            module = Some(name.trim().to_string());
        } else if line == "require (" {
            block = true;
        } else if block && line == ")" {
            block = false;
        } else if block && !line.is_empty() {
            if let Some(name) = line.split_whitespace().next() {
                requirements.push(name.to_string());
            }
        } else if let Some(requirement) = line.strip_prefix("require ")
            && let Some(name) = requirement.split_whitespace().next()
        {
            requirements.push(name.to_string());
        }
    }
    (module, requirements)
}

#[derive(Default)]
struct MavenModel {
    artifact_id: Option<String>,
    modules: Vec<String>,
    dependencies: Vec<String>,
}

fn parse_maven_pom(path: &Path) -> Result<MavenModel, String> {
    let body = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let mut reader = Reader::from_str(&body);
    reader.config_mut().trim_text(true);
    let mut stack = Vec::<String>::new();
    let mut model = MavenModel::default();
    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => {
                stack.push(String::from_utf8_lossy(start.name().as_ref()).to_string());
            }
            Ok(Event::End(_)) => {
                stack.pop();
            }
            Ok(Event::Text(text)) => {
                let value = text
                    .decode()
                    .map_err(|error| error.to_string())?
                    .into_owned();
                let suffix: Vec<&str> = stack.iter().map(String::as_str).collect();
                if suffix.as_slice() == ["project", "artifactId"] {
                    model.artifact_id = Some(value);
                } else if suffix.ends_with(&["modules", "module"]) {
                    model.modules.push(value);
                } else if suffix.ends_with(&["dependencies", "dependency", "artifactId"]) {
                    model.dependencies.push(value);
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(error.to_string()),
            _ => {}
        }
    }
    Ok(model)
}

fn parse_gradle_includes(body: &str) -> Vec<String> {
    let mut projects = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if line.starts_with("include ") || line.starts_with("include(") {
            projects.extend(quoted_values(line));
        }
    }
    projects.sort();
    projects.dedup();
    projects
}

fn parse_gradle_project_directories(body: &str) -> HashMap<String, String> {
    let mut directories = HashMap::new();
    for line in body.lines().filter(|line| line.contains("projectDir")) {
        let Some(project_start) = line.find("project(") else {
            continue;
        };
        let projects = quoted_values(&line[project_start..]);
        let Some(project) = projects.first() else {
            continue;
        };
        let Some(directory) = projects.get(1).or_else(|| projects.last()) else {
            continue;
        };
        if directory == project {
            continue;
        }
        directories.insert(
            project.trim_start_matches(':').to_string(),
            directory.to_string(),
        );
    }
    directories
}

fn parse_gradle_project_dependencies(body: &str) -> Vec<String> {
    let mut dependencies = Vec::new();
    for line in body.lines().filter(|line| line.contains("project(")) {
        dependencies.extend(
            quoted_values(line)
                .into_iter()
                .map(|value| value.trim_start_matches(':').to_string()),
        );
    }
    dependencies
}

fn quoted_values(body: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut quote = None;
    let mut start = 0usize;
    for (index, character) in body.char_indices() {
        match quote {
            None if character == '\'' || character == '"' => {
                quote = Some(character);
                start = index + character.len_utf8();
            }
            Some(open) if character == open => {
                values.push(body[start..index].to_string());
                quote = None;
            }
            _ => {}
        }
    }
    values
}

fn parse_solution_projects(body: &str) -> Vec<(String, String)> {
    body.lines()
        .filter_map(|line| {
            let (_, rest) = line.split_once(" = ")?;
            let mut fields = rest.split(',').map(|field| field.trim().trim_matches('"'));
            let name = fields.next()?;
            let path = fields.next()?;
            let extension = Path::new(path).extension()?.to_str()?;
            if !matches!(
                extension.to_ascii_lowercase().as_str(),
                "csproj" | "fsproj" | "vbproj"
            ) {
                return None;
            }
            Some((name.to_string(), path.to_string()))
        })
        .collect()
}

fn parse_project_references(path: &Path) -> Result<Vec<PathBuf>, String> {
    let body = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let mut reader = Reader::from_str(&body);
    let base = path.parent().unwrap_or(path);
    let mut references = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) | Ok(Event::Empty(start))
                if start.name().as_ref() == b"ProjectReference" =>
            {
                for attribute in start.attributes().flatten() {
                    if attribute.key.as_ref() == b"Include" {
                        let relative =
                            String::from_utf8_lossy(attribute.value.as_ref()).replace('\\', "/");
                        references.push(canonical_path(&base.join(relative)));
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(error.to_string()),
            _ => {}
        }
    }
    Ok(references)
}

fn read_json(path: &Path) -> Result<JsonValue, String> {
    let body = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&body).map_err(|error| error.to_string())
}

fn read_toml(path: &Path) -> Result<TomlValue, String> {
    let body = fs::read_to_string(path).map_err(|error| error.to_string())?;
    toml::from_str(&body).map_err(|error| error.to_string())
}

fn extend_strings(target: &mut Vec<String>, entries: &[JsonValue]) {
    target.extend(
        entries
            .iter()
            .filter_map(JsonValue::as_str)
            .map(str::to_string),
    );
}

fn toml_strings(value: Option<&TomlValue>) -> Vec<String> {
    value
        .and_then(TomlValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(TomlValue::as_str)
        .map(str::to_string)
        .collect()
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix(['\'', '"'])
        .and_then(|value| value.strip_suffix(['\'', '"']))
        .unwrap_or(value)
}

fn file_name(path: &Path, fallback: &str) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn preferred_node_tool(detection: &ToolDetection, root: &Path) -> MonorepoTool {
    detection
        .tools
        .iter()
        .filter(|entry| entry.root == root && is_node_tool(entry.tool))
        .map(|entry| entry.tool)
        .min_by_key(|tool| tool_priority(*tool))
        .unwrap_or(MonorepoTool::NpmWorkspaces)
}

fn is_node_tool(tool: MonorepoTool) -> bool {
    matches!(
        tool,
        MonorepoTool::Nx
            | MonorepoTool::Turborepo
            | MonorepoTool::Lerna
            | MonorepoTool::PnpmWorkspaces
            | MonorepoTool::NpmWorkspaces
            | MonorepoTool::YarnWorkspaces
    )
}

fn tool_priority(tool: MonorepoTool) -> u8 {
    match tool {
        MonorepoTool::Nx => 0,
        MonorepoTool::Turborepo => 1,
        MonorepoTool::Lerna => 2,
        MonorepoTool::PnpmWorkspaces => 3,
        MonorepoTool::YarnWorkspaces => 4,
        MonorepoTool::NpmWorkspaces => 5,
        MonorepoTool::Cargo => 6,
        MonorepoTool::Go => 7,
        MonorepoTool::Maven => 8,
        MonorepoTool::Gradle => 9,
        MonorepoTool::DotNet => 10,
        MonorepoTool::Fallback => 11,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_fs::testutil::TempDir;

    #[test]
    fn extracts_node_projects_and_dependencies() {
        let dir = TempDir::new("project-graph-node");
        dir.write("package.json", r#"{"workspaces":["packages/*"]}"#);
        dir.write(
            "packages/app/package.json",
            r#"{"name":"app","dependencies":{"ui":"workspace:*"}}"#,
        );
        dir.write("packages/ui/package.json", r#"{"name":"ui"}"#);

        let extracted = extract_project_graph("node", &[dir.path().into()]);
        assert_eq!(extracted.graph.projects.len(), 2);
        assert_eq!(
            extracted.graph.project("app").unwrap().dependencies,
            vec!["ui"]
        );
    }

    #[test]
    fn extracts_nx_central_projects_and_lerna_default_packages() {
        let nx = TempDir::new("project-graph-nx-central");
        nx.write("nx.json", "{}");
        nx.write(
            "workspace.json",
            r#"{"projects":{"app":{"root":"apps/app","implicitDependencies":["ui"]},"ui":"libs/ui"}}"#,
        );
        nx.write("apps/app/package.json", r#"{"name":"@repo/application"}"#);
        nx.write("libs/ui/package.json", r#"{"name":"@repo/ui"}"#);
        nx.write("package.json", r#"{"workspaces":["apps/*","libs/*"]}"#);
        let graph = extract_project_graph("nx", &[nx.path().into()]).graph;
        assert_eq!(graph.project("app").unwrap().dependencies, vec!["ui"]);
        assert!(graph.project("ui").unwrap().root.ends_with("libs/ui"));

        let lerna = TempDir::new("project-graph-lerna-default");
        lerna.write("lerna.json", "{}");
        lerna.write("packages/app/package.json", r#"{"name":"app"}"#);
        let graph = extract_project_graph("lerna", &[lerna.path().into()]).graph;
        assert!(graph.project("app").is_some());
    }

    #[test]
    fn gradle_project_directory_remapping_changes_the_owned_root() {
        let dir = TempDir::new("project-graph-gradle-remap");
        dir.write(
            "settings.gradle.kts",
            "include(\":app\")\nproject(\":app\").projectDir = file(\"modules/application\")\n",
        );
        dir.write("modules/application/build.gradle.kts", "");
        let graph = extract_project_graph("gradle", &[dir.path().into()]).graph;
        assert!(
            graph
                .project("app")
                .unwrap()
                .root
                .ends_with("modules/application")
        );
    }

    #[test]
    fn extracts_cargo_go_maven_gradle_and_dotnet_edges() {
        let dir = TempDir::new("project-graph-polyglot");
        dir.write("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n");
        dir.write("crates/api/Cargo.toml", "[package]\nname = \"api\"\nversion = \"0.1.0\"\n[dependencies]\nshared = { path = \"../shared\" }\n");
        dir.write(
            "crates/shared/Cargo.toml",
            "[package]\nname = \"shared\"\nversion = \"0.1.0\"\n",
        );
        dir.write(
            "go.work",
            "go 1.24\nuse (\n ./go/service\n ./go/common\n)\n",
        );
        dir.write(
            "go/service/go.mod",
            "module example/service\nrequire example/common v0.0.0\n",
        );
        dir.write("go/common/go.mod", "module example/common\n");
        dir.write("pom.xml", "<project><artifactId>parent</artifactId><modules><module>java/app</module><module>java/lib</module></modules></project>");
        dir.write("java/app/pom.xml", "<project><artifactId>java-app</artifactId><dependencies><dependency><artifactId>java-lib</artifactId></dependency></dependencies></project>");
        dir.write(
            "java/lib/pom.xml",
            "<project><artifactId>java-lib</artifactId></project>",
        );
        dir.write("settings.gradle", "include ':gradle-app', ':gradle-lib'\n");
        dir.write(
            "gradle-app/build.gradle",
            "implementation project(':gradle-lib')\n",
        );
        dir.write("gradle-lib/build.gradle", "");
        dir.write("company.sln", "Project(\"{kind}\") = \"dotnet-app\", \"dotnet/app/app.csproj\", \"{app}\"\nProject(\"{kind}\") = \"dotnet-lib\", \"dotnet/lib/lib.csproj\", \"{lib}\"\n");
        dir.write("dotnet/app/app.csproj", "<Project><ItemGroup><ProjectReference Include=\"../lib/lib.csproj\" /></ItemGroup></Project>");
        dir.write("dotnet/lib/lib.csproj", "<Project />");

        let graph = extract_project_graph("polyglot", &[dir.path().into()]).graph;
        for (project, dependency) in [
            ("api", "shared"),
            ("example/service", "example/common"),
            ("java-app", "java-lib"),
            ("gradle-app", "gradle-lib"),
            ("dotnet-app", "dotnet-lib"),
        ] {
            assert_eq!(
                graph.project(project).unwrap().dependencies,
                vec![dependency],
                "{project}"
            );
        }
    }

    #[test]
    fn missing_or_malformed_tools_degrade_to_workspace_roots() {
        let dir = TempDir::new("project-graph-fallback");
        let second = dir.mkdir("second-root");
        let graph = extract_project_graph("fallback", &[dir.path().into(), second]).graph;
        assert_eq!(graph.status, ProjectGraphStatus::Fallback);
        assert_eq!(graph.projects.len(), 2);
        assert!(
            graph
                .projects
                .iter()
                .all(|project| project.tool == MonorepoTool::Fallback)
        );

        dir.write("Cargo.toml", "[workspace]\nmembers = [not valid toml\n");
        let extracted = extract_project_graph("malformed", &[dir.path().into()]);
        assert_eq!(extracted.graph.status, ProjectGraphStatus::Fallback);
        assert!(!extracted.warnings.is_empty());
    }
}
