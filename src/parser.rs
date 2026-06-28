//! TypeScript and TSX import extraction contracts.

use crate::failure;
use crate::roots;

/// File content capability consumed by parser input loading.
pub trait SourceReader {
    /// Reads source text for a root-relative path.
    ///
    /// # Errors
    ///
    /// Returns an error when source text cannot be loaded.
    fn source_text(&self, path: &roots::RootRelativePath) -> failure::Result<Box<str>>;
}

/// Request object for parsing a single source file.
pub struct Request<R> {
    /// Reader used to load source text.
    pub reader: R,
    /// Path whose imports should be extracted.
    pub path: roots::RootRelativePath,
}

/// Parsed import records from one source file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Imports {
    /// Static import and re-export specifiers.
    pub static_specifiers: Box<[roots::ImportSpecifier]>,
    /// String-literal dynamic import specifiers.
    pub dynamic_specifiers: Box<[roots::ImportSpecifier]>,
    /// Whether at least one dynamic import cannot be represented statically.
    pub has_unresolved_dynamic_import: bool,
}

/// Extracts import specifiers from TypeScript or TSX source.
///
/// # Errors
///
/// Returns an error when source text cannot be loaded or parsed.
pub fn imports<R>(_request: Request<R>) -> failure::Result<Imports>
where
    R: SourceReader,
{
    unimplemented!()
}
