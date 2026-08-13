use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::MonorepoTool;

/// One tool detected at one workspace root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedTool {
    pub tool: MonorepoTool,
    pub root: PathBuf,
    pub config_files: Vec<PathBuf>,
}

/// Deterministic detection result across every available workspace root.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolDetection {
    pub tools: Vec<DetectedTool>,
    /// Existing configuration files and lockfiles whose content fingerprints
    /// determine whether a cached graph is current.
    pub source_files: Vec<PathBuf>,
}

impl ToolDetection {
    pub fn has(&self, tool: MonorepoTool) -> bool {
        self.tools.iter().any(|detected| detected.tool == tool)
    }
}

/// Detect every supported monorepo family. A root may intentionally contribute
/// more than one graph, for example a repository with Cargo and pnpm projects.
pub fn detect_tools(roots: &[PathBuf]) -> ToolDetection {
    let mut detection = ToolDetection::default();
    for root in roots.iter().filter(|root| root.is_dir()) {
        detect_root(root, &mut detection);
    }
    detection.tools.sort_by(|left, right| {
        left.root
            .cmp(&right.root)
            .then_with(|| left.tool.cmp(&right.tool))
    });
    detection.source_files.sort();
    detection.source_files.dedup();
    detection
}

fn detect_root(root: &Path, detection: &mut ToolDetection) {
    let nx_configs: Vec<PathBuf> = ["nx.json", "workspace.json", "angular.json"]
        .into_iter()
        .map(|name| root.join(name))
        .filter(|path| path.is_file())
        .collect();
    if !nx_configs.is_empty() {
        push_tool(detection, MonorepoTool::Nx, root, nx_configs);
    }

    let turbo = root.join("turbo.json");
    if turbo.is_file() {
        push_tool(detection, MonorepoTool::Turborepo, root, vec![turbo]);
    }

    let lerna = root.join("lerna.json");
    if lerna.is_file() {
        push_tool(detection, MonorepoTool::Lerna, root, vec![lerna]);
    }

    let pnpm = root.join("pnpm-workspace.yaml");
    if pnpm.is_file() {
        push_tool(detection, MonorepoTool::PnpmWorkspaces, root, vec![pnpm]);
    }

    let package = root.join("package.json");
    if package_has_workspaces(&package) {
        let tool = if root.join("pnpm-lock.yaml").is_file() {
            MonorepoTool::PnpmWorkspaces
        } else if root.join("yarn.lock").is_file() {
            MonorepoTool::YarnWorkspaces
        } else {
            MonorepoTool::NpmWorkspaces
        };
        if !detection
            .tools
            .iter()
            .any(|entry| entry.root == root && entry.tool == tool)
        {
            push_tool(detection, tool, root, vec![package.clone()]);
        } else {
            detection.source_files.push(package.clone());
        }
    }

    let cargo = root.join("Cargo.toml");
    if cargo_has_workspace(&cargo) {
        push_tool(detection, MonorepoTool::Cargo, root, vec![cargo]);
    }

    let go = root.join("go.work");
    if go.is_file() {
        push_tool(detection, MonorepoTool::Go, root, vec![go]);
    }

    let pom = root.join("pom.xml");
    if maven_has_modules(&pom) {
        push_tool(detection, MonorepoTool::Maven, root, vec![pom]);
    }

    let gradle = [
        root.join("settings.gradle"),
        root.join("settings.gradle.kts"),
    ]
    .into_iter()
    .find(|path| path.is_file());
    if let Some(gradle) = gradle {
        push_tool(detection, MonorepoTool::Gradle, root, vec![gradle]);
    }

    let mut solutions = read_files(root, |path| {
        path.extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("sln"))
    });
    if !solutions.is_empty() {
        solutions.sort();
        push_tool(detection, MonorepoTool::DotNet, root, solutions);
    }

    for name in [
        "package-lock.json",
        "npm-shrinkwrap.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "bun.lock",
        "bun.lockb",
        "Cargo.lock",
        "go.work.sum",
        "go.sum",
        "gradle.lockfile",
        "packages.lock.json",
    ] {
        let path = root.join(name);
        if path.is_file() {
            detection.source_files.push(path);
        }
    }
}

fn push_tool(
    detection: &mut ToolDetection,
    tool: MonorepoTool,
    root: &Path,
    config_files: Vec<PathBuf>,
) {
    detection.source_files.extend(config_files.iter().cloned());
    detection.tools.push(DetectedTool {
        tool,
        root: root.to_path_buf(),
        config_files,
    });
}

