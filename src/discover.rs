//! Source and test file discovery contracts.

use crate::failure;
use crate::roots;
use crate::settings;

/// Filesystem capability consumed by discovery.
pub trait SourceDiscoverer {
    /// Returns candidate paths before config classification.
    ///
    /// # Errors
    ///
    /// Returns an error when the backing catalog cannot be read.
    fn candidate_paths(&self) -> failure::Result<Box<[roots::RootRelativePath]>>;
}

/// Request object for file discovery.
pub struct Request<C, F> {
    /// Configuration view used for include and exclude decisions.
    pub config: C,
    /// Filesystem catalog used to enumerate files.
    pub file_catalog: F,
}

/// Discovered source and test paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Files {
    /// Stable sorted source files.
    pub sources: Box<[roots::RootRelativePath]>,
    /// Stable sorted test files.
    pub tests: Box<[roots::RootRelativePath]>,
}

/// Discovers source and test files from configured roots.
///
/// # Errors
///
/// Returns an error when file enumeration or pattern matching fails.
pub fn files<C, F>(_request: Request<C, F>) -> failure::Result<Files>
where
    C: settings::View,
    F: SourceDiscoverer,
{
    unimplemented!()
}
