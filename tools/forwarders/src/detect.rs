// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The forwarder test over one parsed file.
//!
//! A forwarder is a free function or an inherent method whose body is one
//! call, wrapped at most in `.await`, `?`, `Ok(..)`, `Some(..)`, `.into()` or
//! a `.map_err` that only converts the error. The call is to a function or to
//! a method of `self`, and its arguments are ones a caller could pass itself:
//! the parameters, literals and constants, and references, dereferences,
//! borrows and clones of those. A call that reads a field of `self`, or a
//! method of a field or a parameter, reaches what the caller cannot, so it is
//! an accessor and never reported.
//!
//! No specification governs this: our own design (#4663).

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use quote::ToTokens;
use syn::{
    Attribute, Block, Expr, ExprLit, ExprPath, FnArg, ImplItem, Item, Lit, Meta, Pat, ReturnType,
    Signature, Stmt, Type, UnOp,
};

/// The methods that only borrow or copy a value an operand may pass through.
const BORROWS: &[&str] = &[
    "as_deref",
    "as_deref_mut",
    "as_mut",
    "as_ref",
    "as_slice",
    "as_str",
    "borrow",
    "borrow_mut",
    "clone",
];

/// One forwarder: who forwards, to what, and the line its name stands on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Forwarder {
    /// The inline modules, the impl type and the function name, joined by `::`.
    pub(crate) owner: String,
    /// The call the body makes, as compact source text.
    pub(crate) target: String,
    /// The 1-based line of the function's name.
    pub(crate) line: usize,
}

/// What one file holds: its forwarders, and the modules it compiles only for
/// tests, as paths of module names below the file's own module.
#[derive(Debug, Default)]
pub(crate) struct FileReport {
    /// The forwarders, in source order.
    pub(crate) forwarders: Vec<Forwarder>,
    /// Each `#[cfg(test)] mod` the file declares, inline or out of line.
    pub(crate) test_modules: Vec<Vec<String>>,
}

/// Lists the forwarders and the test-only modules of one parsed file.
pub(crate) fn scan(file: &syn::File) -> FileReport {
    let mut report = FileReport::default();
    visit(&file.items, &mut Vec::new(), &mut report);
    report
}

fn visit(items: &[Item], path: &mut Vec<String>, report: &mut FileReport) {
    // Each function of this module with the impl type that owns it, if any.
    let mut functions: Vec<(Option<String>, &Signature, &Block, &[Attribute])> = Vec::new();
    for item in items {
        match item {
            // A binary's `main` is the entry point the language requires.
            Item::Fn(function) if function.sig.ident != "main" => {
                functions.push((None, &function.sig, &function.block, &function.attrs));
            }
            // A trait implementation's methods are the ones the trait requires.
            Item::Impl(block) if block.trait_.is_none() && !is_test(&block.attrs) => {
                let ty = type_name(&block.self_ty);
                for member in &block.items {
                    if let ImplItem::Fn(method) = member {
                        functions.push((
                            Some(ty.clone()),
                            &method.sig,
                            &method.block,
                            &method.attrs,
                        ));
                    }
                }
            }
            Item::Mod(module) => {
                path.push(module.ident.to_string());
                if is_test(&module.attrs) {
                    report.test_modules.push(path.clone());
                } else if let Some((_, items)) = &module.content {
                    visit(items, path, report);
                }
                path.pop();
            }
            _ => {}
        }
    }
    for (ty, sig, block, attrs) in &functions {
        // Two functions of one name in one scope are the `#[cfg]` arms of one
        // function, and the one that forwards stands in for the other.
        let arms = functions
            .iter()
            .filter(|(other, other_sig, ..)| other == ty && other_sig.ident == sig.ident)
            .count();
        if arms > 1 || skipped(attrs) {
            continue;
        }
        if let Some(target) = forwarded(sig, block, ty.as_deref()) {
            report.forwarders.push(Forwarder {
                owner: owner(path, ty.as_deref(), sig),
                target,
                line: sig.ident.span().start().line,
            });
        }
    }
}

fn owner(path: &[String], ty: Option<&str>, sig: &Signature) -> String {
    let mut parts: Vec<&str> = path.iter().map(String::as_str).collect();
    parts.extend(ty);
    let name = sig.ident.to_string();
    parts.push(&name);
    parts.join("::")
}

