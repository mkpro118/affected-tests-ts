//! Git changed-file detection contracts for binary-level integration.

use std::path;
use std::process;

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

/// Process-backed repository adapter for real Git invocations.
#[derive(Clone, Debug)]
pub struct ProcessRepository {
    working_directory: Box<str>,
}

impl ProcessRepository {
    /// Creates a Git adapter rooted at the invocation workspace.
    #[must_use]
    pub const fn for_root(working_directory: Box<str>) -> Self {
        Self { working_directory }
    }
}

impl GitRepository for ProcessRepository {
    fn diff_name_status(&self, request: &DiffRequest) -> failure::Result<Box<str>> {
        let range = format!("{}...{}", request.base, request.head);
        let output = process::Command::new("git")
            .arg("-C")
            .arg(path::Path::new(self.working_directory.as_ref()))
            .arg("diff")
            .arg("--name-status")
            .arg("--relative")
            .arg("--find-renames")
            .arg("--diff-filter=ACMRTD")
            .arg(range)
            .output()
            .map_err(|error| failure::AppError::Git {
                message: format!("failed to execute git diff: {error}").into_boxed_str(),
            })?;

        if !output.status.success() {
            return Err(failure::AppError::Git {
                message: git_failure_message(output.stderr.as_slice()),
            });
        }

        String::from_utf8(output.stdout)
            .map(String::into_boxed_str)
            .map_err(|error| failure::AppError::Git {
                message: format!("git diff output was not UTF-8: {error}").into_boxed_str(),
            })
    }
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
pub fn changed_files<R>(request: ChangesRequest<R>) -> failure::Result<ChangeSet>
where
    R: GitRepository,
{
    let ChangesRequest {
        repository,
        base,
        head,
    } = request;
    let output = repository.diff_name_status(&DiffRequest { base, head })?;
    let mut files = Vec::<ChangedFile>::new();

    for line in output.lines() {
        if line.is_empty() {
            continue;
        }
        files.push(changed_file_from_line(line)?);
    }

    files.sort_by(compare_changed_files);
    Ok(ChangeSet {
        files: files.into_boxed_slice(),
    })
}

fn changed_file_from_line(line: &str) -> failure::Result<ChangedFile> {
    let mut fields = line.split('\t');
    let status_text = fields.next().ok_or_else(|| git_parse_error(line))?;
    let status = status_from_text(status_text)?;

    match status {
        ChangedFileStatus::Copied | ChangedFileStatus::Renamed => {
            let previous_path = fields.next().ok_or_else(|| git_parse_error(line))?;
            let current_path = fields.next().ok_or_else(|| git_parse_error(line))?;
            if fields.next().is_some() {
                return Err(git_parse_error(line));
            }

            Ok(ChangedFile {
                status,
                path: roots::RootRelativePath::try_from(current_path)?,
                previous_path: Some(roots::RootRelativePath::try_from(previous_path)?),
            })
        }
        ChangedFileStatus::Added
        | ChangedFileStatus::Modified
        | ChangedFileStatus::TypeChanged
        | ChangedFileStatus::Deleted => {
            let current_path = fields.next().ok_or_else(|| git_parse_error(line))?;
            if fields.next().is_some() {
                return Err(git_parse_error(line));
            }

            Ok(ChangedFile {
                status,
                path: roots::RootRelativePath::try_from(current_path)?,
                previous_path: None,
            })
        }
    }
}

fn status_from_text(status_text: &str) -> failure::Result<ChangedFileStatus> {
    match status_text.chars().next() {
        Some('A') => Ok(ChangedFileStatus::Added),
        Some('C') => Ok(ChangedFileStatus::Copied),
        Some('M') => Ok(ChangedFileStatus::Modified),
        Some('R') => Ok(ChangedFileStatus::Renamed),
        Some('T') => Ok(ChangedFileStatus::TypeChanged),
        Some('D') => Ok(ChangedFileStatus::Deleted),
        Some(_) | None => Err(failure::AppError::Git {
            message: format!("unsupported git name-status record `{status_text}`").into_boxed_str(),
        }),
    }
}

fn compare_changed_files(left: &ChangedFile, right: &ChangedFile) -> std::cmp::Ordering {
    left.path
        .cmp(&right.path)
        .then_with(|| left.previous_path.cmp(&right.previous_path))
}

fn git_parse_error(line: &str) -> failure::AppError {
    failure::AppError::Git {
        message: format!("invalid git name-status line `{line}`").into_boxed_str(),
    }
}

fn git_failure_message(stderr: &[u8]) -> Box<str> {
    let message = String::from_utf8_lossy(stderr);
    if message.trim().is_empty() {
        Box::<str>::from("git diff failed without stderr")
    } else {
        Box::<str>::from(message.trim())
    }
}

#[cfg(test)]
mod tests {
    use crate::failure;
    use crate::roots;

    #[derive(Clone, Debug)]
    struct FixtureRepository {
        output: Box<str>,
    }

    impl super::GitRepository for FixtureRepository {
        fn diff_name_status(&self, _request: &super::DiffRequest) -> failure::Result<Box<str>> {
            Ok(self.output.clone())
        }
    }

    fn path(value: &str) -> roots::RootRelativePath {
        roots::RootRelativePath::try_from(value).unwrap()
    }

    #[test]
    fn parses_name_status_records_for_modified_renamed_and_deleted_paths() {
        let result = super::changed_files(super::ChangesRequest {
            repository: FixtureRepository {
                output: Box::<str>::from(
                    "R100\tsrc/old.ts\tsrc/new.ts\nD\tsrc/removed.ts\nM\tsrc/value.ts\n",
                ),
            },
            base: Box::<str>::from("origin/main"),
            head: Box::<str>::from("HEAD"),
        })
        .unwrap();

        // This fixture mirrors real `git diff --name-status --find-renames`
        // output so parsing stays isolated from process execution.
        assert_eq!(
            result.files,
            Box::from([
                super::ChangedFile {
                    status: super::ChangedFileStatus::Renamed,
                    path: path("src/new.ts"),
                    previous_path: Some(path("src/old.ts")),
                },
                super::ChangedFile {
                    status: super::ChangedFileStatus::Deleted,
                    path: path("src/removed.ts"),
                    previous_path: None,
                },
                super::ChangedFile {
                    status: super::ChangedFileStatus::Modified,
                    path: path("src/value.ts"),
                    previous_path: None,
                },
            ]),
        );
    }
}
