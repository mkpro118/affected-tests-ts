//! TypeScript and TSX import extraction contracts.

use oxc_allocator::Allocator;
use oxc_ast::ast;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::SourceType;

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
    /// Dynamic import expressions that cannot be represented as graph edges.
    pub unsupported_dynamic_imports: Box<[UnsupportedDynamicImport]>,
}

/// Unsupported dynamic import discovered while parsing one source file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnsupportedDynamicImport {
    /// Typed unsupported dynamic import category.
    pub kind: UnsupportedDynamicImportKind,
}

/// Unsupported dynamic import categories that downstream phases can handle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UnsupportedDynamicImportKind {
    /// `import(...)` was called with a non-string-literal argument.
    NonLiteral,
}

/// Extracts import specifiers from TypeScript or TSX source.
///
/// # Errors
///
/// Returns an error when source text cannot be loaded or parsed.
pub fn imports<R>(request: Request<R>) -> failure::Result<Imports>
where
    R: SourceReader,
{
    let Request { reader, path } = request;
    let source = reader.source_text(&path)?;
    let allocator = Allocator::default();
    let source_type = source_type_for_path(&path)?;
    let parser_return = Parser::new(&allocator, source.as_ref(), source_type).parse();
    if parser_return.panicked
        || should_reject_diagnostics(source_type, !parser_return.diagnostics.is_empty())
    {
        return Err(failure::AppError::Parse {
            message: format!("{:?}", parser_return.diagnostics).into_boxed_str(),
        });
    }

    collect_program_imports(&parser_return.program)
}

fn source_type_for_path(path: &roots::RootRelativePath) -> failure::Result<SourceType> {
    SourceType::from_path(path.as_str()).map_err(|error| failure::AppError::Parse {
        message: format!("unsupported source path `{}`: {error}", path.as_str()).into_boxed_str(),
    })
}

fn should_reject_diagnostics(source_type: SourceType, has_diagnostics: bool) -> bool {
    has_diagnostics && !source_type.is_typescript_definition()
}

fn collect_program_imports(program: &ast::Program<'_>) -> failure::Result<Imports> {
    let mut static_specifiers = Vec::<roots::ImportSpecifier>::new();

    for statement in &program.body {
        collect_static_statement_imports(statement, &mut static_specifiers)?;
    }

    let mut dynamic_collector = DynamicImportCollector::default();
    dynamic_collector.visit_program(program);
    if let Some(error) = dynamic_collector.error {
        return Err(error);
    }

    Ok(Imports {
        static_specifiers: static_specifiers.into_boxed_slice(),
        dynamic_specifiers: dynamic_collector.dynamic_specifiers.into_boxed_slice(),
        unsupported_dynamic_imports: dynamic_collector
            .unsupported_dynamic_imports
            .into_boxed_slice(),
    })
}

fn collect_static_statement_imports(
    statement: &ast::Statement<'_>,
    static_specifiers: &mut Vec<roots::ImportSpecifier>,
) -> failure::Result<()> {
    match statement {
        ast::Statement::ImportDeclaration(declaration) => {
            push_specifier(static_specifiers, declaration.source.value.as_str())?;
        }
        ast::Statement::ExportAllDeclaration(declaration) => {
            push_specifier(static_specifiers, declaration.source.value.as_str())?;
        }
        ast::Statement::ExportNamedDeclaration(declaration) => {
            if let Some(source) = &declaration.source {
                push_specifier(static_specifiers, source.value.as_str())?;
            }
        }
        ast::Statement::BlockStatement(_)
        | ast::Statement::BreakStatement(_)
        | ast::Statement::ContinueStatement(_)
        | ast::Statement::DebuggerStatement(_)
        | ast::Statement::DoWhileStatement(_)
        | ast::Statement::EmptyStatement(_)
        | ast::Statement::ExpressionStatement(_)
        | ast::Statement::ForInStatement(_)
        | ast::Statement::ForOfStatement(_)
        | ast::Statement::ForStatement(_)
        | ast::Statement::IfStatement(_)
        | ast::Statement::LabeledStatement(_)
        | ast::Statement::ReturnStatement(_)
        | ast::Statement::SwitchStatement(_)
        | ast::Statement::ThrowStatement(_)
        | ast::Statement::TryStatement(_)
        | ast::Statement::WhileStatement(_)
        | ast::Statement::WithStatement(_)
        | ast::Statement::VariableDeclaration(_)
        | ast::Statement::FunctionDeclaration(_)
        | ast::Statement::ClassDeclaration(_)
        | ast::Statement::TSTypeAliasDeclaration(_)
        | ast::Statement::TSInterfaceDeclaration(_)
        | ast::Statement::TSEnumDeclaration(_)
        | ast::Statement::TSModuleDeclaration(_)
        | ast::Statement::TSGlobalDeclaration(_)
        | ast::Statement::TSImportEqualsDeclaration(_)
        | ast::Statement::ExportDefaultDeclaration(_)
        | ast::Statement::TSExportAssignment(_)
        | ast::Statement::TSNamespaceExportDeclaration(_) => {}
    }

    Ok(())
}

