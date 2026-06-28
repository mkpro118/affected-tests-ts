//! Git changed-file detection contracts for binary-level integration.

use crate::failure;
use crate::roots;

/// Changed-file status reported by Git name-status output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangedFileStatus {
    /// File was added.
    Added,
    /// File was copied.
    Copied,
    /// File was modified.
    Modified,
    /// File was renamed.
    Renamed,
    /// File type changed.
    TypeChanged,
    /// File was deleted.
    Deleted,
}

/// One changed path reported by Git.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangedFile {
    /// Status associated with the current path.
    pub status: ChangedFileStatus,
    /// Current root-relative path when the file still exists.
    pub path: roots::RootRelativePath,
    /// Previous path for rename or copy records.
    pub previous_path: Option<roots::RootRelativePath>,
}

/// Stable changed-file collection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangeSet {
    /// Sorted changed files.
    pub files: Box<[ChangedFile]>,
}

/// Read-only changed-file view consumed by selection logic.
pub trait ChangeSetView {
    /// Returns sorted changed files.
    #[must_use]
    fn files(&self) -> &[ChangedFile];
}

impl ChangeSetView for ChangeSet {
    fn files(&self) -> &[ChangedFile] {
        self.files.as_ref()
    }
}

/// Git command capability consumed by changed-file detection.
pub trait GitRepository {
    /// Runs Git diff name-status for a base/head pair.
    ///
    /// # Errors
    ///
    /// Returns an error when Git cannot compute the requested range.
    fn diff_name_status(&self, request: &DiffRequest) -> failure::Result<Box<str>>;
}

/// Request object for raw Git diff execution.
pub struct DiffRequest {
    /// Base revision for three-dot diff.
    pub base: Box<str>,
    /// Head revision for three-dot diff.
    pub head: Box<str>,
}

/// Request object for typed changed-file detection.
pub struct ChangesRequest<R> {
    /// Git repository adapter.
    pub repository: R,
    /// Base revision for three-dot diff.
    pub base: Box<str>,
    /// Head revision for three-dot diff.
    pub head: Box<str>,
}

/// Detects typed changed files from Git.
///
/// # Errors
///
/// Returns an error when Git fails or name-status output cannot be parsed.
pub fn changed_files<R>(_request: ChangesRequest<R>) -> failure::Result<ChangeSet>
where
    R: GitRepository,
{
    unimplemented!()
}
