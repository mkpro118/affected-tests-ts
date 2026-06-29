//! Static reference resolution for TypeScript dynamic import expressions.

use std::collections::{BTreeMap, BTreeSet};

use oxc_ast::ast;

/// Resolves statically provable import references while tracking lexical shadows.
pub struct Resolver {
    const_string_bindings: BTreeMap<Box<str>, Box<str>>,
    const_function_bindings: BTreeMap<Box<str>, Box<str>>,
    scopes: Vec<StaticScope>,
}

/// Static references and shadows visible in one lexical scope.
#[derive(Default)]
pub struct StaticScope {
    strings: BTreeMap<Box<str>, Box<str>>,
    functions: BTreeMap<Box<str>, Box<str>>,
    shadows: BTreeSet<Box<str>>,
}

impl Resolver {
    /// Builds a resolver from top-level `const` string bindings.
    #[must_use]
    pub fn from_program(program: &ast::Program<'_>) -> Self {
        let const_string_bindings = collect_top_level_const_string_bindings(program);
        let const_function_bindings =
            collect_top_level_const_function_bindings(program, &const_string_bindings);
        Self {
            const_string_bindings,
            const_function_bindings,
            scopes: Vec::new(),
        }
    }

    /// Resolves a dynamic import argument when it is statically provable.
    #[must_use]
    pub fn dynamic_import_specifier(&self, expression: &ast::Expression<'_>) -> Option<Box<str>> {
        match expression {
            ast::Expression::StringLiteral(literal) => Some(strip_query(literal.value.as_str())),
            ast::Expression::TemplateLiteral(template) => self.template_import_specifier(template),
            ast::Expression::Identifier(identifier) => self.identifier_import_specifier(identifier),
            ast::Expression::CallExpression(call) => self.call_import_specifier(call),
            ast::Expression::ParenthesizedExpression(expression) => {
                self.dynamic_import_specifier(&expression.expression)
            }
            ast::Expression::TSAsExpression(expression) => {
                self.dynamic_import_specifier(&expression.expression)
            }
            ast::Expression::TSSatisfiesExpression(expression) => {
                self.dynamic_import_specifier(&expression.expression)
            }
            ast::Expression::TSTypeAssertion(expression) => {
                self.dynamic_import_specifier(&expression.expression)
            }
            ast::Expression::TSNonNullExpression(expression) => {
                self.dynamic_import_specifier(&expression.expression)
            }
            ast::Expression::BooleanLiteral(_)
            | ast::Expression::NullLiteral(_)
            | ast::Expression::NumericLiteral(_)
            | ast::Expression::BigIntLiteral(_)
            | ast::Expression::RegExpLiteral(_)
            | ast::Expression::MetaProperty(_)
            | ast::Expression::Super(_)
            | ast::Expression::ArrayExpression(_)
            | ast::Expression::ArrowFunctionExpression(_)
            | ast::Expression::AssignmentExpression(_)
            | ast::Expression::AwaitExpression(_)
            | ast::Expression::BinaryExpression(_)
            | ast::Expression::ChainExpression(_)
            | ast::Expression::ClassExpression(_)
            | ast::Expression::ConditionalExpression(_)
            | ast::Expression::FunctionExpression(_)
            | ast::Expression::ImportExpression(_)
            | ast::Expression::LogicalExpression(_)
            | ast::Expression::NewExpression(_)
            | ast::Expression::ObjectExpression(_)
            | ast::Expression::SequenceExpression(_)
            | ast::Expression::TaggedTemplateExpression(_)
            | ast::Expression::ThisExpression(_)
            | ast::Expression::UnaryExpression(_)
            | ast::Expression::UpdateExpression(_)
            | ast::Expression::YieldExpression(_)
            | ast::Expression::PrivateInExpression(_)
            | ast::Expression::JSXElement(_)
            | ast::Expression::JSXFragment(_)
            | ast::Expression::TSInstantiationExpression(_)
            | ast::Expression::ComputedMemberExpression(_)
            | ast::Expression::StaticMemberExpression(_)
            | ast::Expression::PrivateFieldExpression(_)
            | ast::Expression::V8IntrinsicExpression(_) => None,
        }
    }

    /// Pushes static bindings and shadows for one lexical scope.
    pub fn push_scope(&mut self, scope: StaticScope) {
        self.scopes.push(scope);
    }

