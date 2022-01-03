use std::{
    cell::{Ref, RefCell},
    fmt::{Debug, Display, Formatter},
    rc::{Rc, Weak},
};

use crate::{
    ast::{AstId, ItemP},
    common::CodeErrorKind,
    parser::ParseCtx,
};
use indexmap::{map::Entry, IndexMap};
use once_cell::unsync::OnceCell;
use tree_sitter::Node;

use super::path::{Path, PathSegment};

#[derive(Debug, Clone)]
pub enum NamedItem<'ast, 'src> {
    Alias(Path<'ast>),
    Function(ItemP<'ast>, Node<'src>, Scope<'ast, 'src>),
    Static(ItemP<'ast>, Node<'src>),
    Const(ItemP<'ast>, Node<'src>),
    Macro(ItemP<'ast>, Node<'src>, Scope<'ast, 'src>),
    Type(ItemP<'ast>, Node<'src>, Scope<'ast, 'src>),
    Mixin(Node<'src>, Scope<'ast, 'src>),
    Module(Scope<'ast, 'src>),
    Protocol(ItemP<'ast>, Node<'src>, Scope<'ast, 'src>),
    Impl(Node<'src>, Scope<'ast, 'src>),
    Placeholder(AstId, Node<'src>),
    Field(Node<'src>),
    EnumMember(ItemP<'ast>, AstId, Node<'src>),
    Local(AstId),
    Parameter(AstId, Node<'src>),
    MacroParameter(AstId, bool),
}

impl Display for NamedItem<'_, '_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            NamedItem::Alias(_) => write!(f, "alias"),
            NamedItem::Function(_, _, _) => write!(f, "function"),
            NamedItem::Static(_, _) => write!(f, "static"),
            NamedItem::Const(_, _) => write!(f, "const"),
            NamedItem::Macro(_, _, _) => write!(f, "macro"),
            NamedItem::Type(_, _, _) => write!(f, "type"),
            NamedItem::Mixin(_, _) => write!(f, "mixin"),
            NamedItem::Module(_) => write!(f, "module"),
            NamedItem::Protocol(_, _, _) => write!(f, "protocol"),
            NamedItem::Impl(_, _) => write!(f, "impl"),
            NamedItem::Placeholder(_, _) => write!(f, "placeholder"),
            NamedItem::Field(_) => write!(f, "field"),
            NamedItem::EnumMember(_, _, _) => write!(f, "enum member"),
            NamedItem::Local(_) => write!(f, "local"),
            NamedItem::Parameter(_, _) => write!(f, "parameter"),
            NamedItem::MacroParameter(_, _) => write!(f, "macro parameter"),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ScopeType {
    Root,
    Crate,
    Module,
    Protocol,
    StructLike,
    Function,
    Macro,
    Closure,
    Impl,
    Enum,
    Block,
}

pub struct ScopeInner<'ast, 'src> {
    pub r#type: ScopeType,
    pub path: Path<'ast>,

    // We use IndexMap to preserve the order of items in the scope. While not important for
    // name resolution, it is important for e.g. struct layout, function signature, generic
    // parameter ordering, etc.
    pub items: IndexMap<Option<&'ast str>, Vec<NamedItem<'ast, 'src>>>,
    pub parent: Option<Weak<RefCell<ScopeInner<'ast, 'src>>>>,

    code: OnceCell<&'src ParseCtx<'src>>,
}

impl<'ast, 'src> ScopeInner<'ast, 'src> {
    pub fn all_items<'i>(
        &'i self,
    ) -> impl Iterator<Item = (Option<&'ast str>, &'i NamedItem<'ast, 'src>)> {
        self.items
            .iter()
            .flat_map(|(n, its)| its.iter().map(|i| (*n, i)))
    }

    pub fn grouped_items<'i>(
        &'i self,
    ) -> impl Iterator<Item = (Option<&'ast str>, &'i [NamedItem<'ast, 'src>])> {
        self.items.iter().map(|(n, its)| (*n, its.as_slice()))
    }

    pub fn items_with_name<'i>(
        &'i self,
        name: &'ast str,
    ) -> impl Iterator<Item = &'i NamedItem<'ast, 'src>> {
        self.items
            .get(&Some(name))
            .into_iter()
            .flat_map(|its| its.iter())
    }
}

impl<'ast, 'src> Debug for ScopeInner<'ast, 'src> {
    fn fmt(&self, fmt: &mut Formatter) -> Result<(), std::fmt::Error> {
        let mut builder = fmt.debug_struct(&format!("{:?}Scope({:?})", self.r#type, self.path));
        for (name, items) in &self.items {
            for item in items {
                builder.field(name.unwrap_or("<unnamed>"), item);
            }
        }
        builder.finish()
    }
}

#[derive(Clone)]
pub struct Scope<'ast, 'src>(pub Rc<RefCell<ScopeInner<'ast, 'src>>>);

impl<'ast, 'src> PartialEq for Scope<'ast, 'src> {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl<'ast, 'src> From<Scope<'ast, 'src>> for Weak<RefCell<ScopeInner<'ast, 'src>>> {
    fn from(scope: Scope<'ast, 'src>) -> Self {
        Rc::downgrade(&scope.0)
    }
}

impl<'ast, 'src> Debug for Scope<'ast, 'src> {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        self.inner().fmt(fmt)
    }
}

impl<'ast, 'src> Scope<'ast, 'src> {
    pub fn new_root() -> Self {
        Scope(Rc::new(RefCell::new(ScopeInner {
            r#type: ScopeType::Root,
            path: Path::root(),
            items: IndexMap::new(),
            parent: None,
            code: OnceCell::new(),
        })))
    }

    pub fn typ(&self) -> ScopeType {
        self.0.borrow().r#type
    }

    pub fn inner(&self) -> Ref<'_, ScopeInner<'ast, 'src>> {
        self.0.borrow()
    }

    pub fn code(&self) -> Option<&'src ParseCtx<'src>> {
        self.inner().code.get().copied()
    }

    pub fn path(&self) -> Path<'ast> {
        self.inner().path.clone()
    }

    pub fn named_child(&self, r#type: ScopeType, name: &'ast str) -> Self {
        let new_path = self.0.borrow().path.extend(PathSegment(name));
        let code = self.0.borrow().code.clone();

        Scope(Rc::new(RefCell::new(ScopeInner {
            r#type,
            path: new_path,
            items: IndexMap::new(),
            code,
            parent: Some(Rc::downgrade(&self.0)),
        })))
    }

    pub fn named_child_without_code(&self, r#type: ScopeType, name: &'ast str) -> Self {
        let new_path = self.0.borrow().path.extend(PathSegment(name));

        Scope(Rc::new(RefCell::new(ScopeInner {
            r#type,
            path: new_path,
            items: IndexMap::new(),
            code: OnceCell::new(),
            parent: Some(Rc::downgrade(&self.0)),
        })))
    }

    pub fn anonymous_child(&self, r#type: ScopeType) -> Self {
        let code = self.0.borrow().code.clone();

        Scope(Rc::new(RefCell::new(ScopeInner {
            r#type,
            path: self.path(),
            items: IndexMap::new(),
            code,
            parent: Some(Rc::downgrade(&self.0)),
        })))
    }

    pub fn set_code(&self, code: &'src ParseCtx<'src>) {
        if self.0.borrow().code.set(code).is_err() {
            panic!(
                "source code context is already set for {}",
                self.0.borrow().path
            );
        }
    }

    pub fn add_item(
        &self,
        name: Option<&'ast str>,
        item: NamedItem<'ast, 'src>,
    ) -> Result<(), CodeErrorKind> {
        let mut current_scope = self.0.borrow_mut();
        let scope_type = current_scope.r#type;

        // Duplicate names are generally not allowed, but we allow them for
        // types and their impls.
        match current_scope.items.entry(name) {
            Entry::Vacant(entry) => {
                entry.insert(vec![item]);
                return Ok(());
            }
            Entry::Occupied(mut entry) => {
                let existing = entry.get_mut();
                // Unnamed items do not generate name conflicts
                if name.is_none() {
                    existing.push(item);
                    return Ok(());
                } else if let ScopeType::Block = scope_type {
                    // In linear scopes we allow shadowing.
                    existing[0] = item;
                    return Ok(());
                } else {
                    let (type_count, impl_count) =
                        existing
                            .iter()
                            .fold((0, 0), |(type_count, impl_count), item| match item {
                                NamedItem::Type(_, _, _) => (type_count + 1, impl_count),
                                NamedItem::Impl(_, _) => (type_count, impl_count + 1),
                                _ => (type_count, impl_count),
                            });

                    if ((type_count == 1 || impl_count > 0)
                        && matches!(item, NamedItem::Impl(_, _)))
                        || (type_count == 0
                            && impl_count > 0
                            && matches!(item, NamedItem::Type(_, _, _)))
                    {
                        existing.push(item);
                        existing.sort_by_key(|i| match i {
                            NamedItem::Type(_, _, _) => 0,
                            NamedItem::Impl(_, _) => 1,
                            _ => unreachable!(),
                        });
                        return Ok(());
                    }
                }
            }
        }

        Err(CodeErrorKind::DuplicateName(name.unwrap().into()))
    }

    pub fn find_root(&self) -> Self {
        let mut current = self.clone();
        while let Some(parent) = current.parent() {
            current = parent;
        }
        current
    }

    pub fn find_crate(&self) -> Option<Self> {
        let mut current = self.clone();

        loop {
            let r#type = current.0.borrow().r#type;
            if let ScopeType::Crate = r#type {
                return Some(current);
            }

            if let Some(parent) = current.parent() {
                current = parent;
            } else {
                break;
            }
        }

        None
    }

    pub fn find_super(&self) -> Option<Self> {
        // Function, struct, enum, ... are transparently scoped to their parent
        match self.0.borrow().r#type {
            ScopeType::Root | ScopeType::Crate => None,
            ScopeType::Module => self.parent(),
            _ => self.parent().and_then(|p| p.find_super()),
        }
    }

