//! Filesystem capability traits and production adapter placeholders.

use crate::failure;
use crate::roots;

/// Metadata snapshot for a root-relative file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileMetadata {
    /// Size in bytes reported by the filesystem.
    pub byte_len: u64,
    /// Whether the path points at a directory.
    pub is_directory: bool,
}

/// File content reader needed by config loading and parsing.
pub trait FileReader {
    /// Reads UTF-8 text from a root-relative path.
    ///
    /// # Errors
    ///
    /// Returns an error when the adapter cannot read or decode the path.
    fn read_text(&self, path: &roots::RootRelativePath) -> failure::Result<Box<str>>;
}

/// File metadata reader needed by discovery and diagnostics.
pub trait FileMetadataReader {
    /// Reads metadata for a root-relative path.
    ///
    /// # Errors
    ///
    /// Returns an error when metadata cannot be read.
    fn metadata(&self, path: &roots::RootRelativePath) -> failure::Result<FileMetadata>;
}

/// File existence capability needed by import resolution.
pub trait FileExistence {
    /// Reports whether a root-relative path exists.
    ///
    /// # Errors
    ///
    /// Returns an error when existence cannot be determined.
    fn exists(&self, path: &roots::RootRelativePath) -> failure::Result<bool>;
}

/// Filesystem directory walking capability used by discovery.
pub trait DirectoryWalker {
    /// Returns candidate source paths in deterministic order.
    ///
    /// # Errors
    ///
    /// Returns an error when walking fails.
    fn source_paths(&self) -> failure::Result<Box<[roots::RootRelativePath]>>;
}

/// Filesystem output capability used by graph or report writers.
pub trait FileWriter {
    /// Writes UTF-8 text to a root-relative path.
    ///
    /// # Errors
    ///
    /// Returns an error when the adapter cannot write the requested path.
    fn write_text(&self, path: &roots::RootRelativePath, content: &str) -> failure::Result<()>;
}

/// Production filesystem adapter composed at the application edge.
#[derive(Clone, Debug)]
pub struct RealFileSystem {
    root: Box<str>,
}

impl RealFileSystem {
    /// Creates a filesystem adapter rooted at a repository path.
    #[must_use]
    pub const fn for_root(root: Box<str>) -> Self {
        Self { root }
    }

    /// Returns the repository root represented by this adapter.
    #[must_use]
    pub fn root(&self) -> &str {
        self.root.as_ref()
    }
}