    /// Pops the most recent lexical scope.
    pub fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// Collects function parameter scope effects.
    #[must_use]
    pub fn scope_for_parameters(&self, parameters: &ast::FormalParameters<'_>) -> StaticScope {
        let mut scope = StaticScope::default();
        for parameter in &parameters.items {
            self.collect_shadow_binding_pattern(&parameter.pattern, &mut scope.shadows);
        }
        if let Some(rest) = &parameters.rest {
            self.collect_shadow_binding_pattern(&rest.rest.argument, &mut scope.shadows);
        }

        scope
    }

    /// Collects scope effects visible inside a function node.
    #[must_use]
    pub fn scope_for_function(&self, function: &ast::Function<'_>) -> StaticScope {
        let mut scope = self.scope_for_parameters(&function.params);
        if matches!(
            function.r#type,
            ast::FunctionType::FunctionExpression
                | ast::FunctionType::TSEmptyBodyFunctionExpression
        ) {
            self.collect_shadow_binding_identifier(function.id.as_ref(), &mut scope.shadows);
        }

        scope
    }

    /// Collects lexical static bindings and shadows declared in a function body.
    #[must_use]
    pub fn scope_for_function_body(&self, body: &ast::FunctionBody<'_>) -> StaticScope {
        self.scope_for_statements(body.statements.as_ref())
    }

    /// Collects lexical static bindings and shadows declared in a block.
    #[must_use]
    pub fn scope_for_block(&self, block: &ast::BlockStatement<'_>) -> StaticScope {
        self.scope_for_statements(block.body.as_ref())
    }

    /// Collects lexical scope effects declared across a switch statement.
    #[must_use]
    pub fn scope_for_switch_statement(
        &self,
        switch_statement: &ast::SwitchStatement<'_>,
    ) -> StaticScope {
        let mut scope = StaticScope::default();
        for switch_case in &switch_statement.cases {
            for statement in &switch_case.consequent {
                self.collect_shadow_statement_tree(statement, &mut scope.shadows);
            }
        }

        scope
    }

    /// Collects lexical static bindings and shadows declared in a switch case body.
    #[must_use]
    pub fn scope_for_switch_case(&self, switch_case: &ast::SwitchCase<'_>) -> StaticScope {
        self.scope_for_statements(switch_case.consequent.as_ref())
    }

    /// Collects scope effects introduced by a catch clause.
    #[must_use]
    pub fn scope_for_catch_clause(&self, catch_clause: &ast::CatchClause<'_>) -> StaticScope {
        let mut scope = StaticScope::default();
        if let Some(parameter) = &catch_clause.param {
            self.collect_shadow_binding_pattern(&parameter.pattern, &mut scope.shadows);
        }

        scope
    }

    /// Collects scope effects introduced by a `for` loop initializer.
    #[must_use]
    pub fn scope_for_for_statement(&self, statement: &ast::ForStatement<'_>) -> StaticScope {
        let mut scope = StaticScope::default();
        self.collect_shadow_for_statement_init(statement.init.as_ref(), &mut scope.shadows);
        scope
    }

    /// Collects scope effects introduced by a `for...in` loop initializer.
    #[must_use]
    pub fn scope_for_for_in_statement(&self, statement: &ast::ForInStatement<'_>) -> StaticScope {
        let mut scope = StaticScope::default();
        self.collect_shadow_for_statement_left(&statement.left, &mut scope.shadows);
        scope
    }

    /// Collects scope effects introduced by a `for...of` loop initializer.
    #[must_use]
    pub fn scope_for_for_of_statement(&self, statement: &ast::ForOfStatement<'_>) -> StaticScope {
        let mut scope = StaticScope::default();
        self.collect_shadow_for_statement_left(&statement.left, &mut scope.shadows);
        scope
    }

    /// Collects scope effects introduced by a named class expression.
    #[must_use]
    pub fn scope_for_class(&self, class: &ast::Class<'_>) -> StaticScope {
        let mut scope = StaticScope::default();
        if class.r#type == ast::ClassType::ClassExpression {
            self.collect_shadow_binding_identifier(class.id.as_ref(), &mut scope.shadows);
        }