    pub fn find_containing_function(&self) -> Option<Self> {
        match self.0.borrow().r#type {
            ScopeType::Closure | ScopeType::Function => Some(self.clone()),
            _ => self.parent().and_then(|p| p.find_containing_function()),
        }
    }

    pub fn parent(&self) -> Option<Self> {
        self.inner()
            .parent
            .as_ref()
            .map(|parent| Self(parent.upgrade().unwrap()))
    }

    pub fn ensure_module(&self, path: Path<'ast>) -> Result<Scope<'ast, 'src>, CodeErrorKind> {
        if path.absolute {
            return self.find_root().ensure_module(Path {
                absolute: false,
                segments: path.segments.clone(),
            });
        }

        if path.segments.is_empty() {
            return Ok(self.clone());
        }

        let remainder = Path {
            absolute: false,
            segments: path.segments[1..].to_vec(),
        };

        for item in self.inner().items_with_name(path.segments[0].0) {
            if let NamedItem::Module(child_scope) = item {
                return child_scope.ensure_module(remainder);
            }
        }

        let scope_type = match self.parent() {
            None => ScopeType::Crate,
            Some(_) => ScopeType::Module,
        };

        let child_scope = self.named_child_without_code(scope_type, path.segments[0].0);
        self.add_item(
            Some(path.segments[0].0),
            NamedItem::Module(child_scope.clone()),
        )?;

        child_scope.ensure_module(remainder)
    }
}
