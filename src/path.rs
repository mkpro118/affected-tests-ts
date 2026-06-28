//! Typed path and import-specifier values used at module boundaries.

use crate::failure;

/// A normalized path relative to the repository root.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RootRelativePath {
    value: Box<str>,
}

impl RootRelativePath {
    /// Returns the normalized root-relative path text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.value.as_ref()
    }
}

impl TryFrom<&str> for RootRelativePath {
    type Error = failure::AppError;

    fn try_from(path: &str) -> failure::Result<Self> {
        if path.is_empty() || path.starts_with('/') || path.contains("..") {
            return Err(failure::AppError::InvalidRootRelativePath {
                path: Box::<str>::from(path),
            });
        }

        Ok(Self {
            value: Box::<str>::from(path),
        })
    }
}

/// A TypeScript import specifier before resolution.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ImportSpecifier {
    value: Box<str>,
}

impl ImportSpecifier {
    /// Returns the original import specifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.value.as_ref()
    }
}

impl TryFrom<&str> for ImportSpecifier {
    type Error = failure::AppError;

    fn try_from(specifier: &str) -> failure::Result<Self> {
        if specifier.is_empty() {
            return Err(failure::AppError::InvalidImportSpecifier {
                specifier: Box::<str>::from(specifier),
            });
        }

        Ok(Self {
            value: Box::<str>::from(specifier),
        })
    }
}