fn push_specifier(
    specifiers: &mut Vec<roots::ImportSpecifier>,
    value: &str,
) -> failure::Result<()> {
    specifiers.push(roots::ImportSpecifier::try_from(value)?);
    Ok(())
}

#[derive(Default)]
struct DynamicImportCollector {
    dynamic_specifiers: Vec<roots::ImportSpecifier>,
    unsupported_dynamic_imports: Vec<UnsupportedDynamicImport>,
    error: Option<failure::AppError>,
}

impl<'a> Visit<'a> for DynamicImportCollector {
    fn visit_import_expression(&mut self, import_expression: &ast::ImportExpression<'a>) {
        if self.error.is_some() {
            return;
        }

        match &import_expression.source {
            ast::Expression::StringLiteral(literal) => {
                match roots::ImportSpecifier::try_from(literal.value.as_str()) {
                    Ok(specifier) => self.dynamic_specifiers.push(specifier),
                    Err(error) => self.error = Some(error),
                }
            }
            ast::Expression::BooleanLiteral(_)
            | ast::Expression::NullLiteral(_)
            | ast::Expression::NumericLiteral(_)
            | ast::Expression::BigIntLiteral(_)
            | ast::Expression::RegExpLiteral(_)
            | ast::Expression::TemplateLiteral(_)
            | ast::Expression::Identifier(_)
            | ast::Expression::MetaProperty(_)
            | ast::Expression::Super(_)
            | ast::Expression::ArrayExpression(_)
            | ast::Expression::ArrowFunctionExpression(_)
            | ast::Expression::AssignmentExpression(_)
            | ast::Expression::AwaitExpression(_)
            | ast::Expression::BinaryExpression(_)
            | ast::Expression::CallExpression(_)
            | ast::Expression::ChainExpression(_)
            | ast::Expression::ClassExpression(_)
            | ast::Expression::ConditionalExpression(_)
            | ast::Expression::FunctionExpression(_)
            | ast::Expression::ImportExpression(_)
            | ast::Expression::LogicalExpression(_)
            | ast::Expression::NewExpression(_)
            | ast::Expression::ObjectExpression(_)
            | ast::Expression::ParenthesizedExpression(_)
            | ast::Expression::SequenceExpression(_)
            | ast::Expression::TaggedTemplateExpression(_)
            | ast::Expression::ThisExpression(_)
            | ast::Expression::UnaryExpression(_)
            | ast::Expression::UpdateExpression(_)
            | ast::Expression::YieldExpression(_)
            | ast::Expression::PrivateInExpression(_)
            | ast::Expression::JSXElement(_)
            | ast::Expression::JSXFragment(_)
            | ast::Expression::TSAsExpression(_)
            | ast::Expression::TSSatisfiesExpression(_)
            | ast::Expression::TSTypeAssertion(_)
            | ast::Expression::TSNonNullExpression(_)
            | ast::Expression::TSInstantiationExpression(_)
            | ast::Expression::ComputedMemberExpression(_)
            | ast::Expression::StaticMemberExpression(_)
            | ast::Expression::PrivateFieldExpression(_)
            | ast::Expression::V8IntrinsicExpression(_) => {
                self.unsupported_dynamic_imports
                    .push(UnsupportedDynamicImport {
                        kind: UnsupportedDynamicImportKind::NonLiteral,
                    });
            }
        }

        walk::walk_import_expression(self, import_expression);
    }
}

#[cfg(test)]
mod tests {
    use crate::failure;
    use crate::roots;

    #[derive(Clone, Debug)]
    struct FixtureReader {
        source: &'static str,
    }

    impl super::SourceReader for FixtureReader {
        fn source_text(&self, _path: &roots::RootRelativePath) -> failure::Result<Box<str>> {
            Ok(Box::<str>::from(self.source))
        }
    }

