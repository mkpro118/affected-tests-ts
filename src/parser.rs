//! TypeScript and TSX import extraction contracts.

use oxc_allocator::Allocator;
use oxc_ast::ast;
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_span::SourceType;

use crate::failure;
use crate::roots;
use crate::static_refs;

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

    let resolver = static_refs::Resolver::from_program(program);
    let mut dynamic_collector = DynamicImportCollector::new(resolver);
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

struct DynamicImportCollector {
    resolver: static_refs::Resolver,
    dynamic_specifiers: Vec<roots::ImportSpecifier>,
    unsupported_dynamic_imports: Vec<UnsupportedDynamicImport>,
    error: Option<failure::AppError>,
}

impl DynamicImportCollector {
    const fn new(resolver: static_refs::Resolver) -> Self {
        Self {
            resolver,
            dynamic_specifiers: Vec::new(),
            unsupported_dynamic_imports: Vec::new(),
            error: None,
        }
    }
}

impl<'a> Visit<'a> for DynamicImportCollector {
    fn visit_import_expression(&mut self, import_expression: &ast::ImportExpression<'a>) {
        if self.error.is_some() {
            return;
        }

        match self
            .resolver
            .dynamic_import_specifier(&import_expression.source)
        {
            Some(specifier_value) => {
                match roots::ImportSpecifier::try_from(specifier_value.as_ref()) {
                    Ok(specifier) => self.dynamic_specifiers.push(specifier),
                    Err(error) => self.error = Some(error),
                }
            }
            None => {
                self.unsupported_dynamic_imports
                    .push(UnsupportedDynamicImport {
                        kind: UnsupportedDynamicImportKind::NonLiteral,
                    });
            }
        }

        walk::walk_import_expression(self, import_expression);
    }

    fn visit_function(
        &mut self,
        function: &ast::Function<'a>,
        flags: oxc_syntax::scope::ScopeFlags,
    ) {
        let scope = self.resolver.scope_for_function(function);
        self.resolver.push_scope(scope);
        walk::walk_function(self, function, flags);
        self.resolver.pop_scope();
    }

    fn visit_arrow_function_expression(&mut self, function: &ast::ArrowFunctionExpression<'a>) {
        let scope = self.resolver.scope_for_parameters(&function.params);
        self.resolver.push_scope(scope);
        walk::walk_arrow_function_expression(self, function);
        self.resolver.pop_scope();
    }

    fn visit_function_body(&mut self, body: &ast::FunctionBody<'a>) {
        let scope = self.resolver.scope_for_function_body(body);
        self.resolver.push_scope(scope);
        walk::walk_function_body(self, body);
        self.resolver.pop_scope();
    }

    fn visit_block_statement(&mut self, block: &ast::BlockStatement<'a>) {
        let scope = self.resolver.scope_for_block(block);
        self.resolver.push_scope(scope);
        walk::walk_block_statement(self, block);
        self.resolver.pop_scope();
    }

    fn visit_switch_statement(&mut self, switch_statement: &ast::SwitchStatement<'a>) {
        let scope = self.resolver.scope_for_switch_statement(switch_statement);
        self.resolver.push_scope(scope);
        walk::walk_switch_statement(self, switch_statement);
        self.resolver.pop_scope();
    }

    fn visit_switch_case(&mut self, switch_case: &ast::SwitchCase<'a>) {
        let scope = self.resolver.scope_for_switch_case(switch_case);
        self.resolver.push_scope(scope);
        walk::walk_switch_case(self, switch_case);
        self.resolver.pop_scope();
    }

    fn visit_catch_clause(&mut self, catch_clause: &ast::CatchClause<'a>) {
        let scope = self.resolver.scope_for_catch_clause(catch_clause);
        self.resolver.push_scope(scope);
        walk::walk_catch_clause(self, catch_clause);
        self.resolver.pop_scope();
    }

    fn visit_for_statement(&mut self, statement: &ast::ForStatement<'a>) {
        let scope = self.resolver.scope_for_for_statement(statement);
        self.resolver.push_scope(scope);
        walk::walk_for_statement(self, statement);
        self.resolver.pop_scope();
    }