fn package_has_workspaces(path: &Path) -> bool {
    let Ok(body) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&body) else {
        return false;
    };
    match value.get("workspaces") {
        Some(Value::Array(entries)) => !entries.is_empty(),
        Some(Value::Object(object)) => object
            .get("packages")
            .and_then(Value::as_array)
            .is_some_and(|entries| !entries.is_empty()),
        _ => false,
    }
}

fn cargo_has_workspace(path: &Path) -> bool {
    fs::read_to_string(path)
        .ok()
        .is_some_and(|body| body.lines().any(|line| line.trim() == "[workspace]"))
}

fn maven_has_modules(path: &Path) -> bool {
    fs::read_to_string(path)
        .ok()
        .is_some_and(|body| body.contains("<modules") && body.contains("<module>"))
}

fn read_files(root: &Path, predicate: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && predicate(path))
        .collect()
}

/// Whether a watcher event can invalidate a monorepo graph. The current graph
/// fingerprint makes the final freshness decision; this predicate prevents
/// unrelated file events from scheduling extraction work.
pub fn is_graph_source_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    matches!(
        name,
        "nx.json"
            | "workspace.json"
            | "angular.json"
            | "project.json"
            | "turbo.json"
            | "lerna.json"
            | "pnpm-workspace.yaml"
            | "package.json"
            | "package-lock.json"
            | "npm-shrinkwrap.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "bun.lock"
            | "bun.lockb"
            | "Cargo.toml"
            | "Cargo.lock"
            | "go.work"
            | "go.work.sum"
            | "go.mod"
            | "go.sum"
            | "pom.xml"
            | "settings.gradle"
            | "settings.gradle.kts"
            | "build.gradle"
            | "build.gradle.kts"
            | "gradle.lockfile"
            | "packages.lock.json"
            | "Directory.Packages.props"
            | "Directory.Build.props"
            | "Directory.Build.targets"
    ) || path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("sln")
                || extension.eq_ignore_ascii_case("csproj")
                || extension.eq_ignore_ascii_case("fsproj")
                || extension.eq_ignore_ascii_case("vbproj")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_fs::testutil::TempDir;

    #[test]
    fn detects_every_required_tool_family() {
        let dir = TempDir::new("project-tool-detection");
        dir.write("nx.json", "{}");
        dir.write("turbo.json", "{}");
        dir.write("lerna.json", r#"{"packages":["packages/*"]}"#);
        dir.write("pnpm-workspace.yaml", "packages:\n  - packages/*\n");
        dir.write("package.json", r#"{"workspaces":["packages/*"]}"#);
        dir.write("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n");
        dir.write("go.work", "go 1.24\nuse ./services/api\n");
        dir.write(
            "pom.xml",
            "<project><modules><module>api</module></modules></project>",
        );
        dir.write("settings.gradle.kts", "include(\":app\")\n");
        dir.write("company.sln", "Microsoft Visual Studio Solution File\n");

        let detected = detect_tools(&[dir.path().to_path_buf()]);
        for tool in [
            MonorepoTool::Nx,
            MonorepoTool::Turborepo,
            MonorepoTool::Lerna,
            MonorepoTool::PnpmWorkspaces,
            MonorepoTool::Cargo,
            MonorepoTool::Go,
            MonorepoTool::Maven,
            MonorepoTool::Gradle,
            MonorepoTool::DotNet,
        ] {
            assert!(detected.has(tool), "missing {tool:?}");
        }
    }

    #[test]
    fn distinguishes_npm_and_yarn_workspaces() {
        let npm = TempDir::new("project-tool-npm");
        npm.write("package.json", r#"{"workspaces":["packages/*"]}"#);
        assert!(detect_tools(&[npm.path().into()]).has(MonorepoTool::NpmWorkspaces));

        let yarn = TempDir::new("project-tool-yarn");
        yarn.write(
            "package.json",
            r#"{"workspaces":{"packages":["packages/*"]}}"#,
        );
        yarn.write("yarn.lock", "");
        assert!(detect_tools(&[yarn.path().into()]).has(MonorepoTool::YarnWorkspaces));
    }

    #[test]
    fn detects_legacy_nx_central_configuration_without_nx_json() {
        for config in ["workspace.json", "angular.json"] {
            let dir = TempDir::new("project-tool-legacy-nx");
            dir.write(config, r#"{"projects":{}}"#);
            assert!(detect_tools(&[dir.path().into()]).has(MonorepoTool::Nx));
        }
    }

    #[test]
    fn graph_sources_include_manifests_and_lockfiles() {
        for path in [
            "project.json",
            "Cargo.lock",
            "go.mod",
            "settings.gradle",
            "api.csproj",
            "workspace.sln",
        ] {
            assert!(is_graph_source_file(Path::new(path)), "{path}");
        }
        assert!(!is_graph_source_file(Path::new("src/main.rs")));
    }
}