fn type_name(ty: &Type) -> String {
    match ty {
        Type::Path(path) if path.qself.is_none() => path
            .path
            .segments
            .last()
            .map_or_else(|| compact(ty), |segment| segment.ident.to_string()),
        _ => compact(ty),
    }
}

/// True for an item compiled only under test, and for a function an openEHR
/// specification defines: its doc comment cites the `.adoc` page, and the
/// specification needs it the way a trait needs its methods.
fn skipped(attrs: &[Attribute]) -> bool {
    is_test(attrs) || attrs.iter().any(cites_specification)
}

fn cites_specification(attr: &Attribute) -> bool {
    let Meta::NameValue(doc) = &attr.meta else {
        return false;
    };
    doc.path.is_ident("doc")
        && matches!(&doc.value, Expr::Lit(ExprLit { lit: Lit::Str(text), .. }) if text.value().contains(".adoc"))
}

/// True for an item compiled only under test: `#[test]`, a test attribute of
/// a runtime such as `#[tokio::test]`, `#[cfg(test)]` or `#[cfg(all(.., test, ..))]`.
fn is_test(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let path = attr.path();
        if path
            .segments
            .last()
            .is_some_and(|last| last.ident == "test")
        {
            return true;
        }
        if !path.is_ident("cfg") {
            return false;
        }
        let Ok(list) = attr.meta.require_list() else {
            return false;
        };
        requires_test(list.tokens.clone())
    })
}

fn requires_test(predicate: TokenStream) -> bool {
    let tokens: Vec<TokenTree> = predicate.into_iter().collect();
    match tokens.as_slice() {
        [TokenTree::Ident(ident)] => ident == "test",
        [TokenTree::Ident(ident), TokenTree::Group(group)]
            if ident == "all" && group.delimiter() == Delimiter::Parenthesis =>
        {
            group
                .stream()
                .into_iter()
                .collect::<Vec<_>>()
                .split(|token| matches!(token, TokenTree::Punct(punct) if punct.as_char() == ','))
                .any(|part| requires_test(part.iter().cloned().collect()))
        }
        _ => false,
    }
}

/// The call a function forwards to, as compact text, or `None` when its body
/// does more than forward.
fn forwarded(sig: &Signature, block: &Block, ty: Option<&str>) -> Option<String> {
    let [Stmt::Expr(body, _)] = block.stmts.as_slice() else {
        return None;
    };
    let params = parameters(sig);
    let core = peel(body);
    let args = match core {
        Expr::Call(call) => {
            let Expr::Path(func) = &*call.func else {
                return None;
            };
            let last = func.path.segments.last()?.ident.to_string();
            // A capitalised callee is a tuple struct or an enum variant: a value, not a call.
            if capitalised(&last) {
                return None;
            }
            // A closure handed in as a parameter is called, not forwarded to.
            if func.qself.is_none() && func.path.segments.len() == 1 && params.contains(&last) {
                return None;
            }
            &call.args
        }
        // A method of a field or a parameter belongs to that value's type, and a
        // private field is out of the caller's reach: only a call on `self` forwards.
        Expr::MethodCall(call) if is_self(&call.receiver) => &call.args,
        _ => return None,
    };
    if !args.iter().all(|arg| operand(arg, &params)) {
        return None;
    }
    if constructs(sig, ty) && args.iter().any(fixed) {
        return None;
    }
    Some(compact(core))
}

/// A constructor: `new`, a `with_*`, or an associated function of the impl
/// type `ty` that takes no `self` and returns `Self` or that type.
fn constructs(sig: &Signature, ty: Option<&str>) -> bool {
    let name = sig.ident.to_string();
    if name == "new" || name.starts_with("with_") {
        return true;
    }
    let (Some(ty), None, ReturnType::Type(_, output)) = (ty, sig.receiver(), &sig.output) else {
        return false;
    };
    matches!(&**output, Type::Path(out) if out.qself.is_none()
        && out.path.segments.last().is_some_and(|last| last.ident == "Self" || last.ident == ty))
}

/// The names the signature binds: `self` and every identifier of each
/// parameter pattern.
fn parameters(sig: &Signature) -> Vec<String> {
    let mut names = Vec::new();
    for input in &sig.inputs {
        match input {
            FnArg::Receiver(_) => names.push("self".to_owned()),
            FnArg::Typed(typed) => bind(&typed.pat, &mut names),
        }
    }
    names
}

