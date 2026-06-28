//! Test-support virtual filesystem contracts for tests that avoid host state.

use std::collections::BTreeMap;
use std::sync;

use crate::discovery;
use crate::failure;
use crate::fs;
use crate::modules;
use crate::parser;
use crate::roots;
use crate::settings;

/// In-memory file map used by unit and integration tests.
#[derive(Clone, Debug, Default)]
pub struct VirtualFileSystem {
    files: sync::Arc<sync::Mutex<BTreeMap<roots::RootRelativePath, Box<str>>>>,
}

impl VirtualFileSystem {
    /// Returns file content for a root-relative path.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is not present in the virtual filesystem.
    pub fn read_text(&self, path: &roots::RootRelativePath) -> failure::Result<Box<str>> {
        self.snapshot_files()?
            .get(path)
            .cloned()
            .ok_or_else(|| failure::AppError::FileSystem {
                message: format!("virtual file not found: {}", path.as_str()).into_boxed_str(),
            })
    }

    fn snapshot_files(&self) -> failure::Result<BTreeMap<roots::RootRelativePath, Box<str>>> {
        self.files
            .lock()
            .map(|files| files.clone())
            .map_err(|error| failure::AppError::FileSystem {
                message: format!("virtual filesystem lock poisoned: {error}").into_boxed_str(),
            })
    }

    fn replace_text(&self, path: &roots::RootRelativePath, content: &str) -> failure::Result<()> {
        self.files
            .lock()
            .map_err(|error| failure::AppError::FileSystem {
                message: format!("virtual filesystem lock poisoned: {error}").into_boxed_str(),
            })?
            .insert(path.clone(), Box::<str>::from(content));

        Ok(())
    }
}

impl PartialEq for VirtualFileSystem {
    fn eq(&self, other: &Self) -> bool {
        if let (Ok(left_files), Ok(right_files)) = (self.snapshot_files(), other.snapshot_files()) {
            left_files == right_files
        } else {
            false
        }
    }
}

impl Eq for VirtualFileSystem {}

#[cfg(any(test, feature = "test-support"))]
impl VirtualFileSystem {
    /// Creates a virtual filesystem from root-relative file contents.
    #[must_use]
    pub fn with_files(files: Box<[(roots::RootRelativePath, Box<str>)]>) -> Self {
        let files = files.into_vec().into_iter().collect();

        Self {
            files: sync::Arc::new(sync::Mutex::new(files)),
        }
    }
}

impl fs::FileReader for VirtualFileSystem {
    fn read_text(&self, path: &roots::RootRelativePath) -> failure::Result<Box<str>> {
        Self::read_text(self, path)
    }
}

impl fs::FileMetadataReader for VirtualFileSystem {
    fn metadata(&self, path: &roots::RootRelativePath) -> failure::Result<fs::FileMetadata> {
        let content = self.read_text(path)?;
        let byte_len =
            u64::try_from(content.len()).map_err(|error| failure::AppError::FileSystem {
                message: format!("virtual file is too large to report metadata: {error}")
                    .into_boxed_str(),
            })?;

        Ok(fs::FileMetadata {
            byte_len,
            is_directory: false,
        })
    }
}

impl fs::DirectoryWalker for VirtualFileSystem {
    fn source_paths(&self) -> failure::Result<Box<[roots::RootRelativePath]>> {
        Ok(self.snapshot_files()?.keys().cloned().collect())
    }
}

impl fs::FileExistence for VirtualFileSystem {
    fn exists(&self, path: &roots::RootRelativePath) -> failure::Result<bool> {
        Ok(self.snapshot_files()?.contains_key(path))
    }
}

impl fs::FileWriter for VirtualFileSystem {
    fn write_text(&self, path: &roots::RootRelativePath, content: &str) -> failure::Result<()> {
        self.replace_text(path, content)
    }
}

impl settings::LoaderFileSystem for VirtualFileSystem {
    fn read_text(&self, path: &roots::RootRelativePath) -> failure::Result<Box<str>> {
        Self::read_text(self, path)
    }
}

impl parser::SourceReader for VirtualFileSystem {
    fn source_text(&self, path: &roots::RootRelativePath) -> failure::Result<Box<str>> {
        Self::read_text(self, path)
    }
}

impl discovery::SourceDiscoverer for VirtualFileSystem {
    fn candidate_paths(&self) -> failure::Result<Box<[roots::RootRelativePath]>> {
        fs::DirectoryWalker::source_paths(self)
    }
}

impl modules::FileExistence for VirtualFileSystem {
    fn exists(&self, path: &roots::RootRelativePath) -> failure::Result<bool> {
        fs::FileExistence::exists(self, path)
    }
}

#[cfg(test)]
mod tests {
    use crate::discovery;
    use crate::fs;
    use crate::roots;

    fn path(value: &str) -> roots::RootRelativePath {
        roots::RootRelativePath::try_from(value).unwrap()
    }

    #[test]
    fn synthetic_typescript_projects_are_created_without_host_filesystem_state() {
        let file_system = super::VirtualFileSystem::with_files(Box::from([
            (
                path("src/accounts/service.ts"),
                Box::<str>::from("import { button } from '../ui/button';"),
            ),
            (
                path("src/ui/button.tsx"),
                Box::<str>::from("export const button = 'primary';"),
            ),
            (
                path("src/ui/button.test.tsx"),
                Box::<str>::from("import { button } from './button';"),
            ),
        ]));

        // The fixture has a source, component, and colocated test so later code can
        // prove discovery and resolver behavior without reading from the host disk.
        let paths = discovery::SourceDiscoverer::candidate_paths(&file_system).unwrap();

        assert_eq!(
            paths,
            Box::<[roots::RootRelativePath]>::from([
                path("src/accounts/service.ts"),
                path("src/ui/button.test.tsx"),
                path("src/ui/button.tsx"),
            ]),
        );
        assert!(fs::FileExistence::exists(&file_system, &path("src/ui/button.tsx")).unwrap());
    }
}
