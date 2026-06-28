//! TypeScript and TSX import extraction contracts.

use crate::failure;
use crate::roots;

/// File content capability consumed by parser input loading.
pub trait SourceReader {
    /// Reads source text for a root-relative path.
    ///
    /// # Errors
    ///
    /// Returns an error when source text cannot be loaded.
    fn source_text(&self, path: &roots::RootRelativePath) -> failure::Result<Box<str>>;
}

/// Request object for parsing a single source file.
pub struct Request<R> {
    /// Reader used to load source text.
    pub reader: R,
    /// Path whose imports should be extracted.
    pub path: roots::RootRelativePath,
}

/// Parsed import records from one source file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Imports {
    /// Static import and re-export specifiers.
    pub static_specifiers: Box<[roots::ImportSpecifier]>,
    /// String-literal dynamic import specifiers.
    pub dynamic_specifiers: Box<[roots::ImportSpecifier]>,
    /// Whether at least one dynamic import cannot be represented statically.
    pub has_unresolved_dynamic_import: bool,
}

/// Extracts import specifiers from TypeScript or TSX source.
///
/// # Errors
///
/// Returns an error when source text cannot be loaded or parsed.
pub fn imports<R>(_request: Request<R>) -> failure::Result<Imports>
where
    R: SourceReader,
{
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use crate::failure;
    use crate::roots;

    #[derive(Clone, Debug)]
    struct FixtureReader {
        source: Box<str>,
    }

    impl super::SourceReader for FixtureReader {
        fn source_text(&self, _path: &roots::RootRelativePath) -> failure::Result<Box<str>> {
            Ok(self.source.clone())
        }
    }

    fn path(value: &str) -> roots::RootRelativePath {
        roots::RootRelativePath::try_from(value).unwrap()
    }

    fn specifier(value: &str) -> roots::ImportSpecifier {
        roots::ImportSpecifier::try_from(value).unwrap()
    }

    #[test]
    #[should_panic(expected = "not implemented")]
    fn extracts_static_side_effect_reexport_type_only_and_literal_dynamic_imports() {
        let source = Box::<str>::from(
            r"
import { AccountCard } from './account-card';
import './register-locale';
export { accountRoutes } from './routes';
import type { Account } from './types';
const lazyPanel = import('./lazy-panel');
",
        );
        let request = super::Request {
            reader: FixtureReader { source },
            path: path("src/accounts/index.ts"),
        };

        // The fixture intentionally mixes import forms common in TS apps so the
        // parser contract documents the graph edges V1 must preserve.
        let imports = super::imports(request).unwrap();

        assert_eq!(
            imports.static_specifiers,
            Box::<[roots::ImportSpecifier]>::from([
                specifier("./account-card"),
                specifier("./register-locale"),
                specifier("./routes"),
                specifier("./types"),
            ]),
        );
        assert_eq!(
            imports.dynamic_specifiers,
            Box::<[roots::ImportSpecifier]>::from([specifier("./lazy-panel")]),
        );
        assert!(!imports.has_unresolved_dynamic_import);
    }

    #[test]
    #[should_panic(expected = "not implemented")]
    fn unknown_dynamic_import_is_reported_as_a_typed_unsupported_signal() {
        let request = super::Request {
            reader: FixtureReader {
                source: Box::<str>::from("const page = import(routeName);"),
            },
            path: path("src/router.ts"),
        };

        // Non-literal dynamic imports must fail closed later instead of silently
        // disappearing from affected-test selection.
        let imports = super::imports(request).unwrap();

        assert!(imports.has_unresolved_dynamic_import);
        assert_eq!(
            imports.dynamic_specifiers,
            Box::<[roots::ImportSpecifier]>::from([])
        );
    }
}