fn bind(pat: &Pat, names: &mut Vec<String>) {
    match pat {
        Pat::Ident(ident) => {
            names.push(ident.ident.to_string());
            if let Some((_, sub)) = &ident.subpat {
                bind(sub, names);
            }
        }
        Pat::Tuple(tuple) => tuple.elems.iter().for_each(|elem| bind(elem, names)),
        Pat::TupleStruct(tuple) => tuple.elems.iter().for_each(|elem| bind(elem, names)),
        Pat::Struct(fields) => fields
            .fields
            .iter()
            .for_each(|field| bind(&field.pat, names)),
        Pat::Reference(reference) => bind(&reference.pat, names),
        Pat::Slice(slice) => slice.elems.iter().for_each(|elem| bind(elem, names)),
        Pat::Type(typed) => bind(&typed.pat, names),
        Pat::Paren(paren) => bind(&paren.pat, names),
        _ => {}
    }
}

/// Strips the wrappers that only carry a result through: `.await`, `?`,
/// `return`, parentheses, `Ok(..)`, `Some(..)`, `.into()` and
/// `.map_err(Into::into)`.
fn peel(mut expr: &Expr) -> &Expr {
    loop {
        expr = match expr {
            Expr::Await(inner) => &inner.base,
            Expr::Try(inner) => &inner.expr,
            Expr::Paren(inner) => &inner.expr,
            Expr::Group(inner) => &inner.expr,
            Expr::Return(inner) => match &inner.expr {
                Some(value) => value,
                None => return expr,
            },
            Expr::Call(call) => match (&*call.func, call.args.first()) {
                (Expr::Path(func), Some(arg))
                    if call.args.len() == 1 && (is_path(func, "Ok") || is_path(func, "Some")) =>
                {
                    arg
                }
                _ => return expr,
            },
            Expr::MethodCall(call)
                if call.method == "into" && call.args.is_empty() && call.turbofish.is_none() =>
            {
                &call.receiver
            }
            Expr::MethodCall(call) if call.method == "map_err" && call.args.len() == 1 => {
                match call.args.first() {
                    Some(Expr::Path(arg)) if converts(arg) => &call.receiver,
                    _ => return expr,
                }
            }
            _ => return expr,
        };
    }
}

fn is_path(path: &ExprPath, name: &str) -> bool {
    path.qself.is_none() && path.path.is_ident(name)
}

/// `self`, or a reference, dereference or parenthesis of it.
fn is_self(expr: &Expr) -> bool {
    match expr {
        Expr::Path(path) => path.qself.is_none() && path.path.is_ident("self"),
        Expr::Reference(inner) => is_self(&inner.expr),
        Expr::Unary(inner) if matches!(inner.op, UnOp::Deref(_)) => is_self(&inner.expr),
        Expr::Paren(inner) => is_self(&inner.expr),
        Expr::Group(inner) => is_self(&inner.expr),
        _ => false,
    }
}

/// An argument a caller could pass itself: a parameter (`self` included), a
/// literal or a constant, or a reference, dereference, borrow or clone of one.
fn operand(expr: &Expr, params: &[String]) -> bool {
    match expr {
        Expr::Lit(_) => true,
        Expr::Path(path) => {
            path.qself.is_none() && (is_constant(path) || is_parameter(path, params))
        }
        Expr::Reference(inner) => operand(&inner.expr, params),
        Expr::Unary(inner) if matches!(inner.op, UnOp::Deref(_)) => operand(&inner.expr, params),
        Expr::Paren(inner) => operand(&inner.expr, params),
        Expr::Group(inner) => operand(&inner.expr, params),
        Expr::MethodCall(call)
            if call.args.is_empty() && BORROWS.iter().any(|name| call.method == name) =>
        {
            operand(&call.receiver, params)
        }
        _ => false,
    }
}

/// An argument a constructor fixes: a literal, `None` or a constant.
fn fixed(expr: &Expr) -> bool {
    match expr {
        Expr::Lit(_) => true,
        Expr::Path(path) => is_constant(path),
        Expr::Reference(inner) => fixed(&inner.expr),
        Expr::Paren(inner) => fixed(&inner.expr),
        Expr::Group(inner) => fixed(&inner.expr),
        _ => false,
    }
}

