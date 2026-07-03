//! Conservative scoping for package metadata changes.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::dependencies;
use crate::failure;
use crate::roots;
use crate::vcs;
use crate::vcs::ChangeSetView;

const DEPENDENCY_SECTIONS: &[&str] = &[
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "optionalDependencies",
];
const PACKAGE_JSON: &str = "package.json";
const BUN_LOCK: &str = "bun.lock";
const BUN_LOCKB: &str = "bun.lockb";

/// Request for replacing package metadata changes with scoped importer changes.
pub struct ScopeRequest<'a, R, G> {
    /// Git repository used to read metadata at base and head.
    pub repository: &'a R,
    /// Base revision for metadata comparison.
    pub base: &'a str,
    /// Head revision for metadata comparison.
    pub head: &'a str,
    /// Original Git change set.
    pub changes: &'a vcs::ChangeSet,
    /// Dependency graph containing external-package importers.
    pub graph: &'a G,
}

/// Returns a change set where safely scoped package metadata changes become
/// importer-file changes. Unscoped metadata changes are kept as-is so normal
/// global invalidator behavior remains fail-closed.
pub fn scoped_changes<R, G>(request: &ScopeRequest<'_, R, G>) -> failure::Result<vcs::ChangeSet>
where
    R: vcs::GitRepository,
    G: dependencies::GraphView,
{
    let mut files = Vec::<vcs::ChangedFile>::new();

    for change in request.changes.files() {
        if !is_package_metadata_path(&change.path) {
            files.push(change.clone());
            continue;
        }

        match scoped_package_importers(&PackageImportersRequest {
            repository: request.repository,
            base: request.base,
            head: request.head,
            change,
            graph: request.graph,
        })? {
            Some(importers) => files.extend(importers.into_vec().into_iter().map(importer_change)),
            None => files.push(change.clone()),
        }
    }

    files.sort_by(compare_changed_files);
    files.dedup_by(|left, right| {
        left.path == right.path && left.previous_path == right.previous_path
    });
    Ok(vcs::ChangeSet {
        files: files.into_boxed_slice(),
    })
}

struct PackageImportersRequest<'a, R, G> {
    repository: &'a R,
    base: &'a str,
    head: &'a str,
    change: &'a vcs::ChangedFile,
    graph: &'a G,
}

fn scoped_package_importers<R, G>(
    request: &PackageImportersRequest<'_, R, G>,
) -> failure::Result<Option<Box<[roots::RootRelativePath]>>>
where
    R: vcs::GitRepository,
    G: dependencies::GraphView,
{
    if request.change.status != vcs::ChangedFileStatus::Modified {
        return Ok(None);
    }

    let old_text = request
        .repository
        .file_at_revision(request.base, &request.change.path)?;
    let new_text = request
        .repository
        .file_at_revision(request.head, &request.change.path)?;
    let (Some(old_text), Some(new_text)) = (old_text, new_text) else {
        return Ok(None);
    };

    let changed_packages = match request.change.path.as_str() {
        PACKAGE_JSON => changed_package_json_packages(old_text.as_ref(), new_text.as_ref()),
        BUN_LOCK => changed_bun_lock_packages(old_text.as_ref(), new_text.as_ref()),
        _ => None,
    };
    let Some(changed_packages) = changed_packages else {
        return Ok(None);
    };
    if changed_packages.is_empty() {
        return Ok(None);
    }
    if changed_packages
        .iter()
        .any(|package| is_global_risk_package(package))
    {
        return Ok(None);
    }

    let importers = importers_for_packages(request.graph, &changed_packages);
    if importers.is_empty() {
        // The dependency changed but no known file imports it at runtime (for
        // example it is only reachable through a type-only import). Keep the
        // metadata change so it stays a fail-closed global invalidator rather
        // than silently selecting nothing.
        return Ok(None);
    }

    Ok(Some(importers))
}

fn changed_package_json_packages(old_text: &str, new_text: &str) -> Option<BTreeSet<Box<str>>> {
    let old_json = serde_json::from_str::<Value>(old_text).ok()?;
    let new_json = serde_json::from_str::<Value>(new_text).ok()?;
    let old_object = old_json.as_object()?;
    let new_object = new_json.as_object()?;

    for key in old_object.keys().chain(new_object.keys()) {
        if DEPENDENCY_SECTIONS.contains(&key.as_str()) {
            continue;
        }
        if old_object.get(key) != new_object.get(key) {
            return None;
        }
    }

    let mut changed = BTreeSet::<Box<str>>::new();
    for section in DEPENDENCY_SECTIONS {
        changed.extend(changed_dependency_section_packages(
            old_object.get(*section),
            new_object.get(*section),
        )?);
    }

    Some(changed)
}

