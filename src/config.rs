//! Configuration contracts for discovery, resolution, and invalidation rules.

use std::collections::BTreeMap;
use std::path;

use serde::Deserialize;

use crate::failure;
use crate::roots;

type RawPathMappings = BTreeMap<Box<str>, Box<[Box<str>]>>;

/// Behavior used when a dynamic import cannot be statically resolved.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum UnknownDynamicImportBehavior {
    /// Treat unresolved dynamic imports as selecting the full suite.
    FailClosed,
    /// Ignore unresolved dynamic imports during graph construction.
    Ignore,
}

/// Immutable configuration values consumed by pure selection modules.
pub trait View {
    /// Returns source include globs.
    #[must_use]
    fn source_includes(&self) -> &[Pattern];

    /// Returns file exclusion globs.
    #[must_use]
    fn excludes(&self) -> &[Pattern];

    /// Returns test file globs.
    #[must_use]
    fn test_patterns(&self) -> &[TestFilePattern];

    /// Returns global invalidator globs.
    #[must_use]
    fn global_invalidators(&self) -> &[Pattern];

    /// Returns dynamic import behavior.
    #[must_use]
    fn dynamic_imports(&self) -> UnknownDynamicImportBehavior;

    /// Returns globs whose files are exempt from the fail-closed dynamic import policy.
    #[must_use]
    fn dynamic_import_ignore(&self) -> &[Pattern] {
        &[]
    }

    /// Returns TypeScript path mappings used by import resolution.
    #[must_use]
    fn path_mappings(&self) -> &[PathMapping] {
        &[]
    }

    /// Returns the TypeScript base URL used by import resolution.
    #[must_use]
    fn base_url(&self) -> Option<&str> {
        None
    }
}

/// Glob pattern text retained as a typed configuration value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Pattern {
    value: Box<str>,
}

impl Pattern {
    /// Returns the configured glob text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.value.as_ref()
    }
}

impl TryFrom<&str> for Pattern {
    type Error = failure::AppError;

    fn try_from(pattern: &str) -> failure::Result<Self> {
        validate_glob(pattern)?;

        Ok(Self {
            value: Box::<str>::from(pattern),
        })
    }
}

/// Glob pattern used to identify test files.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TestFilePattern {
    value: Box<str>,
}

impl TestFilePattern {
    /// Returns the configured test-file glob text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.value.as_ref()
    }
}

impl TryFrom<&str> for TestFilePattern {
    type Error = failure::AppError;

    fn try_from(pattern: &str) -> failure::Result<Self> {
        validate_glob(pattern)?;

        Ok(Self {
            value: Box::<str>::from(pattern),
        })
    }
}

/// TypeScript path alias mapping from an import pattern to target patterns.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathMapping {
    pattern: Box<str>,
    targets: Box<[Box<str>]>,
}

impl PathMapping {
    /// Creates a path mapping from a TS `paths` entry.
    ///
    /// # Errors
    ///
    /// Returns an error when the pattern or target list is empty.
    pub fn try_new(pattern: &str, targets: Box<[Box<str>]>) -> failure::Result<Self> {
        if pattern.is_empty() || targets.is_empty() {
            return Err(failure::AppError::Config {
                message: Box::<str>::from("path mapping requires a pattern and target"),
            });
        }

        Ok(Self {
            pattern: Box::<str>::from(pattern),
            targets,
        })
    }

    /// Returns the import specifier pattern.
    #[must_use]
    pub fn pattern(&self) -> &str {
        self.pattern.as_ref()
    }

    /// Returns target path patterns in configured order.
    #[must_use]
    pub fn targets(&self) -> &[Box<str>] {
        self.targets.as_ref()
    }
}

/// Raw configuration loaded from the repository boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    source_includes: Box<[Pattern]>,
    excludes: Box<[Pattern]>,
    test_patterns: Box<[TestFilePattern]>,
    global_invalidators: Box<[Pattern]>,
    dynamic_imports: UnknownDynamicImportBehavior,
    dynamic_import_ignore: Box<[Pattern]>,
    path_mappings: Box<[PathMapping]>,
    base_url: Option<Box<str>>,
}

