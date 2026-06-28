//! Deterministic dependency graph contracts and traversal views.

use std::collections::BTreeMap;

use crate::failure;
use crate::roots;

/// Reverse dependency access consumed by affected selection and tracing.
pub trait GraphView {
    /// Returns direct reverse dependents for a graph node in stable order.
    #[must_use]
    fn reverse_dependents(&self, path: &roots::RootRelativePath) -> &[roots::RootRelativePath];

    /// Returns direct dependencies for a graph node in stable order.
    #[must_use]
    fn dependencies(&self, path: &roots::RootRelativePath) -> &[roots::RootRelativePath];
}

/// Import source used by graph construction.
pub trait ImportResolver {
    /// Returns resolved dependencies for a file in stable order.
    ///
    /// # Errors
    ///
    /// Returns an error when imports cannot be parsed or resolved.
    fn dependencies_for(
        &self,
        path: &roots::RootRelativePath,
    ) -> failure::Result<Box<[roots::RootRelativePath]>>;
}

/// Request object for graph construction.
pub struct GraphBuildRequest<I> {
    /// Import source used for edge construction.
    pub imports: I,
    /// Source and test files included in the graph.
    pub files: Box<[roots::RootRelativePath]>,
}

/// Deterministic directed dependency graph.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DependencyGraph {
    dependencies: BTreeMap<roots::RootRelativePath, Box<[roots::RootRelativePath]>>,
    reverse_dependencies: BTreeMap<roots::RootRelativePath, Box<[roots::RootRelativePath]>>,
    empty: Box<[roots::RootRelativePath]>,
}

impl GraphView for DependencyGraph {
    fn reverse_dependents(&self, path: &roots::RootRelativePath) -> &[roots::RootRelativePath] {
        self.reverse_dependencies
            .get(path)
            .map_or_else(|| self.empty.as_ref(), Box::as_ref)
    }

    fn dependencies(&self, path: &roots::RootRelativePath) -> &[roots::RootRelativePath] {
        self.dependencies
            .get(path)
            .map_or_else(|| self.empty.as_ref(), Box::as_ref)
    }
}

/// Builds a deterministic dependency graph from resolved imports.
///
/// # Errors
///
/// Returns an error when dependency extraction fails for any graph file.
pub fn build<I>(_request: GraphBuildRequest<I>) -> failure::Result<DependencyGraph>
where
    I: ImportResolver,
{
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::failure;
    use crate::roots;

    #[derive(Clone, Debug)]
    struct FixtureImports {
        edges: BTreeMap<roots::RootRelativePath, Box<[roots::RootRelativePath]>>,
        empty: Box<[roots::RootRelativePath]>,
    }

    impl super::ImportResolver for FixtureImports {
        fn dependencies_for(
            &self,
            path: &roots::RootRelativePath,
        ) -> failure::Result<Box<[roots::RootRelativePath]>> {
            Ok(self
                .edges
                .get(path)
                .map_or_else(|| self.empty.clone(), Clone::clone))
        }
    }

    fn path(value: &str) -> roots::RootRelativePath {
        roots::RootRelativePath::try_from(value).unwrap()
    }

    #[test]
    #[should_panic(expected = "not implemented")]
    fn builds_forward_edges_and_reverse_edges_for_importers_and_imports() {
        let file_a = path("src/file-a.ts");
        let file_b = path("src/file-b.ts");
        let imports = FixtureImports {
            edges: BTreeMap::from([(file_a.clone(), Box::from([file_b.clone()]))]),
            empty: Box::from([]),
        };
        let request = super::GraphBuildRequest {
            imports,
            files: Box::from([file_a.clone(), file_b.clone()]),
        };

        // A single A -> B import is the smallest fixture that proves both graph
        // directions are populated for selectors and explain output.
        let graph = super::build(request).unwrap();

        assert_eq!(
            super::GraphView::dependencies(&graph, &file_a),
            std::slice::from_ref(&file_b),
        );
        assert_eq!(
            super::GraphView::reverse_dependents(&graph, &file_b),
            &[file_a]
        );
    }
}
