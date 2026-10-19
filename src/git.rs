//! Git changed-file detection contracts for binary-level integration.

use std::path;
use std::process;

use globset::{Glob, GlobSet, GlobSetBuilder};

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

    /// Runs Git diff name-status for staged and unstaged tracked worktree changes.
    ///
    /// # Errors
    ///
    /// Returns an error when Git cannot compute the worktree diff.
    fn diff_worktree_name_status(&self) -> failure::Result<Box<str>>;

    /// Lists untracked files in the repository so out-of-workspace changes can fail closed.
    ///
    /// # Errors
    ///
    /// Returns an error when Git cannot list untracked files.
    fn untracked_files(&self) -> failure::Result<Box<str>>;

    /// Reads a UTF-8 file at a Git revision.
    ///
    /// # Errors
    ///
    /// Returns an error when Git cannot read an existing file.
    fn file_at_revision(
        &self,
        revision: &str,
        path: &roots::RootRelativePath,
    ) -> failure::Result<Option<Box<str>>>;
}

/// Process-backed repository adapter for real Git invocations.
#[derive(Clone, Debug)]
pub struct ProcessRepository {
    working_directory: Box<str>,
    ignored_out_of_workspace: GlobSet,
}

impl ProcessRepository {
    /// Creates a Git adapter rooted at the invocation workspace.
    #[must_use]
    pub const fn for_root(working_directory: Box<str>) -> Self {
        Self {
            working_directory,
            ignored_out_of_workspace: GlobSet::empty(),
        }
    }

    /// Creates a Git adapter that ignores configured repository-root paths
    /// outside a subdirectory invocation workspace.
    ///
    /// # Errors
    ///
    /// Returns an error when an ignore glob is invalid.
    pub fn for_root_ignoring(
        working_directory: Box<str>,
        patterns: &[crate::settings::Pattern],
    ) -> failure::Result<Self> {
        let mut builder = GlobSetBuilder::new();
        for pattern in patterns {
            let glob = Glob::new(pattern.as_str()).map_err(|error| failure::AppError::Config {
                message: format!("invalid out-of-workspace ignore glob: {error}").into_boxed_str(),
            })?;
            builder.add(glob);
        }
        let ignored_out_of_workspace =
            builder.build().map_err(|error| failure::AppError::Config {
                message: format!("invalid out-of-workspace ignore globs: {error}").into_boxed_str(),
            })?;
        Ok(Self {
            working_directory,
            ignored_out_of_workspace,
        })
    }
}

impl ProcessRepository {
    fn workspace_prefix(&self) -> failure::Result<Box<str>> {
        let output = process::Command::new("git")
            .arg("-C")
            .arg(path::Path::new(self.working_directory.as_ref()))
            .arg("rev-parse")
            .arg("--show-prefix")
            .output()
            .map_err(|error| failure::AppError::Git {
                message: format!("failed to identify Git workspace prefix: {error}")
                    .into_boxed_str(),
            })?;
        if !output.status.success() {
            return Err(failure::AppError::Git {
                message: git_failure_message(output.stderr.as_slice()),
            });
        }
        let prefix = String::from_utf8(output.stdout).map_err(|error| failure::AppError::Git {
            message: format!("Git workspace prefix was not UTF-8: {error}").into_boxed_str(),
        })?;
        let prefix = prefix.trim_end_matches(['\r', '\n']);
        if prefix.is_empty() {
            Ok(Box::from(""))
        } else {
            Ok(format!("{}/", prefix.trim_end_matches('/')).into_boxed_str())
        }
    }

