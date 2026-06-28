//! Serialized CLI output contracts shared with shell, JSON, graph, and explain renderers.

use serde::{Deserialize, Serialize};

/// Top-level machine-readable command result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase", deny_unknown_fields)]
pub enum CommandResult {
    /// A partial set of tests was selected.
    Partial(PartialResult),
    /// The full test suite must run.
    Full(FullResult),
    /// No tests are affected.
    None(NoneResult),
    /// The command failed before a valid selection was available.
    Error(ErrorResult),
}

/// Contract emitted when only specific tests are affected.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PartialResult {
    /// Stable sorted test paths.
    pub tests: Box<[Box<str>]>,
    /// Stable sorted reason chains for selected tests.
    pub reasons: Box<[ReasonChain]>,
}

/// Contract emitted when the full suite is required.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FullResult {
    /// User-facing fail-closed reason.
    pub reason: Box<str>,
}

/// Contract emitted when no tests are affected.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoneResult {
    /// Stable sorted changed files considered by selection.
    pub changed_files: Box<[Box<str>]>,
}

/// Contract emitted when the command fails.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ErrorResult {
    /// Machine-readable error kind.
    pub code: Box<str>,
    /// Human-readable error message.
    pub message: Box<str>,
}

/// Ordered explanation path from a changed file to a selected test.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReasonChain {
    /// Changed file that started the trace.
    pub changed_file: Box<str>,
    /// Selected test reached by reverse traversal.
    pub test_file: Box<str>,
    /// Stable dependency path connecting the change to the test.
    pub path: Box<[Box<str>]>,
}

/// Machine-readable dependency graph output.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GraphResult {
    /// Stable sorted graph nodes.
    pub nodes: Box<[GraphNode]>,
    /// Stable sorted directed dependency edges.
    pub edges: Box<[GraphEdge]>,
}

/// A dependency graph node.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GraphNode {
    /// Root-relative source or test path.
    pub path: Box<str>,
    /// Node classification used by output renderers.
    pub kind: GraphNodeKind,
}

/// Finite dependency graph node classifications.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GraphNodeKind {
    /// Production source file node.
    Source,
    /// Test file node.
    Test,
}

/// A directed dependency graph edge.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GraphEdge {
    /// Importing file path.
    pub from: Box<str>,
    /// Imported file path.
    pub to: Box<str>,
}

/// Machine-readable explanation output for one selected test.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExplainResult {
    /// Selected test being explained.
    pub test_file: Box<str>,
    /// Stable sorted reason chains that selected the test.
    pub reasons: Box<[ReasonChain]>,
}

#[cfg(test)]
mod tests {
    #[test]
    fn command_result_json_uses_camel_case_reason_fields() {
        let result = super::CommandResult::Partial(super::PartialResult {
            tests: Box::from([Box::<str>::from("src/button.test.tsx")]),
            reasons: Box::from([super::ReasonChain {
                changed_file: Box::<str>::from("src/button.tsx"),
                test_file: Box::<str>::from("src/button.test.tsx"),
                path: Box::from([
                    Box::<str>::from("src/button.tsx"),
                    Box::<str>::from("src/button.test.tsx"),
                ]),
            }]),
        });

        // The JSON shape is consumed by JavaScript tooling, so snake_case keys
        // would be a breaking cross-language contract.
        let json = serde_json::to_string(&result).unwrap();

        assert!(json.contains("changedFile"));
        assert!(json.contains("testFile"));
        assert!(!json.contains("changed_file"));
        assert!(!json.contains("test_file"));
    }

    #[test]
    fn command_result_deserialization_rejects_unexpected_fields() {
        const JSON: &str = r#"{
  "status": "partial",
  "tests": ["src/button.test.tsx"],
  "reasons": [],
  "unexpectedField": true
}"#;

        let result = serde_json::from_str::<super::CommandResult>(JSON);

        assert!(result.is_err());
    }

    #[test]
    fn graph_and_explain_outputs_use_typed_camel_case_contracts() {
        let graph = super::GraphResult {
            nodes: Box::from([super::GraphNode {
                path: Box::<str>::from("src/button.test.tsx"),
                kind: super::GraphNodeKind::Test,
            }]),
            edges: Box::from([super::GraphEdge {
                from: Box::<str>::from("src/button.test.tsx"),
                to: Box::<str>::from("src/button.tsx"),
            }]),
        };
        let explanation = super::ExplainResult {
            test_file: Box::<str>::from("src/button.test.tsx"),
            reasons: Box::from([super::ReasonChain {
                changed_file: Box::<str>::from("src/button.tsx"),
                test_file: Box::<str>::from("src/button.test.tsx"),
                path: Box::from([
                    Box::<str>::from("src/button.tsx"),
                    Box::<str>::from("src/button.test.tsx"),
                ]),
            }]),
        };

        // Graph and explain output are separate command contracts, but both
        // still need the same JavaScript-friendly key shape as selection JSON.
        let graph_json = serde_json::to_string(&graph).unwrap();
        let explain_json = serde_json::to_string(&explanation).unwrap();

        assert!(graph_json.contains("src/button.test.tsx"));
        assert!(explain_json.contains("testFile"));
        assert!(!explain_json.contains("test_file"));
    }
}
