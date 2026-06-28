//! Import resolution contracts for relative paths, extensions, index files, and aliases.

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
pub fn import<C, P>(_request: ResolveRequest<C, P>) -> failure::Result<Outcome>
where
    C: settings::View,
    P: FileExistence,
{
    unimplemented!()
}