    fn workspace_relative_name_status(&self, output: &str) -> failure::Result<Box<str>> {
        let prefix = self.workspace_prefix()?;
        let mut normalized = String::new();
        for line in output.lines() {
            if line.is_empty() {
                continue;
            }
            let (status, paths) = line.split_once('\t').ok_or_else(|| git_parse_error(line))?;
            let mut normalized_paths = Vec::<&str>::new();
            for path in paths.split('\t') {
                let workspace_path = self.workspace_relative_path(path, prefix.as_ref())?;
                if !workspace_path.is_empty() {
                    normalized_paths.push(workspace_path);
                }
            }
            if normalized_paths.is_empty() {
                continue;
            }
            let normalized_status = if normalized_paths.len() == paths.split('\t').count() {
                status.to_owned()
            } else if status.starts_with('R') || status.starts_with('C') {
                // A rename crossing the workspace boundary becomes an add or
                // deletion inside this workspace, respectively.
                if paths
                    .split('\t')
                    .next()
                    .is_some_and(|old| !prefix.is_empty() && !old.starts_with(prefix.as_ref()))
                {
                    "A".to_owned()
                } else {
                    "D".to_owned()
                }
            } else {
                status.to_owned()
            };
            normalized.push_str(&normalized_status);
            for path in normalized_paths {
                normalized.push('\t');
                normalized.push_str(path);
            }
            normalized.push('\n');
        }

        Ok(normalized.into_boxed_str())
    }

    fn workspace_relative_untracked_files(&self, output: &str) -> failure::Result<Box<str>> {
        let prefix = self.workspace_prefix()?;
        let mut normalized = String::new();
        for path in output.lines().filter(|path| !path.is_empty()) {
            let workspace_path = self.workspace_relative_path(path, prefix.as_ref())?;
            if workspace_path.is_empty() {
                continue;
            }
            normalized.push_str(workspace_path);
            normalized.push('\n');
        }

        Ok(normalized.into_boxed_str())
    }

    fn workspace_relative_path<'a>(&self, path: &'a str, prefix: &str) -> failure::Result<&'a str> {
        let workspace_path = if prefix.is_empty() {
            path
        } else {
            match path.strip_prefix(prefix) {
                Some(workspace_path) => workspace_path,
                None if self.ignored_out_of_workspace.is_match(path) => return Ok(""),
                None => {
                    return Err(failure::AppError::Git {
                        message: format!(
                            "changed path `{path}` is outside the invocation workspace `{}`",
                            prefix.trim_end_matches('/')
                        )
                        .into_boxed_str(),
                    });
                }
            }
        };
        if workspace_path.is_empty() {
            return Err(git_parse_error(path));
        }

        Ok(workspace_path)
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

        let output = String::from_utf8(output.stdout).map_err(|error| failure::AppError::Git {
            message: format!("git diff output was not UTF-8: {error}").into_boxed_str(),
        })?;
        self.workspace_relative_name_status(&output)
    }

    fn diff_worktree_name_status(&self) -> failure::Result<Box<str>> {
        let output = process::Command::new("git")
            .arg("-C")
            .arg(path::Path::new(self.working_directory.as_ref()))
            .arg("diff")
            .arg("--name-status")
            .arg("--find-renames")
            .arg("--diff-filter=ACMRTD")
            .arg("HEAD")
            .output()
            .map_err(|error| failure::AppError::Git {
                message: format!("failed to execute git worktree diff: {error}").into_boxed_str(),
            })?;

        if !output.status.success() {
            return Err(failure::AppError::Git {
                message: git_failure_message(output.stderr.as_slice()),
            });
        }

        let output = String::from_utf8(output.stdout).map_err(|error| failure::AppError::Git {
            message: format!("git worktree diff output was not UTF-8: {error}").into_boxed_str(),
        })?;
        self.workspace_relative_name_status(&output)
    }

    fn untracked_files(&self) -> failure::Result<Box<str>> {
        let output = process::Command::new("git")
            .arg("-C")
            .arg(path::Path::new(self.working_directory.as_ref()))
            .arg("ls-files")
            .arg("--others")
            .arg("--exclude-standard")
            .arg("--full-name")
            .arg("--")
            .arg(":(top)")
            .output()
            .map_err(|error| failure::AppError::Git {
                message: format!("failed to execute git ls-files: {error}").into_boxed_str(),
            })?;

        if !output.status.success() {
            return Err(failure::AppError::Git {
                message: git_failure_message(output.stderr.as_slice()),
            });
        }

        let output = String::from_utf8(output.stdout).map_err(|error| failure::AppError::Git {
            message: format!("git ls-files output was not UTF-8: {error}").into_boxed_str(),
        })?;
        self.workspace_relative_untracked_files(&output)
    }

    fn file_at_revision(
        &self,
        revision: &str,
        path: &roots::RootRelativePath,
    ) -> failure::Result<Option<Box<str>>> {
        // The `./` prefix resolves the path relative to the invocation
        // directory (set via `-C`), matching paths normalized to the invocation
        // workspace. Without it, `git show <rev>:<path>` resolves against the repository root and
        // silently fails to read files when run from a subdirectory workspace.
        let object = format!("{revision}:./{}", path.as_str());
        let output = process::Command::new("git")
            .arg("-C")
            .arg(path::Path::new(self.working_directory.as_ref()))
            .arg("show")
            .arg(object)
            .output()
            .map_err(|error| failure::AppError::Git {
                message: format!("failed to execute git show: {error}").into_boxed_str(),
            })?;

        if !output.status.success() {
            return Ok(None);
        }

        // Binary metadata (for example a binary `bun.lockb` lockfile) is not
        // valid UTF-8 and cannot be parsed as text. Treat it as unavailable so
        // scoping falls back to the fail-closed global invalidator instead of
        // aborting the whole run.
        Ok(String::from_utf8(output.stdout)
            .ok()
            .map(String::into_boxed_str))
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
    /// Include staged, unstaged, and untracked worktree changes.
    pub worktree: bool,
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
        worktree,
    } = request;
    let output = repository.diff_name_status(&DiffRequest { base, head })?;
    let mut files = Vec::<ChangedFile>::new();
    push_name_status_files(&mut files, output.as_ref())?;
    if worktree {
        let worktree_output = repository.diff_worktree_name_status()?;
        push_name_status_files(&mut files, worktree_output.as_ref())?;
        let untracked_output = repository.untracked_files()?;
        push_untracked_files(&mut files, untracked_output.as_ref())?;
    }

    files.sort_by(compare_changed_files);
    files.dedup_by(|left, right| {
        left.path == right.path && left.previous_path == right.previous_path
    });
    Ok(ChangeSet {
        files: files.into_boxed_slice(),
    })
}

