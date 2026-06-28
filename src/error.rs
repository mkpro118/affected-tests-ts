//! Error types shared across CLI orchestration and pure selection logic.

/// Crate-local result type for fallible operations.
pub type Result<T, E = AppError> = std::result::Result<T, E>;

/// Application error variants surfaced by the binary and output contracts.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// A path failed root-relative normalization.
    #[error("invalid root-relative path: {path}")]
    InvalidRootRelativePath {
        /// Path text provided at the boundary.
        path: Box<str>,
    },
    /// An import specifier could not be represented as a domain value.
    #[error("invalid import specifier: {specifier}")]
    InvalidImportSpecifier {
        /// Import text provided by the parser.
        specifier: Box<str>,
    },
    /// A configuration file failed to load or parse.
    #[error("configuration error: {message}")]
    Config {
        /// Human-readable configuration failure.
        message: Box<str>,
    },
    /// Source parsing failed before imports could be extracted.
    #[error("parse error: {message}")]
    Parse {
        /// Human-readable parser failure.
        message: Box<str>,
    },
    /// A filesystem adapter failed.
    #[error("filesystem error: {message}")]
    FileSystem {
        /// Human-readable filesystem failure.
        message: Box<str>,
    },
    /// A Git adapter failed.
    #[error("git error: {message}")]
    Git {
        /// Human-readable Git failure.
        message: Box<str>,
    },
    /// Graph construction or traversal failed.
    #[error("graph error: {message}")]
    Graph {
        /// Human-readable graph failure.
        message: Box<str>,
    },
    /// A non-literal dynamic import requires the full suite.
    #[error("unknown dynamic import in {importer}")]
    UnknownDynamicImport {
        /// File containing the unresolved dynamic import.
        importer: crate::roots::RootRelativePath,
    },
    /// A local import could not be resolved safely.
    #[error("unresolved local import `{specifier}` in {importer}")]
    UnresolvedLocalImport {
        /// File containing the unresolved import.
        importer: crate::roots::RootRelativePath,
        /// Import specifier text that could not be resolved.
        specifier: crate::roots::ImportSpecifier,
    },
    /// Rendering failed after selection completed.
    #[error("output error: {message}")]
    Output {
        /// Human-readable rendering failure.
        message: Box<str>,
    },
}