        scope
    }

    fn template_import_specifier(&self, template: &ast::TemplateLiteral<'_>) -> Option<Box<str>> {
        let first_quasi = template.quasis.first()?;
        let first_text = template_element_text(first_quasi);
        if let Some((specifier, _query)) = first_text.split_once('?') {
            return Some(Box::<str>::from(specifier));
        }

        if !first_text.is_empty() {
            return None;
        }

        let first_expression = template.expressions.first()?;
        let ast::Expression::Identifier(identifier) = first_expression else {
            return None;
        };
        if self.is_shadowed(identifier.name.as_str()) {
            return None;
        }

        let bound_specifier = self.string_binding(identifier.name.as_str())?;
        let second_quasi = template.quasis.get(1)?;
        let second_text = template_element_text(second_quasi);
        if second_text.starts_with('?') {
            Some(bound_specifier)
        } else {
            None
        }
    }

    fn identifier_import_specifier(
        &self,
        identifier: &ast::IdentifierReference<'_>,
    ) -> Option<Box<str>> {
        self.string_binding(identifier.name.as_str())
    }

    fn call_import_specifier(&self, call: &ast::CallExpression<'_>) -> Option<Box<str>> {
        if let Some(specifier) = path_resolve_import_meta_dir_specifier(call) {
            return Some(specifier);
        }

        let ast::Expression::Identifier(callee) = &call.callee else {
            return None;
        };
        if self.is_shadowed(callee.name.as_str()) || !call.arguments.is_empty() {
            return None;
        }

        self.function_binding(callee.name.as_str())
    }

    fn string_binding(&self, name: &str) -> Option<Box<str>> {
        for scope in self.scopes.iter().rev() {
            if let Some(specifier) = scope.strings.get(name) {
                return Some(specifier.clone());
            }
            if scope.shadows.contains(name) {
                return None;
            }
        }

        self.const_string_bindings.get(name).cloned()
    }

    fn function_binding(&self, name: &str) -> Option<Box<str>> {
        for scope in self.scopes.iter().rev() {
            if let Some(specifier) = scope.functions.get(name) {
                return Some(specifier.clone());
            }
            if scope.shadows.contains(name) {
                return None;
            }
        }

        self.const_function_bindings.get(name).cloned()
    }

    fn is_shadowed(&self, name: &str) -> bool {
        self.scopes
            .iter()
            .rev()
            .any(|scope| scope.shadows.contains(name))
    }

    fn scope_for_statements(&self, statements: &[ast::Statement<'_>]) -> StaticScope {
        let mut scope = StaticScope::default();
        for statement in statements {
            match statement {
                ast::Statement::VariableDeclaration(declaration) => {
                    self.collect_scope_variable_declaration(declaration, &mut scope);
                }
                ast::Statement::FunctionDeclaration(function) => {
                    self.collect_shadow_binding_identifier(
                        function.id.as_ref(),
                        &mut scope.shadows,
                    );
                }
                ast::Statement::ClassDeclaration(class) => {
                    self.collect_shadow_binding_identifier(class.id.as_ref(), &mut scope.shadows);
                }
                ast::Statement::ForStatement(statement) => {
                    self.collect_shadow_for_statement_init(
                        statement.init.as_ref(),
                        &mut scope.shadows,
                    );
                }
                ast::Statement::ForInStatement(statement) => {
                    self.collect_shadow_for_statement_left(&statement.left, &mut scope.shadows);
                }
                ast::Statement::ForOfStatement(statement) => {
                    self.collect_shadow_for_statement_left(&statement.left, &mut scope.shadows);
                }
                ast::Statement::BlockStatement(_)
                | ast::Statement::BreakStatement(_)
                | ast::Statement::ContinueStatement(_)
                | ast::Statement::DebuggerStatement(_)
                | ast::Statement::DoWhileStatement(_)
                | ast::Statement::EmptyStatement(_)
                | ast::Statement::ExpressionStatement(_)
                | ast::Statement::IfStatement(_)
                | ast::Statement::LabeledStatement(_)
                | ast::Statement::ReturnStatement(_)
                | ast::Statement::SwitchStatement(_)
                | ast::Statement::ThrowStatement(_)
                | ast::Statement::TryStatement(_)
                | ast::Statement::WhileStatement(_)
                | ast::Statement::WithStatement(_)
                | ast::Statement::TSTypeAliasDeclaration(_)
                | ast::Statement::TSInterfaceDeclaration(_)
                | ast::Statement::TSEnumDeclaration(_)
                | ast::Statement::TSModuleDeclaration(_)
                | ast::Statement::TSGlobalDeclaration(_)
                | ast::Statement::TSImportEqualsDeclaration(_)
                | ast::Statement::ExportDefaultDeclaration(_)
                | ast::Statement::TSExportAssignment(_)
                | ast::Statement::TSNamespaceExportDeclaration(_)
                | ast::Statement::ImportDeclaration(_)
                | ast::Statement::ExportAllDeclaration(_)
                | ast::Statement::ExportNamedDeclaration(_) => {}
            }
        }

        scope
    }

    fn collect_shadow_statement_tree(
        &self,
        statement: &ast::Statement<'_>,
        bindings: &mut BTreeSet<Box<str>>,
    ) {
        match statement {
            ast::Statement::BlockStatement(block) => {
                for statement in &block.body {
                    self.collect_shadow_statement_tree(statement, bindings);
                }
            }
            ast::Statement::VariableDeclaration(declaration) => {
                self.collect_shadow_variable_declaration(declaration, bindings);
            }
            ast::Statement::FunctionDeclaration(function) => {
                self.collect_shadow_binding_identifier(function.id.as_ref(), bindings);
            }
            ast::Statement::ClassDeclaration(class) => {
                self.collect_shadow_binding_identifier(class.id.as_ref(), bindings);
            }
            ast::Statement::ForStatement(statement) => {
                self.collect_shadow_for_statement_init(statement.init.as_ref(), bindings);
            }
            ast::Statement::ForInStatement(statement) => {
                self.collect_shadow_for_statement_left(&statement.left, bindings);
            }
            ast::Statement::ForOfStatement(statement) => {
                self.collect_shadow_for_statement_left(&statement.left, bindings);
            }
            ast::Statement::BreakStatement(_)
            | ast::Statement::ContinueStatement(_)
            | ast::Statement::DebuggerStatement(_)
            | ast::Statement::DoWhileStatement(_)
            | ast::Statement::EmptyStatement(_)
            | ast::Statement::ExpressionStatement(_)
            | ast::Statement::IfStatement(_)
            | ast::Statement::LabeledStatement(_)
            | ast::Statement::ReturnStatement(_)
            | ast::Statement::SwitchStatement(_)
            | ast::Statement::ThrowStatement(_)
            | ast::Statement::TryStatement(_)
            | ast::Statement::WhileStatement(_)
            | ast::Statement::WithStatement(_)
            | ast::Statement::TSTypeAliasDeclaration(_)
            | ast::Statement::TSInterfaceDeclaration(_)
            | ast::Statement::TSEnumDeclaration(_)
            | ast::Statement::TSModuleDeclaration(_)
            | ast::Statement::TSGlobalDeclaration(_)
            | ast::Statement::TSImportEqualsDeclaration(_)
            | ast::Statement::ExportDefaultDeclaration(_)
            | ast::Statement::TSExportAssignment(_)
            | ast::Statement::TSNamespaceExportDeclaration(_)
            | ast::Statement::ImportDeclaration(_)
            | ast::Statement::ExportAllDeclaration(_)
            | ast::Statement::ExportNamedDeclaration(_) => {}
        }
    }

    fn collect_shadow_for_statement_init(
        &self,
        init: Option<&ast::ForStatementInit<'_>>,
        bindings: &mut BTreeSet<Box<str>>,
    ) {
        if let Some(ast::ForStatementInit::VariableDeclaration(declaration)) = init {
            self.collect_shadow_variable_declaration(declaration, bindings);
        }
    }

    fn collect_shadow_for_statement_left(
        &self,
        left: &ast::ForStatementLeft<'_>,
        bindings: &mut BTreeSet<Box<str>>,
    ) {
        if let ast::ForStatementLeft::VariableDeclaration(declaration) = left {
            self.collect_shadow_variable_declaration(declaration, bindings);
        }
    }

    fn collect_shadow_variable_declaration(
        &self,
        declaration: &ast::VariableDeclaration<'_>,
        bindings: &mut BTreeSet<Box<str>>,
    ) {
        for declarator in &declaration.declarations {
            self.collect_shadow_binding_pattern(&declarator.id, bindings);
        }
    }

    fn collect_scope_variable_declaration(
        &self,
        declaration: &ast::VariableDeclaration<'_>,
        scope: &mut StaticScope,
    ) {
        for declarator in &declaration.declarations {
            self.collect_scope_variable_declarator(declaration.kind, declarator, scope);
        }
    }

    fn collect_scope_variable_declarator(
        &self,
        kind: ast::VariableDeclarationKind,
        declarator: &ast::VariableDeclarator<'_>,
        scope: &mut StaticScope,
    ) {
        let Some(name) = binding_identifier_name(&declarator.id) else {
            self.collect_shadow_binding_pattern(&declarator.id, &mut scope.shadows);
            return;
        };
        let Some(init) = &declarator.init else {
            self.collect_shadow_name(name, scope);
            return;
        };
        if kind == ast::VariableDeclarationKind::Const {
            if let Some(value) = string_constant_expression(init) {
                scope.strings.insert(Box::<str>::from(name), value);
                return;
            }
            let visible_strings = self.visible_string_bindings_with(scope);
            if let Some(value) = const_function_specifier(init, &visible_strings) {
                scope.functions.insert(Box::<str>::from(name), value);
                return;
            }
        }

        self.collect_shadow_name(name, scope);
    }

    fn collect_shadow_name(&self, name: &str, scope: &mut StaticScope) {
        if self.static_binding_exists_in_outer_scope(name) {
            scope.shadows.insert(Box::<str>::from(name));
        }
    }

    fn visible_string_bindings_with(&self, scope: &StaticScope) -> BTreeMap<Box<str>, Box<str>> {
        let mut bindings = self.const_string_bindings.clone();
        for active_scope in &self.scopes {
            for shadowed in &active_scope.shadows {
                bindings.remove(shadowed.as_ref());
            }
            bindings.extend(active_scope.strings.clone());
        }
        for shadowed in &scope.shadows {
            bindings.remove(shadowed.as_ref());
        }
        bindings.extend(scope.strings.clone());
        bindings
    }

    fn static_binding_exists_in_outer_scope(&self, name: &str) -> bool {
        if self.const_string_bindings.contains_key(name)
            || self.const_function_bindings.contains_key(name)
        {
            return true;
        }
        self.scopes
            .iter()
            .rev()
            .any(|scope| scope.strings.contains_key(name) || scope.functions.contains_key(name))
    }

    fn collect_shadow_binding_pattern(
        &self,
        pattern: &ast::BindingPattern<'_>,
        bindings: &mut BTreeSet<Box<str>>,
    ) {
        match pattern {
            ast::BindingPattern::BindingIdentifier(identifier) => {
                self.collect_shadow_binding_identifier(Some(identifier), bindings);
            }
            ast::BindingPattern::ObjectPattern(pattern) => {
                for property in &pattern.properties {
                    self.collect_shadow_binding_pattern(&property.value, bindings);
                }
                if let Some(rest) = &pattern.rest {
                    self.collect_shadow_binding_pattern(&rest.argument, bindings);
                }
            }
            ast::BindingPattern::ArrayPattern(pattern) => {
                for element_pattern in pattern.elements.iter().flatten() {
                    self.collect_shadow_binding_pattern(element_pattern, bindings);
                }
                if let Some(rest) = &pattern.rest {
                    self.collect_shadow_binding_pattern(&rest.argument, bindings);
                }
            }
            ast::BindingPattern::AssignmentPattern(pattern) => {
                self.collect_shadow_binding_pattern(&pattern.left, bindings);
            }
        }
    }

    fn collect_shadow_binding_identifier(
        &self,
        identifier: Option<&ast::BindingIdentifier<'_>>,
        bindings: &mut BTreeSet<Box<str>>,
    ) {
        let Some(identifier) = identifier else {
            return;
        };
        let name = identifier.name.as_str();
        if self.static_binding_exists_in_outer_scope(name) {
            bindings.insert(Box::<str>::from(name));
        }
    }
}