/// Resolved configuration consumed by discovery, resolution, and selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedConfig {
    source_includes: Box<[Pattern]>,
    excludes: Box<[Pattern]>,
    test_patterns: Box<[TestFilePattern]>,
    global_invalidators: Box<[Pattern]>,
    dynamic_imports: UnknownDynamicImportBehavior,
    dynamic_import_ignore: Box<[Pattern]>,
    path_mappings: Box<[PathMapping]>,
    base_url: Option<Box<str>>,
}

impl View for ResolvedConfig {
    fn source_includes(&self) -> &[Pattern] {
        self.source_includes.as_ref()
    }

    fn excludes(&self) -> &[Pattern] {
        self.excludes.as_ref()
    }

    fn test_patterns(&self) -> &[TestFilePattern] {
        self.test_patterns.as_ref()
    }

    fn global_invalidators(&self) -> &[Pattern] {
        self.global_invalidators.as_ref()
    }

    fn dynamic_imports(&self) -> UnknownDynamicImportBehavior {
        self.dynamic_imports
    }

    fn dynamic_import_ignore(&self) -> &[Pattern] {
        self.dynamic_import_ignore.as_ref()
    }

    fn path_mappings(&self) -> &[PathMapping] {
        self.path_mappings.as_ref()
    }

    fn base_url(&self) -> Option<&str> {
        self.base_url.as_deref()
    }
}

/// Filesystem capability required by configuration loading.
pub trait LoaderFileSystem {
    /// Reads UTF-8 text from a root-relative path.
    ///
    /// # Errors
    ///
    /// Returns an error when the adapter cannot read the requested path.
    fn read_text(&self, path: &roots::RootRelativePath) -> failure::Result<Box<str>>;
}

/// Request object for loading repository configuration.
pub struct LoadRequest<F> {
    /// Filesystem adapter used at the configuration boundary.
    pub file_system: F,
    /// Optional root-relative config file path.
    pub config_path: Option<roots::RootRelativePath>,
}

/// Loads repository configuration from disk or defaults.
///
/// # Errors
///
/// Returns an error when configuration input cannot be read or parsed.
pub fn load<F>(request: LoadRequest<F>) -> failure::Result<ResolvedConfig>
where
    F: LoaderFileSystem,
{
    match request.config_path {
        Some(config_path) => {
            let raw_config = request.file_system.read_text(&config_path)?;
            let config = parse_file_config(&ParseFileConfigRequest {
                raw_config: raw_config.as_ref(),
                config_path: &config_path,
            })?;

            config.resolve()
        }
        None => FileConfig::default().resolve(),
    }
}

/// Reports whether a root-relative path matches a global invalidator.
///
/// # Errors
///
/// Returns an error when a configured glob cannot be compiled.
pub fn matches_global_invalidator<C>(
    config: &C,
    path: &roots::RootRelativePath,
) -> failure::Result<bool>
where
    C: View,
{
    patterns_match(config.global_invalidators(), path)
}

/// Reports whether a root-relative path matches configured test-file patterns.
///
/// # Errors
///
/// Returns an error when a configured glob cannot be compiled.
pub fn matches_test_file<C>(config: &C, path: &roots::RootRelativePath) -> failure::Result<bool>
where
    C: View,
{
    test_patterns_match(config.test_patterns(), path)
}

