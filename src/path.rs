//! Typed path and import-specifier values used at module boundaries.

use std::path;

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
        let normalized = normalize(path)?;

        Ok(Self { value: normalized })
    }
}

fn normalize(path: &str) -> failure::Result<Box<str>> {
    if path.is_empty() || path.contains('\\') {
        return Err(failure::AppError::InvalidRootRelativePath {
            path: Box::<str>::from(path),
        });
    }

    let mut components = Vec::<Box<str>>::new();
    for component in path::Path::new(path).components() {
        match component {
            path::Component::Normal(segment) => {
                let segment_text =
                    segment
                        .to_str()
                        .ok_or_else(|| failure::AppError::InvalidRootRelativePath {
                            path: Box::<str>::from(path),
                        })?;
                components.push(Box::<str>::from(segment_text));
            }
            path::Component::CurDir => {}
            path::Component::ParentDir | path::Component::RootDir | path::Component::Prefix(_) => {
                return Err(failure::AppError::InvalidRootRelativePath {
                    path: Box::<str>::from(path),
                });
            }
        }
    }

    if components.is_empty() {
        return Err(failure::AppError::InvalidRootRelativePath {
            path: Box::<str>::from(path),
        });
    }

    Ok(join_components(components.as_slice()).into_boxed_str())
}

fn join_components(components: &[Box<str>]) -> String {
    let mut normalized = String::new();
    for component in components {
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(component.as_ref());
    }
    normalized
}

#[cfg(test)]
mod tests {
    #[test]
    fn root_relative_paths_normalize_current_directories_and_duplicate_separators() {
        let path = super::RootRelativePath::try_from("./src//button/./index.ts").unwrap();

        // Mixed current-directory spellings should collapse before paths enter
        // graph and output contracts so later comparisons remain deterministic.
        assert_eq!(path.as_str(), "src/button/index.ts");
    }

    #[test]
    fn root_relative_paths_reject_absolute_or_parent_directory_segments() {
        assert!(super::RootRelativePath::try_from("/src/button.ts").is_err());
        assert!(super::RootRelativePath::try_from("../button.ts").is_err());
        assert!(super::RootRelativePath::try_from("src/../button.ts").is_err());
    }
}

impl TryFrom<String> for RootRelativePath {
    type Error = failure::AppError;

    fn try_from(path: String) -> failure::Result<Self> {
        Self::try_from(path.as_str())
    }
}

impl TryFrom<Box<str>> for RootRelativePath {
    type Error = failure::AppError;

    fn try_from(path: Box<str>) -> failure::Result<Self> {
        Self::try_from(path.as_ref())
    }
}

impl std::fmt::Display for RootRelativePath {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
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

impl std::fmt::Display for ImportSpecifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}