fn collect_top_level_const_string_bindings(
    program: &ast::Program<'_>,
) -> BTreeMap<Box<str>, Box<str>> {
    let mut bindings = BTreeMap::<Box<str>, Box<str>>::new();

    for statement in &program.body {
        let ast::Statement::VariableDeclaration(declaration) = statement else {
            continue;
        };
        if declaration.kind != ast::VariableDeclarationKind::Const {
            continue;
        }

        for declarator in &declaration.declarations {
            let Some(name) = binding_identifier_name(&declarator.id) else {
                continue;
            };
            let Some(init) = &declarator.init else {
                continue;
            };
            if let Some(value) = string_constant_expression(init) {
                bindings.insert(Box::<str>::from(name), value);
            }
        }
    }

    bindings
}

fn collect_top_level_const_function_bindings(
    program: &ast::Program<'_>,
    const_string_bindings: &BTreeMap<Box<str>, Box<str>>,
) -> BTreeMap<Box<str>, Box<str>> {
    let mut bindings = BTreeMap::<Box<str>, Box<str>>::new();

    for statement in &program.body {
        let ast::Statement::VariableDeclaration(declaration) = statement else {
            continue;
        };
        if declaration.kind != ast::VariableDeclarationKind::Const {
            continue;
        }

        for declarator in &declaration.declarations {
            let Some(name) = binding_identifier_name(&declarator.id) else {
                continue;
            };
            let Some(init) = &declarator.init else {
                continue;
            };
            if let Some(value) = const_function_specifier(init, const_string_bindings) {
                bindings.insert(Box::<str>::from(name), value);
            }
        }
    }

    bindings
}