fn push_name_status_files(files: &mut Vec<ChangedFile>, output: &str) -> failure::Result<()> {
    for line in output.lines() {
        if line.is_empty() {
            continue;
        }
        files.push(changed_file_from_line(line)?);
    }

    Ok(())
}

fn push_untracked_files(files: &mut Vec<ChangedFile>, output: &str) -> failure::Result<()> {
    for line in output.lines() {
        if line.is_empty() {
            continue;
        }
        files.push(ChangedFile {
            status: ChangedFileStatus::Added,
            path: roots::RootRelativePath::try_from(line)?,
            previous_path: None,
        });
    }

    Ok(())
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
        .then_with(|| dedup_rank(left.status).cmp(&dedup_rank(right.status)))
}

/// Ranks statuses so the most authoritative record survives dedup for one path.
///
/// A worktree deletion must win over a committed modification of the same path:
/// the file is gone in the working tree, so selection must be able to fail
/// closed (deleted source files force the full suite).
const fn dedup_rank(status: ChangedFileStatus) -> u8 {
    match status {
        ChangedFileStatus::Deleted => 0,
        ChangedFileStatus::Added
        | ChangedFileStatus::Copied
        | ChangedFileStatus::Modified
        | ChangedFileStatus::Renamed
        | ChangedFileStatus::TypeChanged => 1,
    }
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
        worktree_output: Box<str>,
        untracked_output: Box<str>,
    }

    impl super::GitRepository for FixtureRepository {
        fn diff_name_status(&self, _request: &super::DiffRequest) -> failure::Result<Box<str>> {
            Ok(self.output.clone())
        }

        fn diff_worktree_name_status(&self) -> failure::Result<Box<str>> {
            Ok(self.worktree_output.clone())
        }

        fn untracked_files(&self) -> failure::Result<Box<str>> {
            Ok(self.untracked_output.clone())
        }

        fn file_at_revision(
            &self,
            _revision: &str,
            _path: &roots::RootRelativePath,
        ) -> failure::Result<Option<Box<str>>> {
            Ok(None)
        }
    }

    fn path(value: &str) -> roots::RootRelativePath {
        roots::RootRelativePath::try_from(value).unwrap()
    }

    #[test]
    fn configured_out_of_workspace_paths_are_ignored_but_code_paths_still_fail_closed() {
        let patterns = [crate::settings::Pattern::try_from("**/*.md").unwrap()];
        let repository =
            super::ProcessRepository::for_root_ignoring(Box::<str>::from("."), &patterns).unwrap();

        assert_eq!(
            repository
                .workspace_relative_path(".claude/skills/example/SKILL.md", "apps/web/")
                .unwrap(),
            "",
        );
        assert!(
            repository
                .workspace_relative_path("packages/core/src/index.ts", "apps/web/")
                .is_err()
        );
    }

    #[test]
    fn parses_name_status_records_for_modified_renamed_and_deleted_paths() {
        let result = super::changed_files(super::ChangesRequest {
            repository: FixtureRepository {
                output: Box::<str>::from(
                    "R100\tsrc/old.ts\tsrc/new.ts\nD\tsrc/removed.ts\nM\tsrc/value.ts\n",
                ),
                worktree_output: Box::<str>::from(""),
                untracked_output: Box::<str>::from(""),
            },
            base: Box::<str>::from("origin/main"),
            head: Box::<str>::from("HEAD"),
            worktree: false,
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

    #[test]
    fn worktree_deletion_wins_over_committed_modification_for_same_path() {
        let result = super::changed_files(super::ChangesRequest {
            repository: FixtureRepository {
                // The committed range modified the file, but the working tree
                // deleted it; the final worktree state is what a runner sees.
                output: Box::<str>::from("M\tsrc/value.ts\n"),
                worktree_output: Box::<str>::from("D\tsrc/value.ts\n"),
                untracked_output: Box::<str>::from(""),
            },
            base: Box::<str>::from("origin/main"),
            head: Box::<str>::from("HEAD"),
            worktree: true,
        })
        .unwrap();

        // Desired: the deletion survives so selection can fail closed via
        // DeletedSourceFile. The status-blind dedup keeps the committed
        // Modified record instead, so this assertion fails on current code.
        let survivor = result
            .files
            .iter()
            .find(|change| change.path == path("src/value.ts"))
            .unwrap();
        assert!(
            survivor.status == super::ChangedFileStatus::Deleted,
            "worktree deletion must survive dedup, got {:?}",
            survivor.status,
        );
    }

    #[test]
    fn worktree_mode_combines_committed_tracked_and_untracked_changes() {
        let result = super::changed_files(super::ChangesRequest {
            repository: FixtureRepository {
                output: Box::<str>::from("M\tsrc/committed.ts\n"),
                worktree_output: Box::<str>::from("M\tsrc/dirty.ts\nD\tsrc/deleted.ts\n"),
                untracked_output: Box::<str>::from("src/new.test.ts\n"),
            },
            base: Box::<str>::from("origin/main"),
            head: Box::<str>::from("HEAD"),
            worktree: true,
        })
        .unwrap();

        assert_eq!(
            result.files,
            Box::from([
                super::ChangedFile {
                    status: super::ChangedFileStatus::Modified,
                    path: path("src/committed.ts"),
                    previous_path: None,
                },
                super::ChangedFile {
                    status: super::ChangedFileStatus::Deleted,
                    path: path("src/deleted.ts"),
                    previous_path: None,
                },
                super::ChangedFile {
                    status: super::ChangedFileStatus::Modified,
                    path: path("src/dirty.ts"),
                    previous_path: None,
                },
                super::ChangedFile {
                    status: super::ChangedFileStatus::Added,
                    path: path("src/new.test.ts"),
                    previous_path: None,
                },
            ]),
        );
    }
}