fn changed_dependency_section_packages(
    old_section: Option<&Value>,
    new_section: Option<&Value>,
) -> Option<BTreeSet<Box<str>>> {
    let empty = serde_json::Map::<String, Value>::new();
    let old_dependencies = dependency_object(old_section, &empty)?;
    let new_dependencies = dependency_object(new_section, &empty)?;
    let mut changed = BTreeSet::<Box<str>>::new();

    for package in old_dependencies.keys().chain(new_dependencies.keys()) {
        if old_dependencies.get(package) != new_dependencies.get(package) {
            changed.extend(package_aliases(package));
        }
    }

    Some(changed)
}

fn dependency_object<'a>(
    section: Option<&'a Value>,
    empty: &'a serde_json::Map<String, Value>,
) -> Option<&'a serde_json::Map<String, Value>> {
    section.map_or(Some(empty), Value::as_object)
}

fn changed_bun_lock_packages(old_text: &str, new_text: &str) -> Option<BTreeSet<Box<str>>> {
    let old_json = serde_json::from_str::<Value>(old_text).ok()?;
    let new_json = serde_json::from_str::<Value>(new_text).ok()?;
    let old_object = old_json.as_object()?;
    let new_object = new_json.as_object()?;

    for key in old_object.keys().chain(new_object.keys()) {
        if key == "packages" {
            continue;
        }
        if old_object.get(key) != new_object.get(key) {
            return None;
        }
    }

    let old_packages = old_object.get("packages")?.as_object()?;
    let new_packages = new_object.get("packages")?.as_object()?;
    let dependents = lockfile_dependents(new_packages);
    let mut changed = BTreeSet::<Box<str>>::new();

    for package in old_packages.keys().chain(new_packages.keys()) {
        if old_packages.get(package) != new_packages.get(package) {
            changed.extend(package_aliases(package));
            changed.extend(lockfile_dependent_closure(package, &dependents));
        }
    }

    Some(changed)
}

fn lockfile_dependents(
    packages: &serde_json::Map<String, Value>,
) -> BTreeMap<Box<str>, BTreeSet<Box<str>>> {
    let mut dependents = BTreeMap::<Box<str>, BTreeSet<Box<str>>>::new();

    for (package, entry) in packages {
        for dependency in lockfile_entry_dependencies(entry) {
            dependents
                .entry(dependency)
                .or_default()
                .insert(Box::<str>::from(package.as_str()));
        }
    }

    dependents
}

fn lockfile_entry_dependencies(entry: &Value) -> BTreeSet<Box<str>> {
    let mut dependencies = BTreeSet::<Box<str>>::new();
    let Some(metadata) = entry.as_array().and_then(|values| values.get(2)) else {
        return dependencies;
    };
    for key in ["dependencies", "peerDependencies", "optionalDependencies"] {
        if let Some(values) = metadata.get(key).and_then(Value::as_object) {
            dependencies.extend(values.keys().flat_map(|package| package_aliases(package)));
        }
    }

    dependencies
}

fn lockfile_dependent_closure(
    package: &str,
    dependents: &BTreeMap<Box<str>, BTreeSet<Box<str>>>,
) -> BTreeSet<Box<str>> {
    let mut seen = BTreeSet::<Box<str>>::new();
    let mut stack = Vec::<Box<str>>::from([Box::<str>::from(package)]);

    while let Some(current) = stack.pop() {
        let Some(package_dependents) = dependents.get(current.as_ref()) else {
            continue;
        };
        for dependent in package_dependents {
            if seen.insert(dependent.clone()) {
                stack.push(dependent.clone());
            }
        }
    }

    seen
}

fn importers_for_packages<G>(
    graph: &G,
    packages: &BTreeSet<Box<str>>,
) -> Box<[roots::RootRelativePath]>
where
    G: dependencies::GraphView,
{
    let mut importers = BTreeSet::<roots::RootRelativePath>::new();
    for package in packages {
        importers.extend(graph.external_importers(package.as_ref()).iter().cloned());
    }

    importers.into_iter().collect()
}

fn package_aliases(package: &str) -> BTreeSet<Box<str>> {
    let mut packages = BTreeSet::<Box<str>>::from([Box::<str>::from(package)]);
    if let Some(untyped_package) = package.strip_prefix("@types/") {
        if untyped_package == "node" {
            packages.insert(Box::<str>::from("node:"));
        } else if let Some((scope, name)) = untyped_package.split_once("__") {
            packages.insert(format!("@{scope}/{name}").into_boxed_str());
        } else {
            packages.insert(Box::<str>::from(untyped_package));
        }
    }

    packages
}

