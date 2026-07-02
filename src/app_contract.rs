//! Application-level conversion into JSON, shell, graph, and explain contracts.

use std::collections::BTreeSet;

use crate::app_pipeline;
use crate::contract;
use crate::dependencies;
use crate::discovery;
use crate::impact;
use crate::roots;
use crate::vcs;
use crate::vcs::ChangeSetView;

/// Request for converting an affected selection into a CLI result contract.
pub struct SelectionOutputRequest<'a> {
    /// Pure selection result.
    pub result: impact::AffectedResult,
    /// Git changes considered by the selection.
    pub changes: &'a vcs::ChangeSet,
    /// Discovered source and test files.
    pub files: &'a discovery::Files,
    /// Whether reason chains should be included.
    pub include_reasons: bool,
}

/// Converts pure selection output into the serialized CLI contract.
#[must_use]
pub fn command_result(request: SelectionOutputRequest<'_>) -> contract::CommandResult {
    match request.result {
        impact::AffectedResult::Partial(partial) => {
            contract::CommandResult::Partial(contract::PartialResult {
                tests: path_strings(partial.tests.as_ref()),
                reasons: if request.include_reasons {
                    reason_chains(partial.reasons.as_ref())
                } else {
                    Box::from([])
                },
            })
        }
        impact::AffectedResult::Full(reason) => {
            contract::CommandResult::Full(contract::FullResult {
                reason: full_reason_text(&reason),
                tests: path_strings(request.files.tests.as_ref()),
            })
        }
        impact::AffectedResult::None => contract::CommandResult::None(contract::NoneResult {
            changed_files: changed_file_paths(request.changes),
        }),
    }
}

/// Request for converting a dependency graph into its output contract.
#[derive(Clone, Copy, Debug)]
pub struct GraphRequest<'a> {
    /// Built dependency graph.
    pub graph: &'a dependencies::DependencyGraph,
    /// Files discovered for graph classification.
    pub files: &'a discovery::Files,
}

/// Builds the machine-readable graph output.
#[must_use]
pub fn graph_result(request: GraphRequest<'_>) -> contract::GraphResult {
    let test_paths = request.files.tests.iter().cloned().collect::<BTreeSet<_>>();
    let nodes = app_pipeline::all_graph_files(request.files)
        .into_vec()
        .into_iter()
        .map(|path| contract::GraphNode {
            kind: if test_paths.contains(&path) {
                contract::GraphNodeKind::Test
            } else {
                contract::GraphNodeKind::Source
            },
            path: Box::<str>::from(path.as_str()),
        })
        .collect();
    let edges = graph_edges(request.graph, request.files);

    contract::GraphResult { nodes, edges }
}

/// Renders graph output as deterministic plain text.
#[must_use]
pub fn graph_plain(graph: &contract::GraphResult) -> Box<str> {
    let mut content = String::new();
    for node in &graph.nodes {
        content.push_str("node ");
        content.push_str(node.path.as_ref());
        content.push('\n');
    }
    for edge in &graph.edges {
        content.push_str("edge ");
        content.push_str(edge.from.as_ref());
        content.push_str(" -> ");
        content.push_str(edge.to.as_ref());
        content.push('\n');
    }

    content.into_boxed_str()
}

fn path_strings(paths: &[roots::RootRelativePath]) -> Box<[Box<str>]> {
    paths
        .iter()
        .map(|path| Box::<str>::from(path.as_str()))
        .collect()
}

fn changed_file_paths(changes: &vcs::ChangeSet) -> Box<[Box<str>]> {
    changes
        .files()
        .iter()
        .map(|change| Box::<str>::from(change.path.as_str()))
        .collect()
}

fn reason_chains(reasons: &[impact::SelectionReason]) -> Box<[contract::ReasonChain]> {
    reasons
        .iter()
        .map(|reason| contract::ReasonChain {
            changed_file: Box::<str>::from(reason.changed_file.as_str()),
            test_file: Box::<str>::from(reason.test_file.as_str()),
            path: path_strings(reason.path.as_ref()),
        })
        .collect()
}

fn full_reason_text(reason: &impact::FullReason) -> Box<str> {
    match reason {
        impact::FullReason::GlobalInvalidator(path) => {
            format!("global invalidator changed: {path}").into_boxed_str()
        }
        impact::FullReason::DeletedSourceFile(path) => {
            format!("deleted source file: {path}").into_boxed_str()
        }
        impact::FullReason::UnresolvedLocalImport {
            importer,
            specifier,
        } => format!("unresolved local import `{specifier}` in: {importer}").into_boxed_str(),
        impact::FullReason::UnknownDynamicImport(path) => {
            format!("unknown dynamic import in: {path}").into_boxed_str()
        }
    }
}

fn graph_edges(
    graph: &dependencies::DependencyGraph,
    files: &discovery::Files,
) -> Box<[contract::GraphEdge]> {
    let mut edges = Vec::<contract::GraphEdge>::new();
    for path in app_pipeline::all_graph_files(files) {
        for dependency in dependencies::GraphView::dependencies(graph, &path) {
            edges.push(contract::GraphEdge {
                from: Box::<str>::from(path.as_str()),
                to: Box::<str>::from(dependency.as_str()),
            });
        }
    }
    edges.sort_by(|left, right| left.from.cmp(&right.from).then(left.to.cmp(&right.to)));
    edges.into_boxed_slice()
}

#[cfg(test)]
mod tests {
    use crate::contract;
    use crate::discovery;
    use crate::impact;
    use crate::roots;
    use crate::vcs;

    fn path(value: &str) -> roots::RootRelativePath {
        roots::RootRelativePath::try_from(value).unwrap()
    }

    #[test]
    fn full_selection_contract_carries_discovered_tests_for_shell_runners() {
        let changes = vcs::ChangeSet {
            files: Box::from([vcs::ChangedFile {
                status: vcs::ChangedFileStatus::Modified,
                path: path("tsconfig.json"),
                previous_path: None,
            }]),
        };
        let files = discovery::Files {
            sources: Box::from([path("src/button.tsx")]),
            tests: Box::from([path("src/button.test.tsx"), path("src/form.test.ts")]),
        };

        let result = super::command_result(super::SelectionOutputRequest {
            result: impact::AffectedResult::Full(impact::FullReason::GlobalInvalidator(path(
                "tsconfig.json",
            ))),
            changes: &changes,
            files: &files,
            include_reasons: false,
        });

        let contract::CommandResult::Full(full) = result else {
            panic!("expected full result");
        };
        assert_eq!(
            full.tests,
            Box::from([
                Box::<str>::from("src/button.test.tsx"),
                Box::<str>::from("src/form.test.ts"),
            ]),
        );
    }
}
