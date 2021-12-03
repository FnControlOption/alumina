use crate::{
    ast::{self, AstId, BuiltinType, Field, Function, Item, ItemP, Parameter, Struct, Ty},
    common::{ArenaAllocatable, SyntaxError},
    context::AstCtx,
    name_resolution::scope::{NamedItem, Scope},
    parser::AluminaVisitor,
};

use super::{expressions::ExpressionVisitor, types::TypeVisitor};

pub struct Maker<'ast> {
    ast: &'ast AstCtx<'ast>,
    pub symbols: Vec<ItemP<'ast>>,
}

impl<'ast> Maker<'ast> {
    pub fn new(ast: &'ast AstCtx<'ast>) -> Self {
        Self {
            ast,
            symbols: Vec::new(),
        }
    }

    fn resolve_associated_fns<'src>(
        &self,
        scope: Scope<'ast, 'src>,
    ) -> Result<&'ast [ItemP<'ast>], SyntaxError<'src>> {
        let mut associated_fns: Vec<ItemP<'ast>> = Vec::new();

        for (_name, item) in scope.inner().all_items() {
            match item {
                NamedItem::Function(symbol, _, _) => associated_fns.push(*symbol),
                _ => {}
            }
        }

        let result = associated_fns.alloc_on(self.ast);
        Ok(result)
    }

    fn make_struct<'src>(
        &mut self,
        symbol: ItemP<'ast>,
        _node: tree_sitter::Node<'src>,
        scope: Scope<'ast, 'src>,
        impl_scope: Option<Scope<'ast, 'src>>,
    ) -> Result<(), SyntaxError<'src>> {
        let mut placeholders: Vec<AstId> = Vec::new();
        let mut fields: Vec<Field<'ast>> = Vec::new();

        for (name, item) in scope.inner().all_items() {
            match item {
                NamedItem::Placeholder(placeholder) => {
                    placeholders.push(*placeholder);
                }
                NamedItem::Field(node) => {
                    let mut visitor = TypeVisitor::new(self.ast, scope.clone());
                    let field_type = visitor.visit(node.child_by_field_name("type").unwrap())?;

                    fields.push(Field {
                        name: name.alloc_on(self.ast),
                        ty: field_type,
                    });
                }
                _ => {}
            }
        }

        let associated_fns = match impl_scope {
            Some(impl_scope) => self.resolve_associated_fns(impl_scope)?,
            None => (&[]).alloc_on(self.ast),
        };

        let result = Item::Struct(Struct {
            placeholders: placeholders.alloc_on(self.ast),
            fields: fields.alloc_on(self.ast),
            associated_fns,
        });

        symbol.assign(result);

        self.symbols.push(symbol);

        Ok(())
    }

    fn make_function_impl<'src>(
        &mut self,
        symbol: ItemP<'ast>,
        node: tree_sitter::Node<'src>,
        scope: Scope<'ast, 'src>,
        body: Option<tree_sitter::Node<'src>>,
    ) -> Result<(), SyntaxError<'src>> {
        let mut placeholders: Vec<AstId> = Vec::new();
        let mut parameters: Vec<Parameter<'ast>> = Vec::new();

        for (_name, item) in scope.inner().all_items() {
            match item {
                NamedItem::Placeholder(placeholder) => {
                    placeholders.push(*placeholder);
                }
                NamedItem::Parameter(id, node) => {
                    let field_type = TypeVisitor::new(self.ast, scope.clone())
                        .visit(node.child_by_field_name("type").unwrap())?;

                    parameters.push(Parameter {
                        id: *id,
                        ty: field_type,
                    });
                }
                _ => {}
            }
        }

        let return_type = node
            .child_by_field_name("return_type")
            .map(|n| TypeVisitor::new(self.ast, scope.clone()).visit(n))
            .transpose()?
            .unwrap_or(self.ast.intern_type(Ty::Builtin(BuiltinType::Void)));

        let function_body = body
            .map(|body| ExpressionVisitor::new(self.ast, scope.clone()).visit(body))
            .transpose()?;

        let result = Item::Function(Function {
            placeholders: placeholders.alloc_on(self.ast),
            parameters: parameters.alloc_on(self.ast),
            return_type,
            body: function_body,
        });

        symbol.assign(result);

        self.symbols.push(symbol);

        Ok(())
    }

    fn make_type<'src>(
        &mut self,
        symbol: ItemP<'ast>,
        node: tree_sitter::Node<'src>,
        scope: Scope<'ast, 'src>,
        impl_scope: Option<Scope<'ast, 'src>>,
    ) -> Result<(), SyntaxError<'src>> {
        match node.kind() {
            "struct_definition" => self.make_struct(symbol, node, scope, impl_scope)?,
            _ => unimplemented!(),
        };

        Ok(())
    }

    fn make_function<'src>(
        &mut self,
        symbol: ItemP<'ast>,
        node: tree_sitter::Node<'src>,
        scope: Scope<'ast, 'src>,
    ) -> Result<(), SyntaxError<'src>> {
        match node.kind() {
            "function_definition" => self.make_function_impl(
                symbol,
                node,
                scope,
                Some(node.child_by_field_name("body").unwrap()),
            )?,
            "extern_function_declaration" => self.make_function_impl(symbol, node, scope, None)?,
            _ => unimplemented!(),
        };

        Ok(())
    }

    pub fn make<'src>(&mut self, scope: Scope<'ast, 'src>) -> Result<(), SyntaxError<'src>> {
        for (_, item) in scope.inner().grouped_items() {
            match item {
                [NamedItem::Module(module)] => {
                    self.make(module.clone())?;
                }
                [NamedItem::Type(symbol, node, scope), NamedItem::Impl(impl_scope)] => {
                    self.make_type(*symbol, *node, scope.clone(), Some(impl_scope.clone()))?;
                    self.make(impl_scope.clone())?;
                }
                [NamedItem::Type(symbol, node, scope)] => {
                    self.make_type(*symbol, *node, scope.clone(), None)?;
                }
                [NamedItem::Function(symbol, node, scope)] => {
                    self.make_function(*symbol, *node, scope.clone())?;
                }
                _ => {}
            }
        }

        Ok(())
    }
}