fn binding_identifier_name<'source>(
    binding: &'source ast::BindingPattern<'_>,
) -> Option<&'source str> {
    match binding {
        ast::BindingPattern::BindingIdentifier(identifier) => Some(identifier.name.as_str()),
        ast::BindingPattern::ObjectPattern(_)
        | ast::BindingPattern::ArrayPattern(_)
        | ast::BindingPattern::AssignmentPattern(_) => None,
    }
}

fn const_function_specifier(
    expression: &ast::Expression<'_>,
    const_string_bindings: &BTreeMap<Box<str>, Box<str>>,
) -> Option<Box<str>> {
    match expression {
        ast::Expression::ArrowFunctionExpression(function) => {
            let return_expression = arrow_function_return_expression(function)?;
            static_specifier_expression(return_expression, const_string_bindings)
        }
        ast::Expression::ParenthesizedExpression(expression) => {
            const_function_specifier(&expression.expression, const_string_bindings)
        }
        ast::Expression::TSAsExpression(expression) => {
            const_function_specifier(&expression.expression, const_string_bindings)
        }
        ast::Expression::TSSatisfiesExpression(expression) => {
            const_function_specifier(&expression.expression, const_string_bindings)
        }
        ast::Expression::TSTypeAssertion(expression) => {
            const_function_specifier(&expression.expression, const_string_bindings)
        }
        ast::Expression::TSNonNullExpression(expression) => {
            const_function_specifier(&expression.expression, const_string_bindings)
        }
        ast::Expression::StringLiteral(_)
        | ast::Expression::BooleanLiteral(_)
        | ast::Expression::NullLiteral(_)
        | ast::Expression::NumericLiteral(_)
        | ast::Expression::BigIntLiteral(_)
        | ast::Expression::RegExpLiteral(_)
        | ast::Expression::TemplateLiteral(_)
        | ast::Expression::Identifier(_)
        | ast::Expression::MetaProperty(_)
        | ast::Expression::Super(_)
        | ast::Expression::ArrayExpression(_)
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
        | ast::Expression::SequenceExpression(_)
        | ast::Expression::TaggedTemplateExpression(_)
        | ast::Expression::ThisExpression(_)
        | ast::Expression::UnaryExpression(_)
        | ast::Expression::UpdateExpression(_)
        | ast::Expression::YieldExpression(_)
        | ast::Expression::PrivateInExpression(_)
        | ast::Expression::JSXElement(_)
        | ast::Expression::JSXFragment(_)
        | ast::Expression::TSInstantiationExpression(_)
        | ast::Expression::ComputedMemberExpression(_)
        | ast::Expression::StaticMemberExpression(_)
        | ast::Expression::PrivateFieldExpression(_)
        | ast::Expression::V8IntrinsicExpression(_) => None,
    }
}