    fn visit_for_in_statement(&mut self, statement: &ast::ForInStatement<'a>) {
        let scope = self.resolver.scope_for_for_in_statement(statement);
        self.resolver.push_scope(scope);
        walk::walk_for_in_statement(self, statement);
        self.resolver.pop_scope();
    }

    fn visit_for_of_statement(&mut self, statement: &ast::ForOfStatement<'a>) {
        let scope = self.resolver.scope_for_for_of_statement(statement);
        self.resolver.push_scope(scope);
        walk::walk_for_of_statement(self, statement);
        self.resolver.pop_scope();
    }

    fn visit_class(&mut self, class: &ast::Class<'a>) {
        let scope = self.resolver.scope_for_class(class);
        self.resolver.push_scope(scope);
        walk::walk_class(self, class);
        self.resolver.pop_scope();
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

    const CACHE_BUSTED_CONST_DYNAMIC_IMPORT_SOURCE: &str = r#"
import { resolve } from "node:path";

const SUT = "src/utils/date";
const REHYDRATE_SUT = "../actions" as const;
const ASSERTED_SUT = <const>"../authReport";
const PATH_RESOLVED_SUT = resolve(import.meta.dir, "../dag-resolver");
const MODULE_PATH = "@/app/(dashboard)/billing/manual-eob/helpers" as const;
const bustCache = () => `${MODULE_PATH}?update=${Date.now()}`;

async function loadModules() {
    await import(`${SUT}?ts=${Date.now()}`);
    await import(`${REHYDRATE_SUT}?ts=${Date.now()}-${Math.random()}`);
    await import(`${ASSERTED_SUT}?ts=${Date.now()}`);
    await import(`${PATH_RESOLVED_SUT}?ts=${Date.now()}`);
    await import(bustCache());
}
"#;

    const CACHE_BUSTED_DIRECT_TEMPLATE_IMPORT_SOURCE: &str = r"
async function loadMetrics() {
    await import(`../dashboard/KPIMetricsCards?ts=${Date.now()}`);
}
";

    const CACHE_BUSTED_STRING_LITERAL_IMPORT_SOURCE: &str =
        "const lazyPanel = import('./lazy-panel?ts=123');";

    const CONST_IDENTIFIER_DYNAMIC_IMPORT_SOURCE: &str = r#"
const PROVIDER_SUT = "src/app/sidebarPrimitives";

async function loadProvider() {
    await import(PROVIDER_SUT);
}
"#;

    const NESTED_CALLBACK_CONST_DYNAMIC_IMPORT_SOURCE: &str = r#"
describe("feature", () => {
    const SYSTEM_UNDER_TEST = "../utils";

    beforeAll(async () => {
        await import(`${SYSTEM_UNDER_TEST}?ts=${Date.now()}`);
    });
});
"#;

    const PARAMETER_SHADOWED_SUT_DYNAMIC_IMPORT_SOURCE: &str = r#"
const SUT = "../safe";

async function load(SUT: string) {
    await import(`${SUT}?ts=${Date.now()}`);
}
"#;

    const LOCAL_SHADOWED_SUT_DYNAMIC_IMPORT_SOURCE: &str = r#"
const SUT = "../safe";

async function load() {
    const SUT = getRuntimePath();
    await import(`${SUT}?ts=${Date.now()}`);
}
"#;

    const LOOP_SHADOWED_SUT_DYNAMIC_IMPORT_SOURCE: &str = r#"
const SUT = "../safe";

async function load(paths: string[], pathMap: Record<string, string>) {
    for (const SUT = nextPath(); shouldLoad(SUT); advance()) {
        await import(`${SUT}?ts=${Date.now()}`);
    }
    for (const SUT in pathMap) {
        await import(`${SUT}?ts=${Date.now()}`);
    }
    for (const SUT of paths) {
        await import(`${SUT}?ts=${Date.now()}`);
    }
}
"#;

    const TOP_LEVEL_LOOP_SHADOWED_SUT_DYNAMIC_IMPORT_SOURCE: &str = r#"
const SUT = "../safe";
declare const paths: string[];

for (const SUT of paths) {
    import(`${SUT}?ts=${Date.now()}`);
}
"#;

    const CONTROL_FLOW_LOOP_SHADOWED_SUT_DYNAMIC_IMPORT_SOURCE: &str = r#"
const SUT = "../safe";

async function load(paths: string[], shouldLoad: boolean) {
    if (shouldLoad)
        for (const SUT of paths)
            await import(`${SUT}?ts=${Date.now()}`);
}
"#;

    const CATCH_PARAMETER_SHADOWED_SUT_DYNAMIC_IMPORT_SOURCE: &str = r#"
const SUT = "../safe";

async function load() {
    try {
        await prepare();
    } catch (SUT) {
        await import(`${SUT}?ts=${Date.now()}`);
    }
}
"#;

    const SWITCH_CASE_SHADOWED_SUT_DYNAMIC_IMPORT_SOURCE: &str = r#"
const SUT = "../safe";

async function load(kind: string) {
    switch (kind) {
        case "runtime":
            const SUT = getRuntimePath();
            await import(`${SUT}?ts=${Date.now()}`);
            break;
    }
}
"#;

    const SWITCH_SIBLING_CASE_SHADOWED_SUT_DYNAMIC_IMPORT_SOURCE: &str = r#"
const SUT = "../safe";

async function load(kind: string) {
    switch (kind) {
        case "a": {
            const SUT = runtimePath();
            break;
        }
        case "b": {
            await import(`${SUT}?ts=${Date.now()}`);
            break;
        }
    }
}
"#;

    const NAMED_FUNCTION_EXPRESSION_SHADOWED_SUT_DYNAMIC_IMPORT_SOURCE: &str = r#"
const SUT = "../safe";

const load = async function SUT() {
    await import(`${SUT}?ts=${Date.now()}`);
};
"#;

    const NAMED_CLASS_EXPRESSION_SHADOWED_SUT_DYNAMIC_IMPORT_SOURCE: &str = r#"
const SUT = "../safe";

const Loader = class SUT {
    static module = import(`${SUT}?ts=${Date.now()}`);
};
"#;

    const ARBITRARY_DYNAMIC_IMPORT_SOURCE: &str = r"
async function loadRoute(routeName: string) {
    await import(`${routeName}/page?ts=${Date.now()}`);
    await import(routeName);
}
";

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

declare namespace AppTypes {
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
    fn cache_busted_const_template_imports_resolve_as_dynamic_specifiers() {
        let request = super::Request {
            reader: FixtureReader {
                source: CACHE_BUSTED_CONST_DYNAMIC_IMPORT_SOURCE,
            },
            path: path("src/utils/date.test.ts"),
        };

        // The fixture mirrors SUT helpers that reload modules with query strings;
        // both imports should become ordinary dynamic graph edges.
        let imports = super::imports(request).unwrap();

        assert_eq!(
            imports.dynamic_specifiers,
            Box::<[roots::ImportSpecifier]>::from([
                specifier("src/utils/date"),
                specifier("../actions"),
                specifier("../authReport"),
                specifier("../dag-resolver"),
                specifier("@/app/(dashboard)/billing/manual-eob/helpers"),
            ]),
        );
        assert_eq!(imports.unsupported_dynamic_imports, Box::from([]));
    }

    #[test]
    fn direct_cache_busted_template_import_resolves_as_dynamic_specifier() {
        let request = super::Request {
            reader: FixtureReader {
                source: CACHE_BUSTED_DIRECT_TEMPLATE_IMPORT_SOURCE,
            },
            path: path("src/dashboard/metrics.test.tsx"),
        };

        // Direct template literals with a static path before the query are safe
        // to keep because the cache-buster does not affect module resolution.
        let imports = super::imports(request).unwrap();

        assert_eq!(
            imports.dynamic_specifiers,
            Box::<[roots::ImportSpecifier]>::from([specifier("../dashboard/KPIMetricsCards")]),
        );
        assert_eq!(imports.unsupported_dynamic_imports, Box::from([]));
    }

    #[test]
    fn string_literal_dynamic_import_query_strings_are_stripped() {
        let request = super::Request {
            reader: FixtureReader {
                source: CACHE_BUSTED_STRING_LITERAL_IMPORT_SOURCE,
            },
            path: path("src/router.ts"),
        };

        // Literal cache-busting queries are runtime noise; the resolver needs
        // only the module specifier portion for graph construction.
        let imports = super::imports(request).unwrap();

        assert_eq!(
            imports.dynamic_specifiers,
            Box::<[roots::ImportSpecifier]>::from([specifier("./lazy-panel")]),
        );
        assert_eq!(imports.unsupported_dynamic_imports, Box::from([]));
    }

    #[test]
    fn const_identifier_dynamic_import_resolves_as_dynamic_specifier() {
        let request = super::Request {
            reader: FixtureReader {
                source: CONST_IDENTIFIER_DYNAMIC_IMPORT_SOURCE,
            },
            path: path("src/app/sidebar.test.tsx"),
        };

        // Some tests intentionally avoid cache-busting shared provider modules;
        // top-level const identifiers still provide enough static proof.
        let imports = super::imports(request).unwrap();

        assert_eq!(
            imports.dynamic_specifiers,
            Box::<[roots::ImportSpecifier]>::from([specifier("src/app/sidebarPrimitives")]),
        );
        assert_eq!(imports.unsupported_dynamic_imports, Box::from([]));
    }

    #[test]
    fn nested_callback_const_template_import_resolves_as_dynamic_specifier() {
        let request = super::Request {
            reader: FixtureReader {
                source: NESTED_CALLBACK_CONST_DYNAMIC_IMPORT_SOURCE,
            },
            path: path("src/features/widget.test.ts"),
        };

        // Test callbacks often declare SUT helpers locally; static lexical consts
        // should still become graph edges when no runtime binding shadows them.
        let imports = super::imports(request).unwrap();

        assert_eq!(
            imports.dynamic_specifiers,
            Box::<[roots::ImportSpecifier]>::from([specifier("../utils")]),
        );
        assert_eq!(imports.unsupported_dynamic_imports, Box::from([]));
    }

    #[test]
    fn parameter_shadowed_sut_template_import_remains_unsupported() {
        let request = super::Request {
            reader: FixtureReader {
                source: PARAMETER_SHADOWED_SUT_DYNAMIC_IMPORT_SOURCE,
            },
            path: path("src/router.ts"),
        };

        // A parameter with the SUT name means the template uses runtime data, not
        // the top-level const, so it must keep the graph fail-closed.
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
    fn local_shadowed_sut_template_import_remains_unsupported() {
        let request = super::Request {
            reader: FixtureReader {
                source: LOCAL_SHADOWED_SUT_DYNAMIC_IMPORT_SOURCE,
            },
            path: path("src/router.ts"),
        };

        // Local bindings shadow the file-level SUT helper for the whole function
        // body, including cache-busted templates that otherwise look static.
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
    fn loop_initializer_shadowed_sut_template_imports_remain_unsupported() {
        let request = super::Request {
            reader: FixtureReader {
                source: LOOP_SHADOWED_SUT_DYNAMIC_IMPORT_SOURCE,
            },
            path: path("src/router.ts"),
        };

        // Loop initializer bindings are lexical runtime values, so templates
        // inside for/for-in/for-of bodies must not use the file-level SUT const.
        let imports = super::imports(request).unwrap();

        assert_eq!(
            imports.unsupported_dynamic_imports,
            Box::from([
                super::UnsupportedDynamicImport {
                    kind: super::UnsupportedDynamicImportKind::NonLiteral,
                },
                super::UnsupportedDynamicImport {
                    kind: super::UnsupportedDynamicImportKind::NonLiteral,
                },
                super::UnsupportedDynamicImport {
                    kind: super::UnsupportedDynamicImportKind::NonLiteral,
                },
            ]),
        );
        assert_eq!(
            imports.dynamic_specifiers,
            Box::<[roots::ImportSpecifier]>::from([])
        );
    }

    #[test]
    fn top_level_loop_initializer_shadowed_sut_template_import_remains_unsupported() {
        let request = super::Request {
            reader: FixtureReader {
                source: TOP_LEVEL_LOOP_SHADOWED_SUT_DYNAMIC_IMPORT_SOURCE,
            },
            path: path("src/router.ts"),
        };

        // A top-level loop introduces its own lexical binding, so the template
        // cannot safely reuse the file-level SUT const.
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
    fn control_flow_loop_initializer_shadowed_sut_template_import_remains_unsupported() {
        let request = super::Request {
            reader: FixtureReader {
                source: CONTROL_FLOW_LOOP_SHADOWED_SUT_DYNAMIC_IMPORT_SOURCE,
            },
            path: path("src/router.ts"),
        };

        // Non-block control-flow parents still leave the loop initializer in
        // scope for the loop body, so the template must fail closed.
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
    fn catch_parameter_shadowed_sut_template_import_remains_unsupported() {
        let request = super::Request {
            reader: FixtureReader {
                source: CATCH_PARAMETER_SHADOWED_SUT_DYNAMIC_IMPORT_SOURCE,
            },
            path: path("src/router.ts"),
        };

        // Catch parameters are runtime bindings inside the catch body and must
        // not resolve through a same-named top-level SUT const.
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
    fn switch_case_shadowed_sut_template_import_remains_unsupported() {
        let request = super::Request {
            reader: FixtureReader {
                source: SWITCH_CASE_SHADOWED_SUT_DYNAMIC_IMPORT_SOURCE,
            },
            path: path("src/router.ts"),
        };

        // Switch cases are not block statements, but lexical declarations in a
        // case still shadow the file-level SUT const for later case statements.
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
    fn switch_sibling_case_shadowed_sut_template_import_remains_unsupported() {
        let request = super::Request {
            reader: FixtureReader {
                source: SWITCH_SIBLING_CASE_SHADOWED_SUT_DYNAMIC_IMPORT_SOURCE,
            },
            path: path("src/router.ts"),
        };

        // Switch lexical scope spans cases for this evaluator, so a SUT binding
        // in one case blocks top-level SUT resolution in sibling cases.
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
    fn named_function_expression_shadowed_sut_template_import_remains_unsupported() {
        let request = super::Request {
            reader: FixtureReader {
                source: NAMED_FUNCTION_EXPRESSION_SHADOWED_SUT_DYNAMIC_IMPORT_SOURCE,
            },
            path: path("src/router.ts"),
        };

        // A named function expression exposes its name inside the function body,
        // so a matching SUT name is not proof of the top-level const.
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
    fn named_class_expression_shadowed_sut_template_import_remains_unsupported() {
        let request = super::Request {
            reader: FixtureReader {
                source: NAMED_CLASS_EXPRESSION_SHADOWED_SUT_DYNAMIC_IMPORT_SOURCE,
            },
            path: path("src/router.ts"),
        };

        // A named class expression exposes its name inside class body traversal,
        // so same-named templates cannot prove they reference the top-level SUT.
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
    fn arbitrary_template_and_identifier_imports_remain_unsupported() {
        let request = super::Request {
            reader: FixtureReader {
                source: ARBITRARY_DYNAMIC_IMPORT_SOURCE,
            },
            path: path("src/router.ts"),
        };

        // Templates that compute path structure and bare identifier imports still
        // lack enough static proof, so selection must continue to fail closed.
        let imports = super::imports(request).unwrap();

        assert_eq!(
            imports.unsupported_dynamic_imports,
            Box::from([
                super::UnsupportedDynamicImport {
                    kind: super::UnsupportedDynamicImportKind::NonLiteral,
                },
                super::UnsupportedDynamicImport {
                    kind: super::UnsupportedDynamicImportKind::NonLiteral,
                },
            ]),
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