/// Reports whether a file is exempt from the fail-closed dynamic import policy.
///
/// # Errors
///
/// Returns an error when a configured glob cannot be compiled.
pub fn matches_dynamic_import_ignore<C>(
    config: &C,
    path: &roots::RootRelativePath,
) -> failure::Result<bool>
where
    C: View,
{
    patterns_match(config.dynamic_import_ignore(), path)
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileConfig {
    source_includes: Option<Box<[Box<str>]>>,
    excludes: Option<Box<[Box<str>]>>,
    test_patterns: Option<Box<[Box<str>]>>,
    global_invalidators: Option<Box<[Box<str>]>>,
    dynamic_imports: Option<UnknownDynamicImportBehavior>,
    dynamic_import_ignore: Option<Box<[Box<str>]>>,
    compiler_options: Option<CompilerOptions>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConfigFileKind {
    AffectedTests,
    TypeScript,
}

struct ParseFileConfigRequest<'a> {
    raw_config: &'a str,
    config_path: &'a roots::RootRelativePath,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct CompilerOptions {
    base_url: Option<Box<str>>,
    paths: Option<RawPathMappings>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct TypeScriptConfig {
    include: Option<Box<[Box<str>]>>,
    exclude: Option<Box<[Box<str>]>>,
    compiler_options: Option<CompilerOptions>,
}

impl From<TypeScriptConfig> for FileConfig {
    fn from(config: TypeScriptConfig) -> Self {
        Self {
            source_includes: config
                .include
                .map(|patterns| normalize_typescript_includes(patterns.as_ref())),
            excludes: config
                .exclude
                .map(|patterns| normalize_typescript_excludes(patterns.as_ref())),
            test_patterns: None,
            global_invalidators: None,
            dynamic_imports: None,
            dynamic_import_ignore: None,
            compiler_options: config.compiler_options,
        }
    }
}

impl FileConfig {
    fn resolve(self) -> failure::Result<ResolvedConfig> {
        let source_includes = self
            .source_includes
            .unwrap_or_else(default_source_include_values);
        let excludes = self.excludes.unwrap_or_else(default_exclude_values);
        let test_patterns = self
            .test_patterns
            .unwrap_or_else(default_test_pattern_values);
        let global_invalidators = self
            .global_invalidators
            .unwrap_or_else(default_global_invalidator_values);
        let compiler_options = self.compiler_options;
        let base_url = compile_base_url(
            compiler_options
                .as_ref()
                .and_then(|options| options.base_url.as_deref()),
        );

        Ok(ResolvedConfig {
            source_includes: compile_patterns(source_includes.as_ref())?,
            excludes: compile_patterns(excludes.as_ref())?,
            test_patterns: compile_test_patterns(test_patterns.as_ref())?,
            global_invalidators: compile_patterns(global_invalidators.as_ref())?,
            dynamic_imports: self
                .dynamic_imports
                .unwrap_or(UnknownDynamicImportBehavior::FailClosed),
            dynamic_import_ignore: compile_patterns(
                self.dynamic_import_ignore.unwrap_or_default().as_ref(),
            )?,
            path_mappings: compile_path_mappings(compiler_options)?,
            base_url,
        })
    }
}

fn parse_file_config(request: &ParseFileConfigRequest<'_>) -> failure::Result<FileConfig> {
    match config_file_kind(request.config_path) {
        ConfigFileKind::AffectedTests => parse_affected_tests_config(request.raw_config),
        ConfigFileKind::TypeScript => parse_typescript_config(request.raw_config),
    }
}

fn config_file_kind(config_path: &roots::RootRelativePath) -> ConfigFileKind {
    if config_path.as_str() == "tsconfig.json" {
        ConfigFileKind::TypeScript
    } else {
        ConfigFileKind::AffectedTests
    }
}

fn parse_affected_tests_config(raw_config: &str) -> failure::Result<FileConfig> {
    serde_json::from_str(raw_config).map_err(|error| parse_config_error(&error))
}

fn parse_typescript_config(raw_config: &str) -> failure::Result<FileConfig> {
    serde_json::from_str::<TypeScriptConfig>(raw_config)
        .map(FileConfig::from)
        .map_err(|error| parse_config_error(&error))
}

fn parse_config_error(error: &serde_json::Error) -> failure::AppError {
    failure::AppError::Config {
        message: format!("failed to parse config: {error}").into_boxed_str(),
    }
}

fn compile_patterns(patterns: &[Box<str>]) -> failure::Result<Box<[Pattern]>> {
    patterns
        .iter()
        .map(|pattern| Pattern::try_from(pattern.as_ref()))
        .collect()
}

fn compile_test_patterns(patterns: &[Box<str>]) -> failure::Result<Box<[TestFilePattern]>> {
    patterns
        .iter()
        .map(|pattern| TestFilePattern::try_from(pattern.as_ref()))
        .collect()
}

fn compile_path_mappings(
    compiler_options: Option<CompilerOptions>,
) -> failure::Result<Box<[PathMapping]>> {
    let Some(options) = compiler_options else {
        return Ok(default_path_mappings());
    };
    let Some(paths) = options.paths else {
        return Ok(default_path_mappings());
    };

    paths
        .into_iter()
        .map(|(pattern, targets)| {
            PathMapping::try_new(
                pattern.as_ref(),
                normalize_targets(options.base_url.as_deref(), targets.as_ref()).into_boxed_slice(),
            )
        })
        .collect()
}

fn compile_base_url(base_url: Option<&str>) -> Option<Box<str>> {
    base_url.map(trim_current_directory).map(Box::<str>::from)
}

fn normalize_targets(base_url: Option<&str>, targets: &[Box<str>]) -> Vec<Box<str>> {
    targets
        .iter()
        .map(|target| normalize_target(base_url, target.as_ref()))
        .collect()
}

fn normalize_target(base_url: Option<&str>, target: &str) -> Box<str> {
    let clean_target = trim_current_directory(target);
    match base_url.map(trim_current_directory) {
        Some("") | None => Box::<str>::from(clean_target),
        Some(clean_base_url) if clean_target.is_empty() => Box::<str>::from(clean_base_url),
        Some(clean_base_url) => Box::<str>::from(format!("{clean_base_url}/{clean_target}")),
    }
}

fn normalize_typescript_includes(patterns: &[Box<str>]) -> Box<[Box<str>]> {
    patterns
        .iter()
        .flat_map(|pattern| typescript_include_patterns(pattern.as_ref()))
        .collect()
}

fn normalize_typescript_excludes(patterns: &[Box<str>]) -> Box<[Box<str>]> {
    patterns
        .iter()
        .map(|pattern| typescript_exclude_pattern(pattern.as_ref()))
        .collect()
}

fn typescript_include_patterns(pattern: &str) -> Box<[Box<str>]> {
    let normalized = normalize_typescript_pattern_path(pattern);
    if is_directory_style_typescript_pattern(normalized) {
        Box::from([
            Box::<str>::from(format!("{normalized}/**/*.ts")),
            Box::<str>::from(format!("{normalized}/**/*.tsx")),
        ])
    } else {
        Box::from([Box::<str>::from(normalized)])
    }
}

fn typescript_exclude_pattern(pattern: &str) -> Box<str> {
    let normalized = normalize_typescript_pattern_path(pattern);
    if is_directory_style_typescript_pattern(normalized) {
        Box::<str>::from(format!("{normalized}/**"))
    } else {
        Box::<str>::from(normalized)
    }
}

fn is_directory_style_typescript_pattern(pattern: &str) -> bool {
    !pattern.is_empty()
        && !contains_glob_meta(pattern)
        && path::Path::new(pattern).extension().is_none()
}

fn contains_glob_meta(pattern: &str) -> bool {
    pattern.contains('*')
        || pattern.contains('?')
        || pattern.contains('[')
        || pattern.contains(']')
        || pattern.contains('{')
        || pattern.contains('}')
}

fn normalize_typescript_pattern_path(pattern: &str) -> &str {
    trim_current_directory(pattern).trim_end_matches('/')
}

fn trim_current_directory(path: &str) -> &str {
    if path == "." {
        ""
    } else {
        path.strip_prefix("./").unwrap_or(path)
    }
}

fn validate_glob(pattern: &str) -> failure::Result<()> {
    globset::Glob::new(pattern)
        .map(|_glob| ())
        .map_err(|error| failure::AppError::Config {
            message: format!("invalid glob pattern `{pattern}`: {error}").into_boxed_str(),
        })
}

fn patterns_match(patterns: &[Pattern], path: &roots::RootRelativePath) -> failure::Result<bool> {
    let glob_set = build_glob_set(patterns)?;

    Ok(glob_set.is_match(path.as_str()))
}

fn test_patterns_match(
    patterns: &[TestFilePattern],
    path: &roots::RootRelativePath,
) -> failure::Result<bool> {
    let mut builder = globset::GlobSetBuilder::new();
    for pattern in patterns {
        let glob =
            globset::Glob::new(pattern.as_str()).map_err(|error| failure::AppError::Config {
                message: format!("invalid test glob pattern `{}`: {error}", pattern.as_str())
                    .into_boxed_str(),
            })?;
        builder.add(glob);
    }

    let glob_set = builder.build().map_err(|error| failure::AppError::Config {
        message: format!("failed to compile test glob set: {error}").into_boxed_str(),
    })?;

    Ok(glob_set.is_match(path.as_str()))
}

fn build_glob_set(patterns: &[Pattern]) -> failure::Result<globset::GlobSet> {
    let mut builder = globset::GlobSetBuilder::new();
    for pattern in patterns {
        let glob =
            globset::Glob::new(pattern.as_str()).map_err(|error| failure::AppError::Config {
                message: format!("invalid glob pattern `{}`: {error}", pattern.as_str())
                    .into_boxed_str(),
            })?;
        builder.add(glob);
    }

    builder.build().map_err(|error| failure::AppError::Config {
        message: format!("failed to compile glob set: {error}").into_boxed_str(),
    })
}

fn default_source_include_values() -> Box<[Box<str>]> {
    Box::from([Box::<str>::from("**/*.ts"), Box::<str>::from("**/*.tsx")])
}

fn default_exclude_values() -> Box<[Box<str>]> {
    Box::from([
        Box::<str>::from("node_modules/**"),
        Box::<str>::from("dist/**"),
        Box::<str>::from("build/**"),
        Box::<str>::from(".next/**"),
    ])
}

fn default_test_pattern_values() -> Box<[Box<str>]> {
    Box::from([
        Box::<str>::from("**/*.test.ts"),
        Box::<str>::from("**/*.test.tsx"),
        Box::<str>::from("**/*.spec.ts"),
        Box::<str>::from("**/*.spec.tsx"),
        Box::<str>::from("**/__tests__/**/*.ts"),
        Box::<str>::from("**/__tests__/**/*.tsx"),
    ])
}

fn default_global_invalidator_values() -> Box<[Box<str>]> {
    Box::from([
        Box::<str>::from("package.json"),
        Box::<str>::from("bun.lock"),
        Box::<str>::from("bun.lockb"),
        Box::<str>::from("tsconfig.json"),
    ])
}

fn default_path_mappings() -> Box<[PathMapping]> {
    Box::from([])
}

#[cfg(test)]
mod tests {
    use crate::failure;
    use crate::roots;

    const TSCONFIG_DIRECTORY_ENTRIES: &str = r#"{
  "include": ["./src", "app/**/*.ts", "next-env.d.ts"],
  "exclude": ["./tests", "build/", "src/generated/**", "scripts/setup.ts"]
}"#;

    #[derive(Clone, Debug, Default)]
    struct FixtureFileSystem {
        content: Option<Box<str>>,
    }

    impl super::LoaderFileSystem for FixtureFileSystem {
        fn read_text(&self, _path: &roots::RootRelativePath) -> failure::Result<Box<str>> {
            self.content
                .clone()
                .ok_or_else(|| failure::AppError::Config {
                    message: Box::<str>::from("fixture config missing"),
                })
        }
    }

    fn pattern_values(patterns: &[super::Pattern]) -> Box<[Box<str>]> {
        patterns
            .iter()
            .map(|pattern| Box::<str>::from(pattern.as_str()))
            .collect()
    }

    fn test_pattern_values(patterns: &[super::TestFilePattern]) -> Box<[Box<str>]> {
        patterns
            .iter()
            .map(|pattern| Box::<str>::from(pattern.as_str()))
            .collect()
    }

    #[test]
    fn defaults_match_prd_patterns_and_invalidators_are_stable() {
        let request = super::LoadRequest {
            file_system: FixtureFileSystem::default(),
            config_path: None,
        };

        let config = super::load(request).unwrap();

        // These defaults represent the V1 TypeScript project shape from the PRD.
        assert_eq!(
            test_pattern_values(super::View::test_patterns(&config)),
            Box::<[Box<str>]>::from([
                Box::<str>::from("**/*.test.ts"),
                Box::<str>::from("**/*.test.tsx"),
                Box::<str>::from("**/*.spec.ts"),
                Box::<str>::from("**/*.spec.tsx"),
                Box::<str>::from("**/__tests__/**/*.ts"),
                Box::<str>::from("**/__tests__/**/*.tsx"),
            ]),
        );
        assert_eq!(
            pattern_values(super::View::global_invalidators(&config)),
            Box::<[Box<str>]>::from([
                Box::<str>::from("package.json"),
                Box::<str>::from("bun.lock"),
                Box::<str>::from("bun.lockb"),
                Box::<str>::from("tsconfig.json"),
            ]),
        );
        assert_eq!(
            super::View::dynamic_imports(&config),
            super::UnknownDynamicImportBehavior::FailClosed,
        );
    }

    #[test]
    fn non_code_files_under_tests_directory_are_not_classified_as_tests() {
        let config = super::load(super::LoadRequest {
            file_system: FixtureFileSystem::default(),
            config_path: None,
        })
        .unwrap();

        // Desired: only runnable test code should be classified as a test. The
        // default `**/__tests__/**/*` pattern also matches fixtures/snapshots,
        // so this non-code path is wrongly treated as a test today.
        let fixture_path =
            roots::RootRelativePath::try_from("src/__tests__/fixtures/data.json").unwrap();
        assert!(
            !super::matches_test_file(&config, &fixture_path).unwrap(),
            "non-code files under __tests__ must not be tests: {}",
            fixture_path.as_str(),
        );
    }

    #[test]
    fn loaded_config_overrides_defaults_and_matches_paths() {
        let request = super::LoadRequest {
            file_system: FixtureFileSystem {
                content: Some(Box::<str>::from(
                    r#"{
  "sourceIncludes": ["app/**/*.ts"],
  "excludes": ["app/generated/**"],
  "testPatterns": ["app/**/*.check.ts"],
  "globalInvalidators": ["workspace.json"],
  "dynamicImports": "ignore"
}"#,
                )),
            },
            config_path: Some(roots::RootRelativePath::try_from("affected-tests.json").unwrap()),
        };

        let config = super::load(request).unwrap();

        // Custom config files let repositories describe their own test naming
        // and fail-closed files without touching host filesystem state.
        assert!(super::matches_test_file(
            &config,
            &roots::RootRelativePath::try_from("app/button.check.ts").unwrap()
        )
        .unwrap());
        assert!(super::matches_global_invalidator(
            &config,
            &roots::RootRelativePath::try_from("workspace.json").unwrap()
        )
        .unwrap());
        assert_eq!(
            super::View::dynamic_imports(&config),
            super::UnknownDynamicImportBehavior::Ignore,
        );
    }

    #[test]
    fn affected_tests_config_rejects_singular_exclude_typo() {
        let request = super::LoadRequest {
            file_system: FixtureFileSystem {
                content: Some(Box::<str>::from(
                    r#"{
  "exclude": ["generated/**"]
}"#,
                )),
            },
            config_path: Some(roots::RootRelativePath::try_from("affected-tests.json").unwrap()),
        };

        let error = super::load(request).unwrap_err();

        // The tool-specific config stays strict so misspelled settings do not
        // silently change affected-test selection.
        assert!(error.to_string().contains("unknown field `exclude`"));
    }

    #[test]
    fn tsconfig_allows_typescript_fields_and_maps_include_exclude() {
        let request = super::LoadRequest {
            file_system: FixtureFileSystem {
                content: Some(Box::<str>::from(
                    r#"{
  "extends": "./tsconfig.base.json",
  "include": ["src/**/*.ts"],
  "exclude": ["src/generated/**"],
  "references": [{ "path": "../shared" }],
  "compilerOptions": {
    "target": "ES2022",
    "baseUrl": ".",
    "paths": {
      "@lib/*": ["src/lib/*"]
    }
  }
}"#,
                )),
            },
            config_path: Some(roots::RootRelativePath::try_from("tsconfig.json").unwrap()),
        };

        let config = super::load(request).unwrap();

        // Real tsconfig files contain TypeScript metadata outside this tool's
        // contract; we preserve the fields that affect discovery/resolution.
        assert_eq!(
            pattern_values(super::View::source_includes(&config)),
            Box::<[Box<str>]>::from([Box::<str>::from("src/**/*.ts")]),
        );
        assert_eq!(
            pattern_values(super::View::excludes(&config)),
            Box::<[Box<str>]>::from([Box::<str>::from("src/generated/**")]),
        );
        assert_eq!(super::View::base_url(&config), Some(""));
        assert_eq!(
            super::View::path_mappings(&config),
            &[
                super::PathMapping::try_new("@lib/*", Box::from([Box::<str>::from("src/lib/*")]),)
                    .unwrap()
            ]
        );
    }

    #[test]
    fn tsconfig_directory_entries_expand_to_recursive_typescript_globs() {
        let request = super::LoadRequest {
            file_system: FixtureFileSystem {
                content: Some(Box::<str>::from(TSCONFIG_DIRECTORY_ENTRIES)),
            },
            config_path: Some(roots::RootRelativePath::try_from("tsconfig.json").unwrap()),
        };

        let config = super::load(request).unwrap();

        // Directory-style tsconfig entries mean recursive TS source roots, while
        // explicit globs and file paths are already precise enough to preserve.
        assert_eq!(
            pattern_values(super::View::source_includes(&config)),
            Box::<[Box<str>]>::from([
                Box::<str>::from("src/**/*.ts"),
                Box::<str>::from("src/**/*.tsx"),
                Box::<str>::from("app/**/*.ts"),
                Box::<str>::from("next-env.d.ts"),
            ]),
        );
        assert_eq!(
            pattern_values(super::View::excludes(&config)),
            Box::<[Box<str>]>::from([
                Box::<str>::from("tests/**"),
                Box::<str>::from("build/**"),
                Box::<str>::from("src/generated/**"),
                Box::<str>::from("scripts/setup.ts"),
            ]),
        );
    }

    #[test]
    fn loaded_compiler_options_paths_are_exposed_for_resolution() {
        let request = super::LoadRequest {
            file_system: FixtureFileSystem {
                content: Some(Box::<str>::from(
                    r#"{
  "compilerOptions": {
    "target": "ES2022",
    "baseUrl": ".",
    "paths": {
      "@lib/*": ["src/lib/*"]
    }
  }
}"#,
                )),
            },
            config_path: Some(roots::RootRelativePath::try_from("tsconfig.json").unwrap()),
        };

        let config = super::load(request).unwrap();
        let mappings = super::View::path_mappings(&config);

        // TS path aliases must stay typed at the config boundary so resolution can
        // distinguish local aliases from external package imports. Real tsconfig
        // files include unrelated compiler options that should not affect this.
        assert_eq!(
            mappings,
            &[
                super::PathMapping::try_new("@lib/*", Box::from([Box::<str>::from("src/lib/*")]),)
                    .unwrap()
            ]
        );
        assert_eq!(super::View::base_url(&config), Some(""));
    }

    #[test]
    fn dynamic_import_ignore_globs_are_parsed_and_matched() {
        let request = super::LoadRequest {
            file_system: FixtureFileSystem {
                content: Some(Box::<str>::from(
                    r#"{
  "dynamicImports": "failClosed",
  "dynamicImportIgnore": ["src/graphql-contract/**", "src/legacy/loader.ts"]
}"#,
                )),
            },
            config_path: Some(roots::RootRelativePath::try_from("affected-tests.json").unwrap()),
        };

        let config = super::load(request).unwrap();

        // The ignore list lets a repo quarantine files whose dynamic imports are
        // genuinely unresolvable, without abandoning fail-closed everywhere else.
        assert_eq!(
            super::View::dynamic_imports(&config),
            super::UnknownDynamicImportBehavior::FailClosed,
        );
        assert!(super::matches_dynamic_import_ignore(
            &config,
            &roots::RootRelativePath::try_from("src/graphql-contract/collector.ts").unwrap(),
        )
        .unwrap());
        assert!(super::matches_dynamic_import_ignore(
            &config,
            &roots::RootRelativePath::try_from("src/legacy/loader.ts").unwrap(),
        )
        .unwrap());
        assert!(!super::matches_dynamic_import_ignore(
            &config,
            &roots::RootRelativePath::try_from("src/router.ts").unwrap(),
        )
        .unwrap());
    }

    #[test]
    fn dynamic_import_ignore_defaults_to_empty() {
        let config = super::load(super::LoadRequest {
            file_system: FixtureFileSystem::default(),
            config_path: None,
        })
        .unwrap();

        // Without configuration nothing is exempted, so fail-closed stays the
        // default behavior for every file.
        assert!(super::View::dynamic_import_ignore(&config).is_empty());
        assert!(!super::matches_dynamic_import_ignore(
            &config,
            &roots::RootRelativePath::try_from("src/graphql-contract/collector.ts").unwrap(),
        )
        .unwrap());
    }
}