fn arrow_function_return_expression<'source>(
    function: &'source ast::ArrowFunctionExpression<'_>,
) -> Option<&'source ast::Expression<'source>> {
    if !function.expression || !function.params.items.is_empty() || function.params.rest.is_some() {
        return None;
    }

    let mut statements = function.body.statements.iter();
    let statement = statements.next()?;
    if statements.next().is_some() {
        return None;
    }

    let ast::Statement::ExpressionStatement(statement) = statement else {
        return None;
    };

    Some(&statement.expression)
}

fn static_specifier_expression(
    expression: &ast::Expression<'_>,
    const_string_bindings: &BTreeMap<Box<str>, Box<str>>,
) -> Option<Box<str>> {
    match expression {
        ast::Expression::StringLiteral(literal) => Some(strip_query(literal.value.as_str())),
        ast::Expression::TemplateLiteral(template) => {
            static_template_specifier(template, const_string_bindings)
        }
        ast::Expression::CallExpression(expression) => {
            path_resolve_import_meta_dir_specifier(expression)
        }
        ast::Expression::ParenthesizedExpression(expression) => {
            static_specifier_expression(&expression.expression, const_string_bindings)
        }
        ast::Expression::TSAsExpression(expression) => {
            static_specifier_expression(&expression.expression, const_string_bindings)
        }
        ast::Expression::TSSatisfiesExpression(expression) => {
            static_specifier_expression(&expression.expression, const_string_bindings)
        }
        ast::Expression::TSTypeAssertion(expression) => {
            static_specifier_expression(&expression.expression, const_string_bindings)
        }
        ast::Expression::TSNonNullExpression(expression) => {
            static_specifier_expression(&expression.expression, const_string_bindings)
        }
        ast::Expression::BooleanLiteral(_)
        | ast::Expression::NullLiteral(_)
        | ast::Expression::NumericLiteral(_)
        | ast::Expression::BigIntLiteral(_)
        | ast::Expression::RegExpLiteral(_)
        | ast::Expression::Identifier(_)
        | ast::Expression::MetaProperty(_)
        | ast::Expression::Super(_)
        | ast::Expression::ArrayExpression(_)
        | ast::Expression::ArrowFunctionExpression(_)
        | ast::Expression::AssignmentExpression(_)
        | ast::Expression::AwaitExpression(_)
        | ast::Expression::BinaryExpression(_)
        | ast::Expression::ChainExpression(_)
        | ast::Expression::ClassExpression(_)
        | ast::Expression::ConditionalExpression(_)
        | ast::Expression::FunctionExpression(_)
        | ast::Expression::ImportExpression(_)
        | ast::Expression::LogicalExpression(_)
        | ast::Expression::NewExpression(_)
        | ast::Expression::ObjectExpression(_)
        | ast::Expression::SequenceExpression(_)
        | ast::Expression::TaggedTemplateExpression(_)
        | ast::Expression::ThisExpression(_)
        | ast::Expression::UnaryExpression(_)
        | ast::Expression::UpdateExpression(_)
        | ast::Expression::YieldExpression(_)
        | ast::Expression::PrivateInExpression(_)
        | ast::Expression::JSXElement(_)
        | ast::Expression::JSXFragment(_)
        | ast::Expression::TSInstantiationExpression(_)
        | ast::Expression::ComputedMemberExpression(_)
        | ast::Expression::StaticMemberExpression(_)
        | ast::Expression::PrivateFieldExpression(_)
        | ast::Expression::V8IntrinsicExpression(_) => None,
    }
}

