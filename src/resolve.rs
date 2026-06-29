//! Import resolution contracts for relative paths, extensions, index files, and aliases.

use std::cmp;
use std::path;

use globset::Glob;

use crate::failure;
use crate::roots;
use crate::settings;

/// Filesystem probing capability used by import resolution.
pub trait FileExistence {
    /// Reports whether a candidate root-relative path exists.
    ///
    /// # Errors
    ///
    /// Returns an error when probing cannot complete.
    fn exists(&self, path: &roots::RootRelativePath) -> failure::Result<bool>;
}

/// Request object for resolving one import specifier.
pub struct ResolveRequest<C, P> {
    /// Configuration view used for aliases and extension policy.
    pub config: C,
    /// Filesystem probe used for candidate checks.
    pub probe: P,
    /// Importing file path.
    pub importer: roots::RootRelativePath,
    /// Import specifier to resolve.
    pub specifier: roots::ImportSpecifier,
}

/// Resolution outcome for an import specifier.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Outcome {
    /// The import resolved to a root-relative source path.
    Resolved(roots::RootRelativePath),
    /// The import is external to the repository graph.
    External(roots::ImportSpecifier),
    /// The import is local but unresolved.
    Unresolved(roots::ImportSpecifier),
}

/// Import resolver capability used by graph construction.
pub trait ImportResolver {
    /// Resolves one import request into a graph edge outcome.
    ///
    /// # Errors
    ///
    /// Returns an error when filesystem probing or configuration access fails.
    fn resolve<C, P>(&self, request: ResolveRequest<C, P>) -> failure::Result<Outcome>
    where
        C: settings::View,
        P: FileExistence;
}

/// Resolves an import specifier into a graph edge target when possible.
///
/// # Errors
///
/// Returns an error when filesystem probing or configuration access fails.
pub fn import<C, P>(request: ResolveRequest<C, P>) -> failure::Result<Outcome>
where
    C: settings::View,
    P: FileExistence,
{
    let specifier = request.specifier.as_str();
    let candidate_request = CandidateRequest {
        config: &request.config,
        importer: &request.importer,
        specifier,
    };
    let candidate_bases = candidate_bases(&candidate_request);

    match candidate_bases {
        CandidateBases::External => Ok(Outcome::External(request.specifier)),
        CandidateBases::Local(paths) => {
            let outcome = resolve_local_paths(&request.probe, paths.as_ref())?;
            Ok(outcome.unwrap_or_else(|| {
                if all_candidates_are_excluded(&request.config, paths.as_ref())
                    || all_candidates_are_generated(paths.as_ref())
                {
                    Outcome::External(request.specifier)
                } else {
                    Outcome::Unresolved(request.specifier)
                }
            }))
        }
        CandidateBases::MaybeLocal(paths) => resolve_local_paths(&request.probe, paths.as_ref())
            .map(|outcome| outcome.unwrap_or(Outcome::External(request.specifier))),
    }
}

enum CandidateBases {
    External,
    Local(Box<[Box<str>]>),
    MaybeLocal(Box<[Box<str>]>),
}

struct CandidateRequest<'a, C> {
    config: &'a C,
    importer: &'a roots::RootRelativePath,
    specifier: &'a str,
}

