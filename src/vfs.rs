//! Test-support virtual filesystem contracts for tests that avoid host state.

use std::collections::BTreeMap;

use crate::discovery;
use crate::failure;
use crate::fs;
use crate::modules;
use crate::parser;
use crate::roots;
use crate::settings;

/// In-memory file map used by unit and integration tests.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VirtualFileSystem {
    files: BTreeMap<roots::RootRelativePath, Box<str>>,
}

impl VirtualFileSystem {
    /// Returns file content for a root-relative path.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is not present in the virtual filesystem.
    pub fn read_text(&self, _path: &roots::RootRelativePath) -> failure::Result<Box<str>> {
        unimplemented!()
    }
}

#[cfg(test)]
impl VirtualFileSystem {
    fn with_files(files: Box<[(roots::RootRelativePath, Box<str>)]>) -> Self {
        let files = files.into_vec().into_iter().collect();

        Self { files }
    }
}

impl fs::FileReader for VirtualFileSystem {
    fn read_text(&self, path: &roots::RootRelativePath) -> failure::Result<Box<str>> {
        Self::read_text(self, path)
    }
}

impl fs::FileMetadataReader for VirtualFileSystem {
    fn metadata(&self, _path: &roots::RootRelativePath) -> failure::Result<fs::FileMetadata> {
        unimplemented!()
    }
}

impl fs::DirectoryWalker for VirtualFileSystem {
    fn source_paths(&self) -> failure::Result<Box<[roots::RootRelativePath]>> {
        unimplemented!()
    }
}

impl fs::FileExistence for VirtualFileSystem {
    fn exists(&self, _path: &roots::RootRelativePath) -> failure::Result<bool> {
        unimplemented!()
    }
}

impl fs::FileWriter for VirtualFileSystem {
    fn write_text(&self, _path: &roots::RootRelativePath, _content: &str) -> failure::Result<()> {
        unimplemented!()
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
        unimplemented!()
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
    #[should_panic(expected = "not implemented")]
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