    const MIXED_IMPORTS_SOURCE: &str = r"
import { AccountCard } from './account-card';
import './register-locale';
export { accountRoutes } from './routes';
import type { Account } from './types';
const lazyPanel = import('./lazy-panel');
";

    const NON_LITERAL_DYNAMIC_IMPORT_SOURCE: &str = "const page = import(routeName);";

    const IMPORT_META_FALSE_EDGE_SOURCE: &str = r#"
const currentModule = import.meta.url;
const payload = { label: "from", value: "./not-a-module" };
"#;

    const PROPERTY_IMPORT_FALSE_DYNAMIC_SOURCE: &str = r#"
const loader = {
    import(path: string) {
        return path;
    },
};
const chunk = loader.import("./chunk");
"#;

    const AMBIENT_DECLARATION_SOURCE: &str = r#"
import type { Account } from "./account";

declare namespace Hipp {
    declare interface Session {
        account: Account;
    }
}
"#;

    fn path(value: &str) -> roots::RootRelativePath {
        roots::RootRelativePath::try_from(value).unwrap()
    }

    fn specifier(value: &str) -> roots::ImportSpecifier {
        roots::ImportSpecifier::try_from(value).unwrap()
    }

    #[test]
    fn extracts_static_side_effect_reexport_type_only_and_literal_dynamic_imports() {
        let request = super::Request {
            reader: FixtureReader {
                source: MIXED_IMPORTS_SOURCE,
            },
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
        assert_eq!(imports.unsupported_dynamic_imports, Box::from([]));
    }

    #[test]
    fn unknown_dynamic_import_is_reported_as_a_typed_unsupported_signal() {
        let request = super::Request {
            reader: FixtureReader {
                source: NON_LITERAL_DYNAMIC_IMPORT_SOURCE,
            },
            path: path("src/router.ts"),
        };

        // Non-literal dynamic imports must fail closed later instead of silently
        // disappearing from affected-test selection.
        let imports = super::imports(request).unwrap();

        assert_eq!(
            imports.unsupported_dynamic_imports,
            Box::from([super::UnsupportedDynamicImport {
                kind: super::UnsupportedDynamicImportKind::NonLiteral,
            }]),
        );
        assert_eq!(
            imports.dynamic_specifiers,
            Box::<[roots::ImportSpecifier]>::from([])
        );
    }

    #[test]
    fn import_meta_and_later_from_text_do_not_create_false_edges() {
        let request = super::Request {
            reader: FixtureReader {
                source: IMPORT_META_FALSE_EDGE_SOURCE,
            },
            path: path("src/router.ts"),
        };

        // `import.meta` is ordinary expression syntax; only declarations and
        // dynamic import calls should contribute graph edges.
        let imports = super::imports(request).unwrap();

        assert_eq!(
            imports.static_specifiers,
            Box::<[roots::ImportSpecifier]>::from([])
        );
        assert_eq!(
            imports.dynamic_specifiers,
            Box::<[roots::ImportSpecifier]>::from([])
        );
        assert_eq!(imports.unsupported_dynamic_imports, Box::from([]));
    }

    #[test]
    fn property_named_import_calls_do_not_create_dynamic_import_edges() {
        let request = super::Request {
            reader: FixtureReader {
                source: PROPERTY_IMPORT_FALSE_DYNAMIC_SOURCE,
            },
            path: path("src/loader.ts"),
        };

        // Only the `ImportExpression` AST node is a dynamic import; regular
        // property calls named `import` must not become graph edges.
        let imports = super::imports(request).unwrap();

        assert_eq!(
            imports.dynamic_specifiers,
            Box::<[roots::ImportSpecifier]>::from([])
        );
        assert_eq!(imports.unsupported_dynamic_imports, Box::from([]));
    }

    #[test]
    fn declaration_file_diagnostics_do_not_hide_recoverable_imports() {
        let request = super::Request {
            reader: FixtureReader {
                source: AMBIENT_DECLARATION_SOURCE,
            },
            path: path("src/types/session.d.ts"),
        };

        // Large declaration files can contain ambient forms Oxc diagnoses while
        // still producing an AST with the imports needed for graph edges.
        let imports = super::imports(request).unwrap();

        assert_eq!(
            imports.static_specifiers,
            Box::<[roots::ImportSpecifier]>::from([specifier("./account")]),
        );
    }
}