fn candidate_bases<C>(request: &CandidateRequest<'_, C>) -> CandidateBases
where
    C: settings::View,
{
    let config = request.config;
    let importer = request.importer;
    let specifier = request.specifier;
    if specifier.starts_with('.') {
        return relative_candidate(importer, specifier).map_or_else(
            || CandidateBases::Local(Box::from([])),
            |candidate| CandidateBases::Local(Box::from([candidate])),
        );
    }

    if let Some(candidate) = root_candidate(specifier) {
        return CandidateBases::Local(Box::from([candidate]));
    }

    let mapped_candidates = mapped_candidates(config, specifier);
    if mapped_candidates.is_empty() {
        base_url_candidate(config, specifier).map_or_else(
            || CandidateBases::External,
            |candidate| CandidateBases::MaybeLocal(Box::from([candidate])),
        )
    } else if specifier_can_fallback_to_external(specifier) {
        CandidateBases::MaybeLocal(mapped_candidates.into_boxed_slice())
    } else {
        CandidateBases::Local(mapped_candidates.into_boxed_slice())
    }
}

fn specifier_can_fallback_to_external(specifier: &str) -> bool {
    !specifier.starts_with("@/") && !specifier.starts_with("src/") && !specifier.starts_with("~/")
}

fn base_url_candidate<C>(config: &C, specifier: &str) -> Option<Box<str>>
where
    C: settings::View,
{
    config.base_url().map(|base_url| {
        if base_url.is_empty() {
            Box::<str>::from(specifier)
        } else {
            Box::<str>::from(format!("{base_url}/{specifier}"))
        }
    })
}

fn relative_candidate(importer: &roots::RootRelativePath, specifier: &str) -> Option<Box<str>> {
    let mut segments = importer_directory_segments(importer);
    for segment in specifier.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            segment => segments.push(Box::<str>::from(segment)),
        }
    }

    Some(join_segments(segments.as_ref()).into_boxed_str())
}

fn importer_directory_segments(importer: &roots::RootRelativePath) -> Vec<Box<str>> {
    let mut segments = Vec::<Box<str>>::new();
    let mut importer_segments = importer.as_str().split('/').peekable();

    while let Some(segment) = importer_segments.next() {
        if importer_segments.peek().is_some() {
            segments.push(Box::<str>::from(segment));
        }
    }

    segments
}

fn root_candidate(specifier: &str) -> Option<Box<str>> {
    if let Some(root_path) = specifier.strip_prefix('/') {
        return Some(Box::<str>::from(root_path));
    }

    specifier
        .strip_prefix("~/")
        .map(|root_path| Box::<str>::from(format!("src/{root_path}")))
        .or_else(|| {
            specifier
                .strip_prefix("src/")
                .map(|_root_path| Box::<str>::from(specifier))
        })
}

