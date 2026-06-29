//! Filesystem capability traits and production adapters.

use std::path;

use crate::discovery;
use crate::failure;
use crate::modules;
use crate::parser;
use crate::roots;
use crate::settings;

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

    fn absolute_path(&self, path: &roots::RootRelativePath) -> path::PathBuf {
        path::Path::new(self.root.as_ref()).join(path.as_str())
    }
}

impl FileReader for RealFileSystem {
    fn read_text(&self, path: &roots::RootRelativePath) -> failure::Result<Box<str>> {
        std::fs::read_to_string(self.absolute_path(path))
            .map(String::into_boxed_str)
            .map_err(|error| failure::AppError::FileSystem {
                message: format!("failed to read `{}`: {error}", path.as_str()).into_boxed_str(),
            })
    }
}

impl FileMetadataReader for RealFileSystem {
    fn metadata(&self, path: &roots::RootRelativePath) -> failure::Result<FileMetadata> {
        let metadata = std::fs::metadata(self.absolute_path(path)).map_err(|error| {
            failure::AppError::FileSystem {
                message: format!("failed to stat `{}`: {error}", path.as_str()).into_boxed_str(),
            }
        })?;

        Ok(FileMetadata {
            byte_len: metadata.len(),
            is_directory: metadata.is_dir(),
        })
    }
}

impl FileExistence for RealFileSystem {
    fn exists(&self, path: &roots::RootRelativePath) -> failure::Result<bool> {
        self.absolute_path(path)
            .try_exists()
            .map_err(|error| failure::AppError::FileSystem {
                message: format!("failed to probe `{}`: {error}", path.as_str()).into_boxed_str(),
            })
    }
}

impl DirectoryWalker for RealFileSystem {
    fn source_paths(&self) -> failure::Result<Box<[roots::RootRelativePath]>> {
        let root = path::Path::new(self.root.as_ref());
        let mut paths = Vec::<roots::RootRelativePath>::new();

        for entry in ignore::WalkBuilder::new(root)
            .standard_filters(true)
            .build()
        {
            let entry = entry.map_err(|error| failure::AppError::FileSystem {
                message: format!("failed to walk `{}`: {error}", self.root()).into_boxed_str(),
            })?;

            if entry
                .file_type()
                .is_some_and(|file_type| !file_type.is_file())
            {
                continue;
            }

            let relative =
                entry
                    .path()
                    .strip_prefix(root)
                    .map_err(|error| failure::AppError::FileSystem {
                        message: format!("failed to relativize walked path: {error}")
                            .into_boxed_str(),
                    })?;
            if relative.as_os_str().is_empty() {
                continue;
            }

            let relative_text = relative
                .to_str()
                .ok_or_else(|| failure::AppError::FileSystem {
                    message: format!("walked path is not UTF-8: {}", relative.display())
                        .into_boxed_str(),
                })?;
            paths.push(roots::RootRelativePath::try_from(relative_text)?);
        }

        paths.sort();
        paths.dedup();

        Ok(paths.into_boxed_slice())
    }
}

impl FileWriter for RealFileSystem {
    fn write_text(&self, path: &roots::RootRelativePath, content: &str) -> failure::Result<()> {
        std::fs::write(self.absolute_path(path), content).map_err(|error| {
            failure::AppError::FileSystem {
                message: format!("failed to write `{}`: {error}", path.as_str()).into_boxed_str(),
            }
        })
    }
}

impl discovery::SourceDiscoverer for RealFileSystem {
    fn candidate_paths(&self) -> failure::Result<Box<[roots::RootRelativePath]>> {
        DirectoryWalker::source_paths(self)
    }
}

impl parser::SourceReader for RealFileSystem {
    fn source_text(&self, path: &roots::RootRelativePath) -> failure::Result<Box<str>> {
        FileReader::read_text(self, path)
    }
}

impl settings::LoaderFileSystem for RealFileSystem {
    fn read_text(&self, path: &roots::RootRelativePath) -> failure::Result<Box<str>> {
        FileReader::read_text(self, path)
    }
}

impl modules::FileExistence for RealFileSystem {
    fn exists(&self, path: &roots::RootRelativePath) -> failure::Result<bool> {
        FileExistence::exists(self, path)
    }
}

#[cfg(test)]
mod tests {
    use crate::discovery;
    use crate::failure;
    use crate::modules;
    use crate::parser;
    use crate::roots;
    use crate::settings;

    #[derive(Clone, Debug)]
    struct FixtureConfig;

    impl settings::View for FixtureConfig {
        fn source_includes(&self) -> &[settings::Pattern] {
            &[]
        }

        fn excludes(&self) -> &[settings::Pattern] {
            &[]
        }

        fn test_patterns(&self) -> &[settings::TestFilePattern] {
            &[]
        }

        fn global_invalidators(&self) -> &[settings::Pattern] {
            &[]
        }

        fn dynamic_imports(&self) -> settings::UnknownDynamicImportBehavior {
            settings::UnknownDynamicImportBehavior::FailClosed
        }
    }

    #[derive(Clone, Debug)]
    struct FixtureResolver;

    impl modules::ImportResolver for FixtureResolver {
        fn resolve<C, P>(
            &self,
            request: modules::ResolveRequest<C, P>,
        ) -> failure::Result<modules::Outcome>
        where
            C: settings::View,
            P: modules::FileExistence,
        {
            modules::import(request)
        }
    }

    fn assert_real_filesystem_traits<T>()
    where
        T: Clone + discovery::SourceDiscoverer + parser::SourceReader + modules::FileExistence,
    {
    }

    fn assert_graph_import_source<C, R, M, P>()
    where
        C: Clone + settings::View,
        R: Clone + parser::SourceReader,
        M: Clone + modules::ImportResolver,
        P: Clone + modules::FileExistence,
        crate::dependencies::LocalImports<C, R, M, P>: crate::dependencies::ImportResolver,
    {
    }

    fn path(value: &str) -> roots::RootRelativePath {
        roots::RootRelativePath::try_from(value).unwrap()
    }

    #[test]
    fn real_filesystem_satisfies_discovery_parser_and_resolver_boundaries() {
        // Compile-time trait assertions prove the production adapter can be
        // composed where phase logic accepts module-local filesystem traits.
        assert_real_filesystem_traits::<super::RealFileSystem>();
        assert_graph_import_source::<
            FixtureConfig,
            super::RealFileSystem,
            FixtureResolver,
            super::RealFileSystem,
        >();
    }

    #[test]
    fn missing_paths_report_false_without_hiding_probe_errors() {
        let file_system = super::RealFileSystem::for_root(Box::<str>::from("."));

        assert!(
            !super::FileExistence::exists(&file_system, &path("definitely-missing.ts")).unwrap()
        );
    }
}