fn is_global_risk_package(package: &str) -> bool {
    matches!(
        package,
        "bun"
            | "next"
            | "react"
            | "react-dom"
            | "typescript"
            | "tsx"
            | "prisma"
            | "@prisma/client"
            | "@types/node"
            | "@types/react"
            | "@types/react-dom"
            | "jest"
            | "bun-plugin-tailwind"
            | "tailwindcss"
            | "webpack"
            | "vite"
            | "eslint"
    )
}

const fn importer_change(path: roots::RootRelativePath) -> vcs::ChangedFile {
    vcs::ChangedFile {
        status: vcs::ChangedFileStatus::Modified,
        path,
        previous_path: None,
    }
}

fn is_package_metadata_path(path: &roots::RootRelativePath) -> bool {
    matches!(path.as_str(), PACKAGE_JSON | BUN_LOCK | BUN_LOCKB)
}

fn compare_changed_files(left: &vcs::ChangedFile, right: &vcs::ChangedFile) -> std::cmp::Ordering {
    left.path
        .cmp(&right.path)
        .then_with(|| left.previous_path.cmp(&right.previous_path))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::dependencies;
    use crate::failure;
    use crate::roots;
    use crate::vcs;
    use crate::vcs::ChangeSetView;

    #[derive(Clone, Debug)]
    struct FixtureRepository {
        files: BTreeMap<(Box<str>, roots::RootRelativePath), Box<str>>,
    }

    impl vcs::GitRepository for FixtureRepository {
        fn diff_name_status(&self, _request: &vcs::DiffRequest) -> failure::Result<Box<str>> {
            Ok(Box::<str>::from(""))
        }

        fn diff_worktree_name_status(&self) -> failure::Result<Box<str>> {
            Ok(Box::<str>::from(""))
        }

        fn untracked_files(&self) -> failure::Result<Box<str>> {
            Ok(Box::<str>::from(""))
        }

        fn file_at_revision(
            &self,
            revision: &str,
            path: &roots::RootRelativePath,
        ) -> failure::Result<Option<Box<str>>> {
            Ok(self
                .files
                .get(&(Box::<str>::from(revision), path.clone()))
                .cloned())
        }
    }

    #[derive(Clone, Debug, Default)]
    struct FixtureGraph {
        external_importers: BTreeMap<Box<str>, Box<[roots::RootRelativePath]>>,
        empty: Box<[roots::RootRelativePath]>,
    }

    impl dependencies::GraphView for FixtureGraph {
        fn reverse_dependents(
            &self,
            _path: &roots::RootRelativePath,
        ) -> &[roots::RootRelativePath] {
            self.empty.as_ref()
        }

        fn dependencies(&self, _path: &roots::RootRelativePath) -> &[roots::RootRelativePath] {
            self.empty.as_ref()
        }

        fn external_importers(&self, package: &str) -> &[roots::RootRelativePath] {
            self.external_importers
                .get(package)
                .map_or_else(|| self.empty.as_ref(), Box::as_ref)
        }
    }

    fn path(value: &str) -> roots::RootRelativePath {
        roots::RootRelativePath::try_from(value).unwrap()
    }

    fn modified(value: &str) -> vcs::ChangedFile {
        vcs::ChangedFile {
            status: vcs::ChangedFileStatus::Modified,
            path: path(value),
            previous_path: None,
        }
    }

    fn repository_with_file(path_value: &str, old_text: &str, new_text: &str) -> FixtureRepository {
        FixtureRepository {
            files: BTreeMap::from([
                (
                    (Box::<str>::from("base"), path(path_value)),
                    Box::<str>::from(old_text),
                ),
                (
                    (Box::<str>::from("head"), path(path_value)),
                    Box::<str>::from(new_text),
                ),
            ]),
        }
    }

    #[test]
    fn package_json_dependency_changes_scope_to_external_importers() {
        let repository = repository_with_file(
            super::PACKAGE_JSON,
            r#"{"name":"app","dependencies":{"lodash":"1.0.0"}}"#,
            r#"{"name":"app","dependencies":{"lodash":"2.0.0"}}"#,
        );
        let graph = FixtureGraph {
            external_importers: BTreeMap::from([(
                Box::<str>::from("lodash"),
                Box::from([path("src/uses-lodash.ts")]),
            )]),
            empty: Box::from([]),
        };
        let changes = vcs::ChangeSet {
            files: Box::from([modified(super::PACKAGE_JSON)]),
        };

        let scoped = super::scoped_changes(&super::ScopeRequest {
            repository: &repository,
            base: "base",
            head: "head",
            changes: &changes,
            graph: &graph,
        })
        .unwrap();

        assert_eq!(scoped.files(), &[modified("src/uses-lodash.ts")]);
    }

    #[test]
    fn package_json_script_changes_remain_global_invalidators() {
        let repository = repository_with_file(
            super::PACKAGE_JSON,
            r#"{"scripts":{"test":"bun test"},"dependencies":{"lodash":"1.0.0"}}"#,
            r#"{"scripts":{"test":"bun test --coverage"},"dependencies":{"lodash":"1.0.0"}}"#,
        );
        let graph = FixtureGraph::default();
        let changes = vcs::ChangeSet {
            files: Box::from([modified(super::PACKAGE_JSON)]),
        };

        let scoped = super::scoped_changes(&super::ScopeRequest {
            repository: &repository,
            base: "base",
            head: "head",
            changes: &changes,
            graph: &graph,
        })
        .unwrap();

        assert_eq!(scoped.files(), changes.files());
    }

    #[test]
    fn bun_lock_transitive_changes_scope_to_imported_dependents() {
        let repository = repository_with_file(
            super::BUN_LOCK,
            r#"{"lockfileVersion":1,"packages":{"app-lib":["app-lib@1.0.0","",{"dependencies":{"leaf":"1.0.0"}},"sha"],"leaf":["leaf@1.0.0","",{},"sha"]}}"#,
            r#"{"lockfileVersion":1,"packages":{"app-lib":["app-lib@1.0.0","",{"dependencies":{"leaf":"1.0.0"}},"sha"],"leaf":["leaf@1.0.1","",{},"sha2"]}}"#,
        );
        let graph = FixtureGraph {
            external_importers: BTreeMap::from([(
                Box::<str>::from("app-lib"),
                Box::from([path("src/uses-app-lib.ts")]),
            )]),
            empty: Box::from([]),
        };
        let changes = vcs::ChangeSet {
            files: Box::from([modified(super::BUN_LOCK)]),
        };

        let scoped = super::scoped_changes(&super::ScopeRequest {
            repository: &repository,
            base: "base",
            head: "head",
            changes: &changes,
            graph: &graph,
        })
        .unwrap();

        assert_eq!(scoped.files(), &[modified("src/uses-app-lib.ts")]);
    }

    #[test]
    fn dependency_change_with_no_known_importer_stays_fail_closed() {
        let repository = repository_with_file(
            super::PACKAGE_JSON,
            r#"{"name":"app","dependencies":{"lodash":"1.0.0"}}"#,
            r#"{"name":"app","dependencies":{"lodash":"2.0.0"}}"#,
        );
        // The graph records no importer for lodash, modeling a package used only
        // through a type-only import or under `dynamicImports: ignore`.
        let graph = FixtureGraph::default();
        let changes = vcs::ChangeSet {
            files: Box::from([modified(super::PACKAGE_JSON)]),
        };

        let scoped = super::scoped_changes(&super::ScopeRequest {
            repository: &repository,
            base: "base",
            head: "head",
            changes: &changes,
            graph: &graph,
        })
        .unwrap();

        // Desired: scoping to zero known importers must not silently erase the
        // change; package.json should remain so it can fail closed. Current code
        // drops it entirely, yielding an empty change set.
        assert!(
            scoped
                .files()
                .iter()
                .any(|change| change.path == path(super::PACKAGE_JSON)),
            "package metadata change must stay fail-closed when it maps to no importers, got {} files",
            scoped.files().len(),
        );
    }

    #[test]
    fn risk_package_changes_remain_global_invalidators() {
        let repository = repository_with_file(
            super::PACKAGE_JSON,
            r#"{"dependencies":{"react":"18.0.0"}}"#,
            r#"{"dependencies":{"react":"19.0.0"}}"#,
        );
        let graph = FixtureGraph::default();
        let changes = vcs::ChangeSet {
            files: Box::from([modified(super::PACKAGE_JSON)]),
        };

        let scoped = super::scoped_changes(&super::ScopeRequest {
            repository: &repository,
            base: "base",
            head: "head",
            changes: &changes,
            graph: &graph,
        })
        .unwrap();

        assert_eq!(scoped.files(), changes.files());
    }
}