fn mapped_candidates<C>(config: &C, specifier: &str) -> Vec<Box<str>>
where
    C: settings::View,
{
    let mut candidates = Vec::<MatchedTarget>::new();
    for (order, mapping) in config.path_mappings().iter().enumerate() {
        if let Some(pattern_match) = match_pattern(mapping.pattern(), specifier) {
            for target in mapping.targets() {
                candidates.push(MatchedTarget {
                    target: apply_target(target.as_ref(), pattern_match.capture.as_ref()),
                    score: pattern_match.score,
                    order,
                });
            }
        }
    }

    candidates.sort_by(compare_matched_targets);
    candidates
        .into_iter()
        .map(|candidate| candidate.target)
        .collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MatchedTarget {
    target: Box<str>,
    score: MatchScore,
    order: usize,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct MatchScore {
    exact: bool,
    prefix_len: usize,
    suffix_len: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PatternMatch {
    capture: Box<str>,
    score: MatchScore,
}

fn compare_matched_targets(left: &MatchedTarget, right: &MatchedTarget) -> cmp::Ordering {
    right
        .score
        .cmp(&left.score)
        .then(left.order.cmp(&right.order))
}

fn match_pattern(pattern: &str, specifier: &str) -> Option<PatternMatch> {
    match pattern.split_once('*') {
        Some((prefix, suffix)) => {
            let capture = specifier
                .strip_prefix(prefix)
                .and_then(|remaining| remaining.strip_suffix(suffix))?;

            Some(PatternMatch {
                capture: Box::<str>::from(capture),
                score: MatchScore {
                    exact: false,
                    prefix_len: prefix.len(),
                    suffix_len: suffix.len(),
                },
            })
        }
        None if pattern == specifier => Some(PatternMatch {
            capture: Box::<str>::from(""),
            score: MatchScore {
                exact: true,
                prefix_len: pattern.len(),
                suffix_len: 0,
            },
        }),
        None => None,
    }
}

fn apply_target(target: &str, capture: &str) -> Box<str> {
    match target.split_once('*') {
        Some((prefix, suffix)) => Box::<str>::from(format!("{prefix}{capture}{suffix}")),
        None => Box::<str>::from(target),
    }
}

fn resolve_local_paths<P>(
    probe: &P,
    candidate_bases: &[Box<str>],
) -> failure::Result<Option<Outcome>>
where
    P: FileExistence,
{
    for candidate_base in candidate_bases {
        if let Some(path) = resolve_candidate(probe, candidate_base.as_ref())? {
            return Ok(Some(Outcome::Resolved(path)));
        }
    }

    Ok(None)
}

fn all_candidates_are_excluded<C>(config: &C, candidate_bases: &[Box<str>]) -> bool
where
    C: settings::View,
{
    !candidate_bases.is_empty()
        && candidate_bases
            .iter()
            .all(|candidate| path_matches_any_pattern(candidate.as_ref(), config.excludes()))
}

fn all_candidates_are_generated(candidate_bases: &[Box<str>]) -> bool {
    !candidate_bases.is_empty()
        && candidate_bases
            .iter()
            .all(|candidate| candidate_is_generated(candidate.as_ref()))
}

fn candidate_is_generated(candidate: &str) -> bool {
    candidate == "generated"
        || candidate.starts_with("generated/")
        || candidate.contains("/generated/")
}

fn path_matches_any_pattern(path: &str, patterns: &[settings::Pattern]) -> bool {
    patterns
        .iter()
        .any(|pattern| path_matches_pattern(path, pattern.as_str()))
}

fn path_matches_pattern(path: &str, pattern: &str) -> bool {
    Glob::new(pattern).is_ok_and(|glob| glob.compile_matcher().is_match(path))
}

fn resolve_candidate<P>(
    probe: &P,
    candidate_base: &str,
) -> failure::Result<Option<roots::RootRelativePath>>
where
    P: FileExistence,
{
    for candidate_path in candidate_paths(candidate_base) {
        let Some(normalized_candidate) = normalize_candidate_path(candidate_path.as_ref()) else {
            continue;
        };
        let path = roots::RootRelativePath::try_from(normalized_candidate)?;
        if probe.exists(&path)? {
            return Ok(Some(path));
        }
    }

    Ok(None)
}

fn candidate_paths(candidate_base: &str) -> Box<[Box<str>]> {
    let mut paths = Vec::<Box<str>>::new();
    paths.push(Box::<str>::from(candidate_base));
    paths.extend(
        [".ts", ".tsx", ".mts", ".cts", ".js", ".jsx"]
            .into_iter()
            .map(|extension| Box::<str>::from(format!("{candidate_base}{extension}"))),
    );
    paths.extend(
        [
            "index.ts",
            "index.tsx",
            "index.mts",
            "index.cts",
            "index.js",
            "index.jsx",
        ]
        .into_iter()
        .map(|file_name| Box::<str>::from(format!("{candidate_base}/{file_name}"))),
    );

    paths.into_boxed_slice()
}

fn normalize_candidate_path(candidate_path: &str) -> Option<Box<str>> {
    if candidate_path.is_empty() || candidate_path.contains('\\') {
        return None;
    }

    let mut segments = Vec::<Box<str>>::new();
    for component in path::Path::new(candidate_path).components() {
        match component {
            path::Component::Normal(segment) => {
                let segment_text = segment.to_str()?;
                segments.push(Box::<str>::from(segment_text));
            }
            path::Component::CurDir => {}
            path::Component::ParentDir => {
                segments.pop()?;
            }
            path::Component::RootDir | path::Component::Prefix(_) => return None,
        }
    }

    if segments.is_empty() {
        None
    } else {
        Some(join_segments(segments.as_ref()).into_boxed_str())
    }
}

fn join_segments(segments: &[Box<str>]) -> String {
    let mut path = String::new();
    for segment in segments {
        if !path.is_empty() {
            path.push('/');
        }
        path.push_str(segment.as_ref());
    }

    path
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::failure;
    use crate::roots;
    use crate::settings;

    #[derive(Clone, Debug, Default)]
    struct FixtureConfig {
        source_includes: Box<[settings::Pattern]>,
        excludes: Box<[settings::Pattern]>,
        test_patterns: Box<[settings::TestFilePattern]>,
        global_invalidators: Box<[settings::Pattern]>,
        path_mappings: Box<[settings::PathMapping]>,
        base_url: Option<Box<str>>,
    }

    impl settings::View for FixtureConfig {
        fn source_includes(&self) -> &[settings::Pattern] {
            self.source_includes.as_ref()
        }

        fn excludes(&self) -> &[settings::Pattern] {
            self.excludes.as_ref()
        }

        fn test_patterns(&self) -> &[settings::TestFilePattern] {
            self.test_patterns.as_ref()
        }

        fn global_invalidators(&self) -> &[settings::Pattern] {
            self.global_invalidators.as_ref()
        }

        fn dynamic_imports(&self) -> settings::UnknownDynamicImportBehavior {
            settings::UnknownDynamicImportBehavior::FailClosed
        }

        fn path_mappings(&self) -> &[settings::PathMapping] {
            self.path_mappings.as_ref()
        }

        fn base_url(&self) -> Option<&str> {
            self.base_url.as_deref()
        }
    }

    #[derive(Clone, Debug)]
    struct FixtureProbe {
        existing_paths: BTreeSet<roots::RootRelativePath>,
    }

    impl super::FileExistence for FixtureProbe {
        fn exists(&self, path: &roots::RootRelativePath) -> failure::Result<bool> {
            Ok(self.existing_paths.contains(path))
        }
    }

    fn path(value: &str) -> roots::RootRelativePath {
        roots::RootRelativePath::try_from(value).unwrap()
    }

    fn specifier(value: &str) -> roots::ImportSpecifier {
        roots::ImportSpecifier::try_from(value).unwrap()
    }

    fn config_with_shared_alias() -> FixtureConfig {
        FixtureConfig {
            path_mappings: Box::from([settings::PathMapping::try_new(
                "@shared/*",
                Box::from([Box::<str>::from("src/shared/*")]),
            )
            .unwrap()]),
            ..FixtureConfig::default()
        }
    }

    fn config_with_base_url() -> FixtureConfig {
        FixtureConfig {
            base_url: Some(Box::<str>::from("src")),
            ..FixtureConfig::default()
        }
    }

    fn config_with_overlapping_aliases() -> FixtureConfig {
        FixtureConfig {
            path_mappings: Box::from([
                settings::PathMapping::try_new(
                    "@app/*",
                    Box::from([Box::<str>::from("src/app/*")]),
                )
                .unwrap(),
                settings::PathMapping::try_new(
                    "@app/config",
                    Box::from([Box::<str>::from("src/config/explicit")]),
                )
                .unwrap(),
            ]),
            ..FixtureConfig::default()
        }
    }

    fn config_with_root_alias() -> FixtureConfig {
        FixtureConfig {
            path_mappings: Box::from([settings::PathMapping::try_new(
                "@/*",
                Box::from([Box::<str>::from("src/*")]),
            )
            .unwrap()]),
            ..FixtureConfig::default()
        }
    }

    fn config_with_package_mapping() -> FixtureConfig {
        FixtureConfig {
            path_mappings: Box::from([settings::PathMapping::try_new(
                "type-graphql",
                Box::from([Box::<str>::from("src/index.ts")]),
            )
            .unwrap()]),
            ..FixtureConfig::default()
        }
    }

    fn config_with_excluded_generated_alias() -> FixtureConfig {
        FixtureConfig {
            excludes: Box::from([settings::Pattern::try_from("src/generated/**").unwrap()]),
            path_mappings: Box::from([settings::PathMapping::try_new(
                "@/*",
                Box::from([Box::<str>::from("src/*")]),
            )
            .unwrap()]),
            ..FixtureConfig::default()
        }
    }

    fn config_with_generated_alias() -> FixtureConfig {
        FixtureConfig {
            path_mappings: Box::from([settings::PathMapping::try_new(
                "@/*",
                Box::from([Box::<str>::from("src/*")]),
            )
            .unwrap()]),
            ..FixtureConfig::default()
        }
    }

    #[test]
    fn resolves_relative_extensions_indexes_and_ts_path_aliases() {
        let probe = FixtureProbe {
            existing_paths: BTreeSet::from([
                path("src/components/button.tsx"),
                path("src/components/menu/index.ts"),
                path("src/shared/date.ts"),
            ]),
        };
        let relative_request = super::ResolveRequest {
            config: FixtureConfig::default(),
            probe: probe.clone(),
            importer: path("src/pages/home.tsx"),
            specifier: specifier("../components/button"),
        };

        // The fixture names extensionless, index, and alias-shaped imports because
        // these are the resolution cases that determine graph completeness.
        assert_eq!(
            super::import(relative_request).unwrap(),
            super::Outcome::Resolved(path("src/components/button.tsx")),
        );

        let index_request = super::ResolveRequest {
            config: FixtureConfig::default(),
            probe: probe.clone(),
            importer: path("src/pages/home.tsx"),
            specifier: specifier("../components/menu"),
        };

        assert_eq!(
            super::import(index_request).unwrap(),
            super::Outcome::Resolved(path("src/components/menu/index.ts")),
        );

        let alias_request = super::ResolveRequest {
            config: config_with_shared_alias(),
            probe,
            importer: path("src/pages/home.tsx"),
            specifier: specifier("@shared/date"),
        };

        assert_eq!(
            super::import(alias_request).unwrap(),
            super::Outcome::Resolved(path("src/shared/date.ts")),
        );
    }

    #[test]
    fn external_package_imports_do_not_create_local_graph_nodes() {
        let request = super::ResolveRequest {
            config: config_with_base_url(),
            probe: FixtureProbe {
                existing_paths: BTreeSet::from([path("src/shared/date.ts")]),
            },
            importer: path("src/pages/home.tsx"),
            specifier: specifier("@testing-library/react"),
        };

        // Scoped packages look alias-shaped, but they must stay external unless a
        // configured TS path mapping explicitly matches them.
        assert_eq!(
            super::import(request).unwrap(),
            super::Outcome::External(specifier("@testing-library/react")),
        );
    }

    #[test]
    fn base_url_without_paths_resolves_existing_bare_local_imports() {
        let probe = FixtureProbe {
            existing_paths: BTreeSet::from([path("src/components/Button.tsx")]),
        };
        let request = super::ResolveRequest {
            config: config_with_base_url(),
            probe,
            importer: path("src/pages/home.tsx"),
            specifier: specifier("components/Button"),
        };

        // baseUrl-only projects rely on bare specifiers for local source imports
        // while unresolved package names must still remain external.
        assert_eq!(
            super::import(request).unwrap(),
            super::Outcome::Resolved(path("src/components/Button.tsx")),
        );
    }

    #[test]
    fn exact_path_alias_wins_over_broader_wildcard_alias() {
        let probe = FixtureProbe {
            existing_paths: BTreeSet::from([
                path("src/app/config.ts"),
                path("src/config/explicit.ts"),
            ]),
        };
        let request = super::ResolveRequest {
            config: config_with_overlapping_aliases(),
            probe,
            importer: path("src/pages/home.tsx"),
            specifier: specifier("@app/config"),
        };

        // The broad wildcard is intentionally listed first to prove resolution
        // uses TS-like specificity instead of serialized mapping order.
        assert_eq!(
            super::import(request).unwrap(),
            super::Outcome::Resolved(path("src/config/explicit.ts")),
        );
    }

    #[test]
    fn path_mapping_candidates_normalize_parent_segments_inside_root() {
        let probe = FixtureProbe {
            existing_paths: BTreeSet::from([path("testing-library.ts")]),
        };
        let resolved_request = super::ResolveRequest {
            config: config_with_root_alias(),
            probe: probe.clone(),
            importer: path("src/app/example.test.ts"),
            specifier: specifier("@/../testing-library"),
        };
        let escaped_request = super::ResolveRequest {
            config: config_with_root_alias(),
            probe,
            importer: path("src/app/example.test.ts"),
            specifier: specifier("@/../../outside"),
        };

        // TS path mappings can produce lexical parent segments that still point
        // inside the workspace; candidates escaping the root remain unresolved.
        assert_eq!(
            super::import(resolved_request).unwrap(),
            super::Outcome::Resolved(path("testing-library.ts")),
        );
        assert_eq!(
            super::import(escaped_request).unwrap(),
            super::Outcome::Unresolved(specifier("@/../../outside")),
        );
    }

    #[test]
    fn missing_excluded_generated_aliases_are_external_to_the_graph() {
        let generated_request = super::ResolveRequest {
            config: config_with_excluded_generated_alias(),
            probe: FixtureProbe {
                existing_paths: BTreeSet::new(),
            },
            importer: path("src/pages/api.ts"),
            specifier: specifier("@/generated/client/enums"),
        };
        let generated_without_exclude_request = super::ResolveRequest {
            config: config_with_generated_alias(),
            probe: FixtureProbe {
                existing_paths: BTreeSet::new(),
            },
            importer: path("src/pages/api.ts"),
            specifier: specifier("@/generated/client/runtime"),
        };
        let missing_source_request = super::ResolveRequest {
            config: config_with_root_alias(),
            probe: FixtureProbe {
                existing_paths: BTreeSet::new(),
            },
            importer: path("src/pages/api.ts"),
            specifier: specifier("@/features/missing"),
        };

        // Generated outputs outside the source graph should not force a
        // full run, while ordinary missing source aliases remain fail-closed.
        assert_eq!(
            super::import(generated_request).unwrap(),
            super::Outcome::External(specifier("@/generated/client/enums")),
        );
        assert_eq!(
            super::import(generated_without_exclude_request).unwrap(),
            super::Outcome::External(specifier("@/generated/client/runtime")),
        );
        assert_eq!(
            super::import(missing_source_request).unwrap(),
            super::Outcome::Unresolved(specifier("@/features/missing")),
        );
    }

    #[test]
    fn missing_package_path_mappings_fall_back_to_external_packages() {
        let package_request = super::ResolveRequest {
            config: config_with_package_mapping(),
            probe: FixtureProbe {
                existing_paths: BTreeSet::new(),
            },
            importer: path("src/pages/api.ts"),
            specifier: specifier("type-graphql"),
        };
        let alias_request = super::ResolveRequest {
            config: config_with_root_alias(),
            probe: FixtureProbe {
                existing_paths: BTreeSet::new(),
            },
            importer: path("src/pages/api.ts"),
            specifier: specifier("@/missing"),
        };

        // Package path mappings may be local shims when present, but can safely
        // fall back to external packages when absent; app aliases stay local.
        assert_eq!(
            super::import(package_request).unwrap(),
            super::Outcome::External(specifier("type-graphql")),
        );
        assert_eq!(
            super::import(alias_request).unwrap(),
            super::Outcome::Unresolved(specifier("@/missing")),
        );
    }
}