fn is_parameter(path: &ExprPath, params: &[String]) -> bool {
    path.path
        .get_ident()
        .is_some_and(|ident| params.iter().any(|param| ident == param))
}

/// A path that names a value rather than a binding: a constant (`LIMIT`,
/// `Self::MAX`), a unit variant (`None`, `Kind::EhrStatus`) or a unit struct,
/// all of which start with a capital.
fn is_constant(path: &ExprPath) -> bool {
    path.path
        .segments
        .last()
        .is_some_and(|last| capitalised(&last.ident.to_string()))
}

fn capitalised(name: &str) -> bool {
    name.starts_with(|first: char| first.is_ascii_uppercase())
}

/// An error conversion `map_err` may carry without deciding anything:
/// `Into::into`, `X::from`, or a variant or struct that wraps the error.
fn converts(path: &ExprPath) -> bool {
    path.path.segments.last().is_some_and(|last| {
        let name = last.ident.to_string();
        name == "into" || name == "from" || capitalised(&name)
    })
}

/// The tokens of a node as one line of compact text: a space only between two
/// words and after a comma, so the same code always reads the same.
pub(crate) fn compact(node: &impl ToTokens) -> String {
    let mut out = String::new();
    write_tokens(node.to_token_stream(), &mut out);
    out
}

