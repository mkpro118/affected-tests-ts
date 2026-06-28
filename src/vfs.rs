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