fn static_template_specifier(
    template: &ast::TemplateLiteral<'_>,
    const_string_bindings: &BTreeMap<Box<str>, Box<str>>,
) -> Option<Box<str>> {
    let first_quasi = template.quasis.first()?;
    let first_text = template_element_text(first_quasi);
    if let Some((specifier, _query)) = first_text.split_once('?') {
        return Some(Box::<str>::from(specifier));
    }

    if !first_text.is_empty() {
        return None;
    }

    let first_expression = template.expressions.first()?;
    let ast::Expression::Identifier(identifier) = first_expression else {
        return None;
    };

    let bound_specifier = const_string_bindings.get(identifier.name.as_str())?;
    let second_quasi = template.quasis.get(1)?;
    let second_text = template_element_text(second_quasi);
    if second_text.starts_with('?') {
        Some(bound_specifier.clone())
    } else {
        None
    }
}

fn string_constant_expression(expression: &ast::Expression<'_>) -> Option<Box<str>> {
    match expression {
        ast::Expression::StringLiteral(literal) => Some(strip_query(literal.value.as_str())),
        ast::Expression::CallExpression(expression) => {
            path_resolve_import_meta_dir_specifier(expression)
        }
        ast::Expression::TSAsExpression(expression) => {
            string_constant_expression(&expression.expression)
        }
        ast::Expression::TSSatisfiesExpression(expression) => {
            string_constant_expression(&expression.expression)
        }
        ast::Expression::TSTypeAssertion(expression) => {
            string_constant_expression(&expression.expression)
        }
        ast::Expression::TSNonNullExpression(expression) => {
            string_constant_expression(&expression.expression)
        }
        ast::Expression::ParenthesizedExpression(expression) => {
            string_constant_expression(&expression.expression)
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
        | ast::Expression::ChainExpression(_)
        | ast::Expression::ClassExpression(_)
        | ast::Expression::ConditionalExpression(_)
        | ast::Expression::FunctionExpression(_)
        | ast::Expression::ImportExpression(_)
        | ast::Expression::LogicalExpression(_)
        | ast::Expression::NewExpression(_)
        | ast::Expression::ObjectExpression(_)
        | ast::Expression::SequenceExpression(_)
        | ast::Expression::TaggedTemplateExpression(_)
        | ast::Expression::ThisExpression(_)
        | ast::Expression::UnaryExpression(_)
        | ast::Expression::UpdateExpression(_)
        | ast::Expression::YieldExpression(_)
        | ast::Expression::PrivateInExpression(_)
        | ast::Expression::JSXElement(_)
        | ast::Expression::JSXFragment(_)
        | ast::Expression::TSInstantiationExpression(_)
        | ast::Expression::ComputedMemberExpression(_)
        | ast::Expression::StaticMemberExpression(_)
        | ast::Expression::PrivateFieldExpression(_)
        | ast::Expression::V8IntrinsicExpression(_) => None,
    }
}

fn template_element_text(element: &ast::TemplateElement<'_>) -> Box<str> {
    element.value.cooked.as_ref().map_or_else(
        || Box::<str>::from(element.value.raw.as_str()),
        |cooked| Box::<str>::from(cooked.as_str()),
    )
}

fn path_resolve_import_meta_dir_specifier(call: &ast::CallExpression<'_>) -> Option<Box<str>> {
    if !is_resolve_call(call) {
        return None;
    }

    let mut arguments = call.arguments.iter();
    let first_argument = arguments.next()?;
    let second_argument = arguments.next()?;
    if arguments.next().is_some() || !is_import_meta_dir_argument(first_argument) {
        return None;
    }

    string_argument_expression(second_argument)
}

fn is_resolve_call(call: &ast::CallExpression<'_>) -> bool {
    let ast::Expression::Identifier(callee) = &call.callee else {
        return false;
    };

    callee.name.as_str() == "resolve"
}

fn is_import_meta_dir_argument(argument: &ast::Argument<'_>) -> bool {
    let ast::Argument::StaticMemberExpression(member) = argument else {
        return false;
    };
    if member.property.name.as_str() != "dir" {
        return false;
    }
    let ast::Expression::MetaProperty(meta) = &member.object else {
        return false;
    };

    meta.meta.name.as_str() == "import" && meta.property.name.as_str() == "meta"
}

fn string_argument_expression(argument: &ast::Argument<'_>) -> Option<Box<str>> {
    match argument {
        ast::Argument::StringLiteral(literal) => Some(strip_query(literal.value.as_str())),
        ast::Argument::ParenthesizedExpression(expression) => {
            string_constant_expression(&expression.expression)
        }
        ast::Argument::TSAsExpression(expression) => {
            string_constant_expression(&expression.expression)
        }
        ast::Argument::TSSatisfiesExpression(expression) => {
            string_constant_expression(&expression.expression)
        }
        ast::Argument::TSTypeAssertion(expression) => {
            string_constant_expression(&expression.expression)
        }
        ast::Argument::TSNonNullExpression(expression) => {
            string_constant_expression(&expression.expression)
        }
        ast::Argument::SpreadElement(_)
        | ast::Argument::BooleanLiteral(_)
        | ast::Argument::NullLiteral(_)
        | ast::Argument::NumericLiteral(_)
        | ast::Argument::BigIntLiteral(_)
        | ast::Argument::RegExpLiteral(_)
        | ast::Argument::TemplateLiteral(_)
        | ast::Argument::Identifier(_)
        | ast::Argument::MetaProperty(_)
        | ast::Argument::Super(_)
        | ast::Argument::ArrayExpression(_)
        | ast::Argument::ArrowFunctionExpression(_)
        | ast::Argument::AssignmentExpression(_)
        | ast::Argument::AwaitExpression(_)
        | ast::Argument::BinaryExpression(_)
        | ast::Argument::CallExpression(_)
        | ast::Argument::ChainExpression(_)
        | ast::Argument::ClassExpression(_)
        | ast::Argument::ConditionalExpression(_)
        | ast::Argument::FunctionExpression(_)
        | ast::Argument::ImportExpression(_)
        | ast::Argument::LogicalExpression(_)
        | ast::Argument::NewExpression(_)
        | ast::Argument::ObjectExpression(_)
        | ast::Argument::SequenceExpression(_)
        | ast::Argument::TaggedTemplateExpression(_)
        | ast::Argument::ThisExpression(_)
        | ast::Argument::UnaryExpression(_)
        | ast::Argument::UpdateExpression(_)
        | ast::Argument::YieldExpression(_)
        | ast::Argument::PrivateInExpression(_)
        | ast::Argument::JSXElement(_)
        | ast::Argument::JSXFragment(_)
        | ast::Argument::TSInstantiationExpression(_)
        | ast::Argument::ComputedMemberExpression(_)
        | ast::Argument::StaticMemberExpression(_)
        | ast::Argument::PrivateFieldExpression(_)
        | ast::Argument::V8IntrinsicExpression(_) => None,
    }
}

fn strip_query(value: &str) -> Box<str> {
    match value.split_once('?') {
        Some((specifier, _query)) => Box::<str>::from(specifier),
        None => Box::<str>::from(value),
    }
}