fn write_tokens(stream: TokenStream, out: &mut String) {
    for token in stream {
        match token {
            TokenTree::Group(group) => {
                let (open, close) = match group.delimiter() {
                    Delimiter::Parenthesis => ("(", ")"),
                    Delimiter::Brace => ("{", "}"),
                    Delimiter::Bracket => ("[", "]"),
                    Delimiter::None => ("", ""),
                };
                out.push_str(open);
                write_tokens(group.stream(), out);
                out.push_str(close);
            }
            TokenTree::Punct(punct) => {
                out.push(punct.as_char());
                if punct.as_char() == ',' {
                    out.push(' ');
                }
            }
            TokenTree::Ident(_) | TokenTree::Literal(_) => {
                if out.ends_with(|last: char| last.is_alphanumeric() || last == '_' || last == '"')
                {
                    out.push(' ');
                }
                let text = token.to_string();
                out.extend(
                    text.chars()
                        .map(|c| if c.is_whitespace() { ' ' } else { c }),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Forwarder, scan};

    fn found(source: &str) -> Vec<String> {
        let file = syn::parse_file(source).expect("the fixture should parse");
        scan(&file)
            .forwarders
            .into_iter()
            .map(|Forwarder { owner, target, .. }| format!("{owner} -> {target}"))
            .collect()
    }

    #[test]
    fn free_function_and_inherent_method_forwarders() {
        let source = r"
            pub fn load(id: u32) -> Row { store::load(id) }
            impl Service {
                pub async fn get(&self, id: u32) -> Result<Row, E> { self.get_row(&id).await }
                fn delegated(&self, ehr: &Ehr) -> Result<(), E> { Ok(check(self, ehr.clone())?) }
                fn converted(&self, id: u32) -> Result<Row, E> { self.read(id).map_err(Into::into) }
                fn fixed(&self) -> Row { self.read(LIMIT, 3, None) }
                fn variant(&self, id: u32) -> Row { self.meta(id, Kind::EhrStatus) }
                async fn wrapped(&self, id: u32) -> Result<Row, E> { store::row(id).await.map_err(E::Storage) }
                fn borrowed(&self, tag: Option<String>, name: &String) -> Row { self.tags(tag.as_deref(), name.as_str()) }
            }
        ";
        assert_eq!(
            found(source),
            [
                "load -> store::load(id)",
                "Service::get -> self.get_row(&id)",
                "Service::delegated -> check(self, ehr.clone())",
                "Service::converted -> self.read(id)",
                "Service::fixed -> self.read(LIMIT, 3, None)",
                "Service::variant -> self.meta(id, Kind::EhrStatus)",
                "Service::wrapped -> store::row(id)",
                "Service::borrowed -> self.tags(tag.as_deref(), name.as_str())",
            ]
        );
    }

    #[test]
    fn the_exceptions_are_left_out() {
        let source = r#"
            impl Display for Id { fn fmt(&self, f: &mut Formatter) -> fmt::Result { self.0.fmt(f) } }
            impl From<u32> for Id { fn from(v: u32) -> Self { Self::make(v) } }
            impl Pool {
                pub fn new() -> Self { Self::with_capacity(16) }
                pub fn with_default(size: usize) -> Self { Self::with_limit(size, MAX) }
                pub fn new_sized(size: usize) -> Self { Self::sized(size) }
                pub fn bounded(size: usize) -> Pool { Self::with_limit(size, MAX) }
                pub fn reset(&self, size: usize) -> Self { self.with_limit(size, MAX) }
            }
            #[cfg(feature = "store")]
            fn ends(records: &[Row]) -> Vec<End> { records.iter().map(end).collect() }
            #[cfg(not(feature = "store"))]
            fn ends(records: &[Row]) -> Vec<End> { empty(records) }
            fn main() { tool::run() }
            impl CTime {
                /// `second_validity` (`org.openehr.am.aom2.c_time.adoc` §Functions).
                pub fn second_validity(&self) -> Option<Validity> { self.slot(2) }
            }
            #[cfg(test)]
            fn helper(x: u32) -> u32 { other(x) }
            #[cfg(test)]
            mod tests { fn inner(x: u32) -> u32 { other(x) } }
        "#;
        assert_eq!(
            found(source),
            [
                "Pool::new_sized -> Self::sized(size)",
                "Pool::reset -> self.with_limit(size, MAX)",
            ]
        );
    }

    #[test]
    fn bodies_that_decide_are_left_out() {
        let source = r#"
            fn two(x: u32) -> u32 { let y = x; other(y) }
            fn computed(x: u32) -> u32 { other(x + 1) }
            fn chained(&self) -> u32 { self.pool().get_row(1) }
            fn mapped(&self, x: u32) -> Option<u32> { self.read(x).map(Row::id) }
            fn closure(&self, x: u32) -> u32 { self.read(x, |r| r.id) }
            fn param_field(row: Row) -> u32 { other(row.id) }
            fn variant(x: u32) -> Kind { Kind::Some(x) }
            fn tuple(x: u32) -> Id { Id(x) }
            fn macro_call(x: u32) -> String { format!("{x}") }
            fn calls_param(f: impl Fn() -> u32) -> u32 { f() }
            fn wrapped(x: u32) -> Option<u32> { Some(x) }
            fn literal() -> String { "x".to_owned() }
            impl Bag {
                fn len(&self) -> usize { self.items.len() }
                fn name(&self) -> &str { self.name.as_str() }
                fn id(&self) -> Id { self.id.clone() }
                fn status(&self) -> u16 { self.error.status() }
                fn ovid(&self) -> String { object_version_id(self.vo_id, &self.system) }
                fn opted_out(&self, id: u32) -> bool { store::opted_out(&self.pool, id) }
            }
            impl Code {
                fn is_at(code: &str) -> bool { code.starts_with(Self::AT) }
                fn lower(name: &str) -> String { name.to_lowercase() }
            }
        "#;
        assert!(found(source).is_empty(), "{:?}", found(source));
    }

    #[test]
    fn inline_modules_name_the_owner_and_test_modules_are_listed() {
        let source = r"
            mod inner { impl Store { fn put(&self, x: u32) { self.put_raw(x); } } }
            #[cfg(test)]
            mod tests;
            #[cfg(all(test, feature = 'x'))]
            mod slow { }
            #[cfg(any(test, feature = 'x'))]
            mod shared { fn kept(x: u32) -> u32 { other(x) } }
        ";
        let file = syn::parse_file(source).expect("the fixture should parse");
        let report = scan(&file);
        assert_eq!(
            report.test_modules,
            [vec!["tests".to_owned()], vec!["slow".to_owned()]]
        );
        let owners: Vec<&str> = report.forwarders.iter().map(|f| f.owner.as_str()).collect();
        assert_eq!(owners, ["inner::Store::put", "shared::kept"]);
    }

    #[test]
    fn the_line_is_the_function_name() {
        let file = syn::parse_file("\n\nfn a(x: u32) -> u32 {\n    b(x)\n}\n")
            .expect("the fixture should parse");
        assert_eq!(scan(&file).forwarders.first().map(|f| f.line), Some(3));
    }
}
