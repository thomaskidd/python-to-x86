use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use rustpython_parser::ast;

use crate::hir::{
    BinOp, BoolOp, ClassDef, ClassId, CmpOp, DictId, Expr, Function, ListId, Param, Program, SetId,
    Stmt, TupleId, Type, TypedExpr, UnaryOp,
};
use crate::parser;
use crate::parser::Module;

thread_local! {
    /// Per-`lower()` map from class name → ClassId. Populated by the
    /// class-collection pass (Pass 0), read by parse_type_annotation
    /// and by call/attribute lowering. Cleared at the start of each
    /// `lower()` call.
    static CLASS_REGISTRY: RefCell<HashMap<String, ClassId>> =
        RefCell::new(HashMap::new());
    /// The class currently being lowered (a method body). Set by
    /// `lower_method` and used by `super()` resolution. None when
    /// lowering top-level functions.
    static CURRENT_CLASS: RefCell<Option<ClassId>> = const { RefCell::new(None) };
}

const MAX_PARAMS: usize = 16;

/// Per-function type scope: name → type.
type Scope = HashMap<String, Type>;

#[derive(Debug)]
struct FunctionSig {
    params: Vec<Param>,
    defaults: Vec<Option<TypedExpr>>,
    return_ty: Type,
}

type SignatureTable = HashMap<String, FunctionSig>;

pub fn lower(module: &Module, source_path: &Path) -> Result<Program> {
    CLASS_REGISTRY.with(|r| r.borrow_mut().clear());
    // Pre-register `ABC` as a sentinel class with no fields, no methods,
    // and no abstract methods. Used to recognize `class Foo(ABC):` at
    // pass 0b. v0.34 marks classes as abstract via their declared
    // @abstractmethod decorators, not via ABC inheritance directly
    // (matches Python: `class C(ABC): pass; C()` is allowed).
    let abc_id = ClassId::intern(ClassDef {
        name: "ABC".to_string(),
        fields: Vec::new(),
        parent: None,
        abstract_methods: Vec::new(),
    });
    CLASS_REGISTRY.with(|r| r.borrow_mut().insert("ABC".to_string(), abc_id));
    let mut loaded: HashSet<PathBuf> = HashSet::new();
    let mut all_functions: Vec<Function> = Vec::new();
    let mut signatures: SignatureTable = HashMap::new();

    let abs = source_path
        .canonicalize()
        .unwrap_or_else(|_| source_path.to_path_buf());
    loaded.insert(abs.clone());

    lower_module_into(
        module,
        source_path,
        &mut signatures,
        &mut all_functions,
        &mut loaded,
        true, // root module: collects top-level functions for self-defined main
    )?;

    if !signatures.contains_key("main") {
        bail!("unsupported_feature: no `main` function defined at the top level");
    }
    let main_sig = signatures.get("main").unwrap();
    if !matches!(main_sig.return_ty, Type::I64 | Type::F64 | Type::Str) {
        bail!(
            "unsupported_feature: `main` must return `int`, `float`, or `str`, found {}",
            main_sig.return_ty.name()
        );
    }
    for p in &main_sig.params {
        if !matches!(p.ty, Type::I64 | Type::F64) {
            bail!(
                "unsupported_feature: `main` parameter `{}` must be `int` or `float`, found {}",
                p.name,
                p.ty.name()
            );
        }
    }

    Ok(Program { functions: all_functions })
}

/// Recursively lower a module + its sibling-file imports into the
/// shared signature table and function list. `is_root` only affects
/// error messages (a non-root module that lacks `main` is fine).
fn lower_module_into(
    module: &Module,
    source_path: &Path,
    signatures: &mut SignatureTable,
    all_functions: &mut Vec<Function>,
    loaded: &mut HashSet<PathBuf>,
    _is_root: bool,
) -> Result<()> {
    // Phase 0a: pre-register class names so type annotations can
    // reference them (including in class methods of the same class).
    for stmt in &module.body {
        if let ast::Stmt::ClassDef(c) = stmt {
            let name = c.name.as_str().to_string();
            if CLASS_REGISTRY.with(|r| r.borrow().contains_key(&name))
                || signatures.contains_key(&name)
            {
                bail!(
                    "unsupported_feature: duplicate class or function `{}`",
                    name
                );
            }
            // Pre-register with empty fields and no parent; filled in below.
            let id = ClassId::intern(ClassDef {
                name: name.clone(),
                fields: Vec::new(),
                parent: None,
                abstract_methods: Vec::new(),
            });
            CLASS_REGISTRY.with(|r| r.borrow_mut().insert(name, id));
        }
    }

    // Phase 0b: process each class — collect fields, register methods
    // as mangled top-level functions (`<ClassName>.<method>`).
    let mut local_method_defs: Vec<(ClassId, String, &ast::StmtFunctionDef)> = Vec::new();
    for stmt in &module.body {
        if let ast::Stmt::ClassDef(c) = stmt {
            let name = c.name.as_str().to_string();
            let class_id = CLASS_REGISTRY.with(|r| r.borrow()[&name]);
            // v0.34 keeps single-base inheritance (from v0.33). The base
            // may be either a concrete class or an abstract one — the
            // only difference is that the subclass inherits any
            // unimplemented abstract methods. The sentinel `ABC` itself
            // is a valid base; it has no fields and no methods, so
            // `class Foo(ABC):` is equivalent to no base for layout but
            // signals "this class participates in the ABC protocol".
            // Multi-base interface-style inheritance is deferred to a
            // later slice (needs vtables to be useful anyway).
            let parent_id: Option<ClassId> = if c.bases.is_empty() {
                None
            } else if c.bases.len() == 1 {
                let base_name = match &c.bases[0] {
                    ast::Expr::Name(n) => n.id.as_str(),
                    _ => bail!(
                        "unsupported_feature: base class must be a simple class name (in `{}`)",
                        name
                    ),
                };
                let base_id = CLASS_REGISTRY
                    .with(|r| r.borrow().get(base_name).copied())
                    .ok_or_else(|| {
                        anyhow!(
                            "unsupported_feature: base class `{}` of `{}` is not defined",
                            base_name,
                            name
                        )
                    })?;
                Some(base_id)
            } else {
                bail!(
                    "unsupported_feature: multiple inheritance is not supported in v0.34 (only single-base; interface-style ABCs are deferred)"
                );
            };
            class_id.set_parent(parent_id);
            if !c.decorator_list.is_empty() {
                bail!(
                    "unsupported_feature: class decorators not supported (in `{}`)",
                    name
                );
            }
            if !c.keywords.is_empty() {
                bail!(
                    "unsupported_feature: class keyword args (e.g. metaclass=) not supported (in `{}`)",
                    name
                );
            }
            // Start the field list with the parent's fields (prepended) so
            // the layout prefix matches the parent.
            let mut fields: Vec<(String, Type)> = match parent_id {
                Some(p) => p.fields(),
                None => Vec::new(),
            };
            let mut field_names: HashSet<String> =
                fields.iter().map(|(n, _)| n.clone()).collect();
            // Seed unimplemented-abstract set with what we inherit.
            let mut unimplemented: HashSet<String> = match parent_id {
                Some(p) => p.abstract_methods().into_iter().collect(),
                None => HashSet::new(),
            };
            for inner in &c.body {
                match inner {
                    ast::Stmt::AnnAssign(a) => {
                        let fname = match a.target.as_ref() {
                            ast::Expr::Name(n) => n.id.as_str().to_string(),
                            _ => bail!(
                                "unsupported_feature: class field target must be a simple name (in `{}`)",
                                name
                            ),
                        };
                        if !field_names.insert(fname.clone()) {
                            bail!(
                                "unsupported_feature: duplicate field `{}` in class `{}`",
                                fname,
                                name
                            );
                        }
                        let ty = parse_type_annotation(Some(&a.annotation)).ok_or_else(|| {
                            anyhow!(
                                "unsupported_feature: class field `{}` annotation could not be resolved (in `{}`)",
                                fname,
                                name
                            )
                        })?;
                        if a.value.is_some() {
                            bail!(
                                "unsupported_feature: class field defaults not supported (on `{}` in `{}`)",
                                fname,
                                name
                            );
                        }
                        fields.push((fname, ty));
                    }
                    ast::Stmt::FunctionDef(f) => {
                        let is_abstract_decorated = f.decorator_list.iter().any(|d| {
                            matches!(d, ast::Expr::Name(n) if n.id.as_str() == "abstractmethod")
                        });
                        let mname = format!("{}.{}", name, f.name);
                        if is_abstract_decorated {
                            // Abstract methods are recorded in the
                            // abstract set and don't get lowered or
                            // registered in `signatures`. (We also don't
                            // care about their body — Python idiomatically
                            // uses `pass` or `...`.)
                            unimplemented.insert(f.name.as_str().to_string());
                        } else {
                            if signatures.contains_key(&mname) {
                                bail!(
                                    "unsupported_feature: duplicate method `{}` in class `{}`",
                                    f.name,
                                    name
                                );
                            }
                            let sig = collect_method_signature(class_id, f)?;
                            signatures.insert(mname.clone(), sig);
                            local_method_defs.push((class_id, mname, f));
                            // A concrete method override removes the
                            // method name from the unimplemented set.
                            unimplemented.remove(f.name.as_str());
                        }
                    }
                    ast::Stmt::Pass(_) => {}
                    other => bail!(
                        "unsupported_feature: only field annotations, methods, and `pass` allowed in class body (got {} in `{}`)",
                        stmt_kind_name(other),
                        name
                    ),
                }
            }
            // Now finalize fields + abstract-method set on the class.
            class_id.set_fields(fields);
            let mut unimplemented_vec: Vec<String> = unimplemented.into_iter().collect();
            unimplemented_vec.sort();
            class_id.set_abstract_methods(unimplemented_vec);
        }
    }

    let mut local_func_defs: Vec<&ast::StmtFunctionDef> = Vec::new();
    for stmt in &module.body {
        match stmt {
            ast::Stmt::ClassDef(_) => {
                // Already handled in Phase 0.
            }
            ast::Stmt::FunctionDef(f) => {
                let sig = collect_signature(f)?;
                let name = f.name.as_str().to_string();
                if signatures.contains_key(&name) {
                    bail!(
                        "unsupported_feature: duplicate top-level function `{}` (collisions across files are not supported)",
                        name
                    );
                }
                signatures.insert(name, sig);
                local_func_defs.push(f);
            }
            ast::Stmt::ImportFrom(im) => {
                let module_name = im.module.as_ref().map(|s| s.as_str()).unwrap_or("");
                match module_name {
                    "pyx86.types" | "__future__" | "math" | "abc" => {
                        // Documentary imports; no file load. Names from
                        // math are recognized by lower_builtin_call directly.
                    }
                    _ => {
                        // Resolve sibling file: <source_dir>/<module-as-path>.py.
                        let dir = source_path.parent().unwrap_or_else(|| Path::new("."));
                        let mut rel = PathBuf::new();
                        for part in module_name.split('.') {
                            rel.push(part);
                        }
                        let mut candidate = dir.join(&rel);
                        candidate.set_extension("py");
                        if !candidate.exists() {
                            bail!(
                                "unsupported_feature: cannot find module `{}` (looked for {})",
                                module_name,
                                candidate.display()
                            );
                        }
                        let abs = candidate
                            .canonicalize()
                            .unwrap_or_else(|_| candidate.clone());
                        if loaded.contains(&abs) {
                            // Already loaded (or a cycle) — skip recursive load.
                            // The names being imported are presumed to be in
                            // signatures already.
                            continue;
                        }
                        loaded.insert(abs.clone());
                        let source = std::fs::read_to_string(&candidate).with_context(|| {
                            format!("read imported module {}", candidate.display())
                        })?;
                        let imported_module = parser::parse(&source, &candidate)?;
                        lower_module_into(
                            &imported_module,
                            &candidate,
                            signatures,
                            all_functions,
                            loaded,
                            false,
                        )?;
                        // Verify the requested names actually exist.
                        for alias in &im.names {
                            let name = alias.name.as_str();
                            if !signatures.contains_key(name) {
                                bail!(
                                    "unsupported_feature: module `{}` does not define `{}`",
                                    module_name,
                                    name
                                );
                            }
                            if alias.asname.is_some() {
                                bail!(
                                    "unsupported_feature: import aliases (`as`) are not yet supported"
                                );
                            }
                        }
                    }
                }
            }
            other => bail!(
                "unsupported_feature: only `def` and `from … import …` are allowed at the top level, found {}",
                stmt_kind_name(other)
            ),
        }
    }
    // Pass 2: lower bodies of THIS module's local functions and class methods.
    for func in local_func_defs {
        let lowered = lower_function(func, signatures)?;
        all_functions.push(lowered);
    }
    for (class_id, mname, fdef) in local_method_defs {
        let lowered = lower_method(class_id, &mname, fdef, signatures)?;
        all_functions.push(lowered);
    }
    Ok(())
}

/// Build a FunctionSig for a class method. The first param is `self`
/// of type Class(class_id), implicitly added (no annotation required
/// — Python's convention).
fn collect_method_signature(
    class_id: ClassId,
    func: &ast::StmtFunctionDef,
) -> Result<FunctionSig> {
    if !func.args.posonlyargs.is_empty() || !func.args.kwonlyargs.is_empty() {
        bail!(
            "unsupported_feature: method `{}` may not use positional-only / keyword-only params",
            func.name
        );
    }
    if func.args.vararg.is_some() || func.args.kwarg.is_some() {
        bail!(
            "unsupported_feature: method `{}` may not use *args / **kwargs",
            func.name
        );
    }
    if !func.decorator_list.is_empty() {
        bail!(
            "unsupported_feature: method decorators not yet supported (on `{}`)",
            func.name
        );
    }
    if func.args.args.is_empty() {
        bail!(
            "unsupported_feature: method `{}` must take `self` as its first parameter",
            func.name
        );
    }
    let first_arg = &func.args.args[0];
    if first_arg.def.arg.as_str() != "self" {
        bail!(
            "unsupported_feature: method `{}` first parameter must be named `self`",
            func.name
        );
    }
    if first_arg.def.annotation.is_some() {
        bail!(
            "unsupported_feature: don't annotate `self` (it's implicit) (on `{}`)",
            func.name
        );
    }
    let mut params: Vec<Param> = Vec::with_capacity(func.args.args.len());
    params.push(Param {
        name: "self".to_string(),
        ty: Type::Class(class_id),
    });
    let mut seen: HashSet<String> = HashSet::new();
    seen.insert("self".to_string());
    for arg in func.args.args.iter().skip(1) {
        let name = arg.def.arg.as_str().to_string();
        if !seen.insert(name.clone()) {
            bail!(
                "unsupported_feature: duplicate parameter `{}` in method `{}`",
                name,
                func.name
            );
        }
        let ty = parse_type_annotation(arg.def.annotation.as_deref()).ok_or_else(|| {
            anyhow!(
                "unsupported_feature: parameter `{}` of method `{}` must be annotated",
                name,
                func.name
            )
        })?;
        reject_abstract_type(ty, "method parameter type")?;
        params.push(Param { name, ty });
    }
    let return_ty = match parse_type_annotation(func.returns.as_deref()) {
        Some(ty) => ty,
        // __init__ implicitly returns None — for our model we use
        // Class(class_id) so the constructor pattern works (the body
        // ends with `return self` semantics).
        None if func.name.as_str() == "__init__" => Type::Class(class_id),
        None => bail!(
            "unsupported_feature: method `{}` requires a return annotation",
            func.name
        ),
    };
    reject_abstract_type(return_ty, "method return type")?;
    if func.args.defaults().next().is_some() {
        bail!(
            "unsupported_feature: default args on methods not yet supported (on `{}`)",
            func.name
        );
    }
    let n = params.len();
    let defaults: Vec<Option<TypedExpr>> = vec![None; n];
    Ok(FunctionSig { params, defaults, return_ty })
}

/// Lower a method body. Same as lower_function but the function's
/// HIR name is the mangled `<ClassName>.<method>` and the implicit
/// `__init__` return is `self` (we synthesize it).
fn lower_method(
    class_id: ClassId,
    mangled_name: &str,
    func: &ast::StmtFunctionDef,
    signatures: &SignatureTable,
) -> Result<Function> {
    let sig = signatures.get(mangled_name).expect("method sig collected");
    let mut scope: Scope = sig.params.iter().map(|p| (p.name.clone(), p.ty)).collect();
    if func.body.is_empty() {
        bail!("unsupported_feature: method `{}` body is empty", func.name);
    }
    let prev = CURRENT_CLASS.with(|c| c.replace(Some(class_id)));
    let body_result = lower_block(&func.body, &mut scope, 0, signatures, sig.return_ty);
    CURRENT_CLASS.with(|c| *c.borrow_mut() = prev);
    let mut body = body_result?;
    // For __init__: synthesize `return self` if the body doesn't already
    // end with a return (Python's __init__ implicitly returns None; we
    // return self so `Foo(...)` evaluates to the instance).
    if func.name.as_str() == "__init__"
        && !matches!(body.last(), Some(Stmt::Return { .. }))
    {
        body.push(Stmt::Return {
            value: TypedExpr::new(Type::Class(class_id), Expr::Var("self".to_string())),
        });
    }
    if !block_always_returns(&body) {
        bail!(
            "unsupported_feature: not all paths return a value in method `{}`",
            func.name
        );
    }
    Ok(Function {
        name: mangled_name.to_string(),
        params: sig.params.clone(),
        return_ty: sig.return_ty,
        body,
    })
}

fn collect_signature(func: &ast::StmtFunctionDef) -> Result<FunctionSig> {
    if !func.args.posonlyargs.is_empty() || !func.args.kwonlyargs.is_empty() {
        bail!(
            "unsupported_feature: positional-only and keyword-only parameters are not supported (in `{}`)",
            func.name
        );
    }
    if func.args.vararg.is_some() || func.args.kwarg.is_some() {
        bail!(
            "unsupported_feature: *args / **kwargs are not supported (in `{}`)",
            func.name
        );
    }
    if !func.decorator_list.is_empty() {
        bail!(
            "unsupported_feature: decorators are not supported (in `{}`)",
            func.name
        );
    }
    if func.args.args.len() > MAX_PARAMS {
        bail!(
            "unsupported_feature: at most {} parameters supported, found {} (in `{}`)",
            MAX_PARAMS,
            func.args.args.len(),
            func.name
        );
    }

    let mut params = Vec::with_capacity(func.args.args.len());
    let mut seen = HashSet::new();
    for arg in &func.args.args {
        let name = arg.def.arg.as_str().to_string();
        if !seen.insert(name.clone()) {
            bail!(
                "unsupported_feature: duplicate parameter name `{}` (in `{}`)",
                name,
                func.name
            );
        }
        let ty = parse_type_annotation(arg.def.annotation.as_deref()).ok_or_else(|| {
            anyhow!(
                "unsupported_feature: parameter `{}` must be annotated `: int` (in `{}`)",
                name,
                func.name
            )
        })?;
        reject_abstract_type(ty, "function parameter type")?;
        params.push(Param { name, ty });
    }

    let return_ty = match parse_type_annotation(func.returns.as_deref()) {
        Some(ty) => ty,
        None => bail!(
            "unsupported_feature: function `{}` requires a return annotation `-> int`",
            func.name
        ),
    };
    reject_abstract_type(return_ty, "function return type")?;

    let raw_defaults: Vec<&ast::Expr> = func.args.defaults().collect();
    let n = params.len();
    let n_defaulted = raw_defaults.len();
    let n_required = n - n_defaulted;
    let mut defaults: Vec<Option<TypedExpr>> = vec![None; n];
    for (i, raw) in raw_defaults.iter().enumerate() {
        let param_idx = n_required + i;
        let lowered = lower_default(raw).map_err(|e| {
            anyhow!(
                "unsupported_feature: default for parameter `{}` (in `{}`): {}",
                params[param_idx].name,
                func.name,
                e
            )
        })?;
        // Coerce default to declared param type.
        let coerced = coerce(lowered, params[param_idx].ty)?;
        defaults[param_idx] = Some(coerced);
    }

    Ok(FunctionSig { params, defaults, return_ty })
}

fn lower_default(e: &ast::Expr) -> Result<TypedExpr> {
    match e {
        ast::Expr::Constant(c) => match &c.value {
            ast::Constant::Int(big) => {
                let v: i64 = big
                    .try_into()
                    .map_err(|_| anyhow!("integer literal does not fit in i64"))?;
                Ok(TypedExpr::new(Type::I64, Expr::ConstI64(v)))
            }
            ast::Constant::Bool(b) => Ok(TypedExpr::new(Type::Bool, Expr::ConstBool(*b))),
            _ => bail!("only integer or bool literals are allowed as defaults"),
        },
        ast::Expr::UnaryOp(u) if matches!(u.op, ast::UnaryOp::USub | ast::UnaryOp::UAdd) => {
            let inner = lower_default(&u.operand)?;
            match (u.op, &inner.expr) {
                (ast::UnaryOp::USub, Expr::ConstI64(v)) => {
                    Ok(TypedExpr::new(Type::I64, Expr::ConstI64(-v)))
                }
                (ast::UnaryOp::UAdd, _) => Ok(inner),
                _ => bail!("only integer literals are allowed as defaults"),
            }
        }
        _ => bail!("only integer literals are allowed as defaults"),
    }
}

fn lower_function(func: &ast::StmtFunctionDef, signatures: &SignatureTable) -> Result<Function> {
    let name = func.name.as_str().to_string();
    let sig = signatures.get(&name).expect("signature collected in pass 1");

    let mut scope: Scope =
        sig.params.iter().map(|p| (p.name.clone(), p.ty)).collect();

    if func.body.is_empty() {
        bail!("unsupported_feature: function `{}` body is empty", name);
    }

    let body = lower_block(&func.body, &mut scope, 0, signatures, sig.return_ty)?;

    if !block_always_returns(&body) {
        bail!(
            "unsupported_feature: not all paths return a value in `{}` (the function body, or both branches of every trailing `if`, must end with `return`)",
            name
        );
    }

    Ok(Function {
        name,
        params: sig.params.clone(),
        return_ty: sig.return_ty,
        body,
    })
}

fn block_always_returns(body: &[Stmt]) -> bool {
    match body.last() {
        Some(Stmt::Return { .. }) => true,
        Some(Stmt::If { then_body, else_body, .. }) => {
            !else_body.is_empty()
                && block_always_returns(then_body)
                && block_always_returns(else_body)
        }
        _ => false,
    }
}

fn lower_block(
    stmts: &[ast::Stmt],
    scope: &mut Scope,
    loop_depth: usize,
    signatures: &SignatureTable,
    return_ty: Type,
) -> Result<Vec<Stmt>> {
    let mut out = Vec::with_capacity(stmts.len());
    for stmt in stmts {
        match stmt {
            ast::Stmt::Assign(a) => {
                // Single-target assignment to either:
                //   - a simple Name           → Stmt::Let
                //   - an Attribute(obj.field) → Stmt::SetField
                if a.targets.len() != 1 {
                    bail!("unsupported_feature: chained assignment `a = b = ...` is not supported");
                }
                match &a.targets[0] {
                    ast::Expr::Name(n) => {
                        let name = n.id.as_str().to_string();
                        let value = lower_expr(&a.value, scope, signatures)?;
                        scope.insert(name.clone(), value.ty);
                        out.push(Stmt::Let { name, value });
                    }
                    ast::Expr::Attribute(attr) => {
                        let obj = lower_expr(&attr.value, scope, signatures)?;
                        let class_id = match obj.ty {
                            Type::Class(id) => id,
                            other => bail!(
                                "unsupported_feature: attribute assignment on {} not supported",
                                other.name()
                            ),
                        };
                        let field_name = attr.attr.as_str();
                        let field_index = class_id.field_index(field_name).ok_or_else(|| {
                            anyhow!(
                                "unsupported_feature: class `{}` has no field `{}`",
                                class_id.name(),
                                field_name
                            )
                        })?;
                        let field_ty = class_id.field_ty(field_name).unwrap();
                        let value = lower_expr(&a.value, scope, signatures)?;
                        let value = coerce(value, field_ty)?;
                        out.push(Stmt::SetField { obj, field_index, value });
                    }
                    ast::Expr::Subscript(s) => {
                        let container = lower_expr(&s.value, scope, signatures)?;
                        let (key_ty, value_ty) = match container.ty {
                            Type::Dict(id) => (id.key(), id.val()),
                            Type::List(id) => (Type::I64, id.elem()),
                            other => bail!(
                                "unsupported_feature: subscript-assignment on {} is not supported",
                                other.name()
                            ),
                        };
                        let key = lower_expr(&s.slice, scope, signatures)?;
                        let key = coerce(key, key_ty)?;
                        let value = lower_expr(&a.value, scope, signatures)?;
                        let value = coerce(value, value_ty)?;
                        out.push(Stmt::SetSubscript { container, key, value });
                    }
                    other => bail!(
                        "unsupported_feature: assignment target `{}` not supported",
                        expr_kind_name(other)
                    ),
                }
            }
            ast::Stmt::AnnAssign(a) => {
                let name = match a.target.as_ref() {
                    ast::Expr::Name(n) => n.id.as_str().to_string(),
                    _ => bail!(
                        "unsupported_feature: only simple-name targets are supported in assignments"
                    ),
                };
                if !a.simple {
                    bail!(
                        "unsupported_feature: parenthesised annotation targets are not supported"
                    );
                }
                let declared_ty = parse_type_annotation(Some(&a.annotation)).ok_or_else(|| {
                    anyhow!(
                        "unsupported_feature: only `: int` / `: float` / `: bool` annotations are supported on locals, on `{}`",
                        name
                    )
                })?;
                reject_abstract_type(declared_ty, "local variable type")?;
                let value_expr = a.value.as_deref().ok_or_else(|| {
                    anyhow!(
                        "unsupported_feature: bare annotation `{}: <type>` (no value) is not supported",
                        name
                    )
                })?;
                let value = lower_expr(value_expr, scope, signatures)?;
                let value = coerce(value, declared_ty)?;
                scope.insert(name.clone(), declared_ty);
                out.push(Stmt::Let { name, value });
            }
            ast::Stmt::AugAssign(a) => {
                let name = match a.target.as_ref() {
                    ast::Expr::Name(n) => n.id.as_str().to_string(),
                    _ => bail!(
                        "unsupported_feature: augmented-assignment target must be a simple name"
                    ),
                };
                let lhs_ty = *scope.get(&name).ok_or_else(|| {
                    anyhow!(
                        "unsupported_feature: augmented assignment to unbound name `{}` (must already be a parameter or assigned local)",
                        name
                    )
                })?;
                let op = match a.op {
                    ast::Operator::Add => BinOp::Add,
                    ast::Operator::Sub => BinOp::Sub,
                    ast::Operator::Mult => BinOp::Mul,
                    ast::Operator::FloorDiv => BinOp::FloorDiv,
                    ast::Operator::Mod => BinOp::Mod,
                    ast::Operator::Div => BinOp::TrueDiv,
                    ast::Operator::Pow => BinOp::Pow,
                    ast::Operator::LShift => BinOp::Shl,
                    ast::Operator::RShift => BinOp::Shr,
                    ast::Operator::BitAnd => BinOp::BitAnd,
                    ast::Operator::BitOr => BinOp::BitOr,
                    ast::Operator::BitXor => BinOp::BitXor,
                    ast::Operator::MatMult => bail!(
                        "unsupported_feature: `@=` (matmul) is not supported"
                    ),
                };
                let lhs = TypedExpr::new(lhs_ty, Expr::Var(name.clone()));
                let rhs = lower_expr(&a.value, scope, signatures)?;
                let combined = apply_binop(op, lhs, rhs)?;
                let combined = coerce(combined, lhs_ty)?;
                out.push(Stmt::Let { name, value: combined });
            }
            ast::Stmt::Return(r) => {
                let value_expr = r
                    .value
                    .as_deref()
                    .ok_or_else(|| anyhow!("unsupported_feature: `return` must have a value"))?;
                let value = lower_expr(value_expr, scope, signatures)?;
                let value = coerce(value, return_ty)?;
                out.push(Stmt::Return { value });
            }
            ast::Stmt::If(if_stmt) => {
                let cond = lower_expr(&if_stmt.test, scope, signatures)?;
                let cond = coerce(cond, Type::Bool)?;
                let then_body = lower_block(&if_stmt.body, scope, loop_depth, signatures, return_ty)?;
                let else_body =
                    lower_block(&if_stmt.orelse, scope, loop_depth, signatures, return_ty)?;
                out.push(Stmt::If { cond, then_body, else_body });
            }
            ast::Stmt::For(f) => {
                if !f.orelse.is_empty() {
                    bail!("unsupported_feature: `else` clause on `for` is not supported");
                }
                let loop_var = match f.target.as_ref() {
                    ast::Expr::Name(n) => n.id.as_str().to_string(),
                    _ => bail!(
                        "unsupported_feature: for-loop target must be a simple name (no tuple unpacking yet)"
                    ),
                };

                // Two for-loop forms supported:
                //   for i in range(...): ...
                //   for x in <list-expr>: ...
                // Distinguish by attempting to lower the iter expression
                // first if it isn't an obvious `range(...)` call.
                let is_range_call = matches!(
                    f.iter.as_ref(),
                    ast::Expr::Call(c) if matches!(c.func.as_ref(), ast::Expr::Name(n) if n.id.as_str() == "range")
                );
                if is_range_call {
                    let (start, stop, step) =
                        parse_and_lower_range(&f.iter, scope, signatures)?;
                    let step_value = match &step.expr {
                        Expr::ConstI64(v) => *v,
                        _ => bail!(
                            "unsupported_feature: range() step must be a constant integer literal"
                        ),
                    };
                    if step_value == 0 {
                        bail!("unsupported_feature: range() step must be non-zero");
                    }
                    if step_value < 0 {
                        bail!(
                            "unsupported_feature: negative range() step is not yet supported (use a `while` loop)"
                        );
                    }
                    scope.insert(loop_var.clone(), Type::I64);
                    let body_inner =
                        lower_block(&f.body, scope, loop_depth + 1, signatures, return_ty)?;
                    out.push(Stmt::Let { name: loop_var.clone(), value: start });
                    let cond = TypedExpr::new(
                        Type::Bool,
                        Expr::Cmp {
                            op: CmpOp::Lt,
                            lhs: Box::new(TypedExpr::new(
                                Type::I64,
                                Expr::Var(loop_var.clone()),
                            )),
                            rhs: Box::new(stop),
                        },
                    );
                    let mut while_body = body_inner;
                    let incr = TypedExpr::new(
                        Type::I64,
                        Expr::BinOp {
                            op: BinOp::Add,
                            lhs: Box::new(TypedExpr::new(
                                Type::I64,
                                Expr::Var(loop_var.clone()),
                            )),
                            rhs: Box::new(step),
                        },
                    );
                    while_body.push(Stmt::Let { name: loop_var.clone(), value: incr });
                    out.push(Stmt::While { cond, body: while_body });
                } else {
                    // for-over-list: lower the iter, expect List type, desugar to
                    //   __lst = <iter>
                    //   __i = 0
                    //   while __i < len(__lst):
                    //       loop_var = __lst[__i]
                    //       <body>
                    //       __i = __i + 1
                    let iter = lower_expr(&f.iter, scope, signatures)?;
                    let elem_ty = match iter.ty {
                        Type::List(id) => id.elem(),
                        other => bail!(
                            "unsupported_feature: for-loop iterables must be range(...) or list[T] (got {})",
                            other.name()
                        ),
                    };
                    // Synthesise unique helper names so they don't collide
                    // with user vars across nested loops.
                    let lst_name = format!("__forlst_{}", loop_depth);
                    let idx_name = format!("__foridx_{}", loop_depth);
                    scope.insert(lst_name.clone(), iter.ty);
                    scope.insert(idx_name.clone(), Type::I64);
                    scope.insert(loop_var.clone(), elem_ty);

                    let body_inner =
                        lower_block(&f.body, scope, loop_depth + 1, signatures, return_ty)?;

                    out.push(Stmt::Let { name: lst_name.clone(), value: iter });
                    out.push(Stmt::Let {
                        name: idx_name.clone(),
                        value: TypedExpr::new(Type::I64, Expr::ConstI64(0)),
                    });
                    let lst_ref = TypedExpr::new(
                        Type::List(ListId::intern(elem_ty)),
                        Expr::Var(lst_name.clone()),
                    );
                    let cond = TypedExpr::new(
                        Type::Bool,
                        Expr::Cmp {
                            op: CmpOp::Lt,
                            lhs: Box::new(TypedExpr::new(Type::I64, Expr::Var(idx_name.clone()))),
                            rhs: Box::new(TypedExpr::new(
                                Type::I64,
                                Expr::ListLen { list: Box::new(lst_ref.clone()) },
                            )),
                        },
                    );
                    let mut while_body = Vec::new();
                    while_body.push(Stmt::Let {
                        name: loop_var.clone(),
                        value: TypedExpr::new(
                            elem_ty,
                            Expr::ListIndex {
                                list: Box::new(lst_ref),
                                index: Box::new(TypedExpr::new(
                                    Type::I64,
                                    Expr::Var(idx_name.clone()),
                                )),
                            },
                        ),
                    });
                    while_body.extend(body_inner);
                    while_body.push(Stmt::Let {
                        name: idx_name.clone(),
                        value: TypedExpr::new(
                            Type::I64,
                            Expr::BinOp {
                                op: BinOp::Add,
                                lhs: Box::new(TypedExpr::new(Type::I64, Expr::Var(idx_name))),
                                rhs: Box::new(TypedExpr::new(Type::I64, Expr::ConstI64(1))),
                            },
                        ),
                    });
                    out.push(Stmt::While { cond, body: while_body });
                }
            }
            ast::Stmt::While(w) => {
                if !w.orelse.is_empty() {
                    bail!(
                        "unsupported_feature: `else` clause on `while` is not supported"
                    );
                }
                let cond = lower_expr(&w.test, scope, signatures)?;
                let cond = coerce(cond, Type::Bool)?;
                let body = lower_block(&w.body, scope, loop_depth + 1, signatures, return_ty)?;
                out.push(Stmt::While { cond, body });
            }
            ast::Stmt::Break(_) => {
                if loop_depth == 0 {
                    bail!("unsupported_feature: `break` outside of a loop");
                }
                out.push(Stmt::Break);
            }
            ast::Stmt::Continue(_) => {
                if loop_depth == 0 {
                    bail!("unsupported_feature: `continue` outside of a loop");
                }
                out.push(Stmt::Continue);
            }
            ast::Stmt::Pass(_) => {}
            ast::Stmt::Expr(e) => {
                // Currently the only allowed expression-statement is
                // `<list>.append(<value>)`. Anything else is rejected
                // (no general expression-statements yet).
                if let ast::Expr::Call(c) = e.value.as_ref() {
                    if let ast::Expr::Attribute(attr) = c.func.as_ref() {
                        if attr.attr.as_str() == "append" {
                            // <list>.append(<value>) — list must be a Var.
                            let list_name = match attr.value.as_ref() {
                                ast::Expr::Name(n) => n.id.as_str().to_string(),
                                _ => bail!(
                                    "unsupported_feature: list.append() target must be a simple name (no nested expressions yet)"
                                ),
                            };
                            let list_ty = *scope.get(&list_name).ok_or_else(|| {
                                anyhow!(
                                    "unsupported_feature: name `{}` is not in scope",
                                    list_name
                                )
                            })?;
                            let elem_ty = match list_ty {
                                Type::List(id) => id.elem(),
                                other => bail!(
                                    "unsupported_feature: .append() on {} (only list[T] supported)",
                                    other.name()
                                ),
                            };
                            if c.args.len() != 1 || !c.keywords.is_empty() {
                                bail!(
                                    "unsupported_feature: list.append() takes exactly 1 positional argument"
                                );
                            }
                            let value = lower_expr(&c.args[0], scope, signatures)?;
                            let value = coerce(value, elem_ty)?;
                            let list_expr =
                                TypedExpr::new(list_ty, Expr::Var(list_name));
                            out.push(Stmt::ListAppend {
                                list: list_expr,
                                value,
                            });
                            continue;
                        }
                        if attr.attr.as_str() == "add" {
                            // <set>.add(<value>) — intercept only when the
                            // receiver is actually a set. Otherwise (e.g.
                            // a class method named `add`) fall through to
                            // the regular method-call statement handling.
                            if let ast::Expr::Name(n) = attr.value.as_ref() {
                                let set_name = n.id.as_str().to_string();
                                if let Some(&set_ty) = scope.get(&set_name) {
                                    if let Type::Set(id) = set_ty {
                                        if c.args.len() != 1 || !c.keywords.is_empty() {
                                            bail!(
                                                "unsupported_feature: set.add() takes exactly 1 positional argument"
                                            );
                                        }
                                        let value =
                                            lower_expr(&c.args[0], scope, signatures)?;
                                        let value = coerce(value, id.elem())?;
                                        let set_expr =
                                            TypedExpr::new(set_ty, Expr::Var(set_name));
                                        out.push(Stmt::SetAdd { set: set_expr, value });
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                }
                // Allow any Call expression as a stmt (its side effects
                // matter; the return value is discarded).
                if matches!(e.value.as_ref(), ast::Expr::Call(_)) {
                    let lowered = lower_expr(&e.value, scope, signatures)?;
                    out.push(Stmt::ExprStmt(lowered));
                    continue;
                }
                bail!(
                    "unsupported_feature: expression statements are only supported for call expressions (and `list.append(...)`)"
                );
            }
            other => bail!(
                "unsupported_feature: statement `{}` is not supported",
                stmt_kind_name(other)
            ),
        }
    }
    Ok(out)
}

fn parse_type_annotation(ann: Option<&ast::Expr>) -> Option<Type> {
    match ann? {
        ast::Expr::Name(n) => match n.id.as_str() {
            "int" => Some(Type::I64),
            "float" => Some(Type::F64),
            "bool" => Some(Type::Bool),
            "str" => Some(Type::Str),
            "i8" => Some(Type::I8),
            "i16" => Some(Type::I16),
            "i32" => Some(Type::I32),
            "i64" => Some(Type::I64),
            other => {
                // User-defined class name?
                CLASS_REGISTRY
                    .with(|r| r.borrow().get(other).copied())
                    .map(Type::Class)
            }
        },
        // `tuple[T1, T2, ...]` or `list[T]` — Subscript on the type name.
        ast::Expr::Subscript(s) => {
            let head = match s.value.as_ref() {
                ast::Expr::Name(n) => n.id.as_str(),
                _ => return None,
            };
            match head {
                "tuple" => {
                    let elem_exprs: Vec<&ast::Expr> = match s.slice.as_ref() {
                        ast::Expr::Tuple(t) => t.elts.iter().collect(),
                        single => vec![single],
                    };
                    let mut elems = Vec::with_capacity(elem_exprs.len());
                    for e in elem_exprs {
                        elems.push(parse_type_annotation(Some(e))?);
                    }
                    Some(Type::Tuple(TupleId::intern(elems)))
                }
                "list" => {
                    let elem = parse_type_annotation(Some(s.slice.as_ref()))?;
                    Some(Type::List(ListId::intern(elem)))
                }
                "dict" => {
                    // `dict[K, V]` — slice is a Tuple.
                    let elem_exprs: Vec<&ast::Expr> = match s.slice.as_ref() {
                        ast::Expr::Tuple(t) => t.elts.iter().collect(),
                        _ => return None,
                    };
                    if elem_exprs.len() != 2 {
                        return None;
                    }
                    let k = parse_type_annotation(Some(elem_exprs[0]))?;
                    let v = parse_type_annotation(Some(elem_exprs[1]))?;
                    // v0.26: only I64 keys + I64 values.
                    if k != Type::I64 || v != Type::I64 {
                        return None;
                    }
                    Some(Type::Dict(DictId::intern(k, v)))
                }
                "set" => {
                    let elem = parse_type_annotation(Some(s.slice.as_ref()))?;
                    // v0.32: only I64 elements.
                    if elem != Type::I64 {
                        return None;
                    }
                    Some(Type::Set(SetId::intern(elem)))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn lower_expr(e: &ast::Expr, scope: &Scope, signatures: &SignatureTable) -> Result<TypedExpr> {
    match e {
        ast::Expr::Constant(c) => match &c.value {
            ast::Constant::Int(big) => {
                let v: i64 = big.try_into().map_err(|_| {
                    anyhow!("unsupported_feature: integer literal does not fit in i64")
                })?;
                Ok(TypedExpr::new(Type::I64, Expr::ConstI64(v)))
            }
            ast::Constant::Float(f) => Ok(TypedExpr::new(Type::F64, Expr::ConstF64(*f))),
            ast::Constant::Bool(b) => Ok(TypedExpr::new(Type::Bool, Expr::ConstBool(*b))),
            ast::Constant::Str(s) => {
                // Reject non-ASCII for now — the printer/repr story is
                // ASCII-only and Python escapes non-ASCII differently.
                if !s.is_ascii() {
                    bail!(
                        "unsupported_feature: non-ASCII string literals are not yet supported"
                    );
                }
                if s.bytes().any(|b| b == b'\\' || b == b'\'') {
                    bail!(
                        "unsupported_feature: string literals containing backslash or single-quote are not yet supported (printer can't escape them yet)"
                    );
                }
                Ok(TypedExpr::new(Type::Str, Expr::StrLit(s.clone())))
            }
            _ => bail!("unsupported_feature: only int / float / bool / str literals are supported"),
        },
        ast::Expr::Name(n) => {
            let name = n.id.as_str();
            let ty = scope.get(name).copied().ok_or_else(|| {
                anyhow!(
                    "unsupported_feature: name `{}` is not in scope (must be a parameter or previously assigned local)",
                    name
                )
            })?;
            Ok(TypedExpr::new(ty, Expr::Var(name.to_string())))
        }
        ast::Expr::BinOp(b) => {
            let op = match b.op {
                ast::Operator::Add => BinOp::Add,
                ast::Operator::Sub => BinOp::Sub,
                ast::Operator::Mult => BinOp::Mul,
                ast::Operator::FloorDiv => BinOp::FloorDiv,
                ast::Operator::Mod => BinOp::Mod,
                ast::Operator::Div => BinOp::TrueDiv,
                ast::Operator::Pow => BinOp::Pow,
                ast::Operator::MatMult => {
                    bail!("unsupported_feature: `@` (matmul) is not supported")
                }
                ast::Operator::LShift => BinOp::Shl,
                ast::Operator::RShift => BinOp::Shr,
                ast::Operator::BitAnd => BinOp::BitAnd,
                ast::Operator::BitOr => BinOp::BitOr,
                ast::Operator::BitXor => BinOp::BitXor,
            };
            let lhs = lower_expr(&b.left, scope, signatures)?;
            let rhs = lower_expr(&b.right, scope, signatures)?;
            apply_binop(op, lhs, rhs)
        }
        ast::Expr::UnaryOp(u) => {
            let operand = lower_expr(&u.operand, scope, signatures)?;
            match u.op {
                ast::UnaryOp::USub => apply_unop(UnaryOp::Neg, operand),
                ast::UnaryOp::UAdd => apply_unop(UnaryOp::Pos, operand),
                ast::UnaryOp::Not => {
                    let coerced = coerce(operand, Type::Bool)?;
                    Ok(TypedExpr::new(Type::Bool, Expr::Not(Box::new(coerced))))
                }
                ast::UnaryOp::Invert => apply_unop(UnaryOp::BitNot, operand),
            }
        }
        ast::Expr::Compare(c) => {
            // Special case: single-comparison `k in <container>` /
            // `k not in <container>`. Dispatched by container type.
            if c.ops.len() == 1
                && matches!(c.ops[0], ast::CmpOp::In | ast::CmpOp::NotIn)
            {
                let key = lower_expr(&c.left, scope, signatures)?;
                let container = lower_expr(&c.comparators[0], scope, signatures)?;
                let negated = matches!(c.ops[0], ast::CmpOp::NotIn);
                let result = match container.ty {
                    Type::Dict(id) => {
                        let key = coerce(key, id.key())?;
                        TypedExpr::new(
                            Type::Bool,
                            Expr::DictHas {
                                dict: Box::new(container),
                                key: Box::new(key),
                            },
                        )
                    }
                    Type::Set(id) => {
                        let key = coerce(key, id.elem())?;
                        TypedExpr::new(
                            Type::Bool,
                            Expr::SetHas {
                                set: Box::new(container),
                                key: Box::new(key),
                            },
                        )
                    }
                    other => bail!(
                        "unsupported_feature: `in` / `not in` on {} not supported (only dict / set so far)",
                        other.name()
                    ),
                };
                if negated {
                    return Ok(TypedExpr::new(Type::Bool, Expr::Not(Box::new(result))));
                }
                return Ok(result);
            }
            let first = lower_expr(&c.left, scope, signatures)?;
            let rest_ops: Result<Vec<(CmpOp, TypedExpr)>> = c
                .ops
                .iter()
                .zip(c.comparators.iter())
                .map(|(op, e)| Ok((convert_cmp_op(op)?, lower_expr(e, scope, signatures)?)))
                .collect();
            let rest = rest_ops?;
            if rest.len() == 1 {
                let (op, rhs) = rest.into_iter().next().unwrap();
                // Class instance == / != → dispatch to __eq__.
                if let (Type::Class(lc), Type::Class(rc)) = (first.ty, rhs.ty) {
                    if matches!(op, CmpOp::Eq | CmpOp::Ne) {
                        if lc != rc {
                            bail!(
                                "unsupported_feature: == between class instances must use the same class type ({} vs {})",
                                lc.name(),
                                rc.name()
                            );
                        }
                        let (_owner, mangled) = resolve_method(lc, "__eq__", signatures)
                            .ok_or_else(|| {
                                anyhow!(
                                    "unsupported_feature: class `{}` has no `__eq__` method (define one to use `==`)",
                                    lc.name()
                                )
                            })?;
                        let sig = signatures.get(&mangled).unwrap();
                        if sig.params.len() != 2 || sig.return_ty != Type::Bool {
                            bail!(
                                "unsupported_feature: `__eq__` on `{}` must take (self, other) and return `bool`",
                                lc.name()
                            );
                        }
                        let lhs_arg = coerce(first, sig.params[0].ty)?;
                        let rhs_arg = coerce(rhs, sig.params[1].ty)?;
                        let call = TypedExpr::new(
                            Type::Bool,
                            Expr::Call { callee: mangled, args: vec![lhs_arg, rhs_arg] },
                        );
                        if matches!(op, CmpOp::Ne) {
                            return Ok(TypedExpr::new(Type::Bool, Expr::Not(Box::new(call))));
                        }
                        return Ok(call);
                    } else {
                        bail!(
                            "unsupported_feature: only == and != are supported between class instances (no <, >, etc.)"
                        );
                    }
                }
                // String comparison: only ==/!= supported.
                if first.ty == Type::Str || rhs.ty == Type::Str {
                    if first.ty != Type::Str || rhs.ty != Type::Str {
                        bail!(
                            "unsupported_feature: cannot compare {} and {}",
                            first.ty.name(),
                            rhs.ty.name()
                        );
                    }
                    let negated = match op {
                        CmpOp::Eq => false,
                        CmpOp::Ne => true,
                        _ => bail!(
                            "unsupported_feature: only == and != are supported for strings (no lexicographic ordering yet)"
                        ),
                    };
                    return Ok(TypedExpr::new(
                        Type::Bool,
                        Expr::StrEq {
                            lhs: Box::new(first),
                            rhs: Box::new(rhs),
                            negated,
                        },
                    ));
                }
                let (lhs, rhs) = unify_cmp_operands(first, rhs)?;
                Ok(TypedExpr::new(
                    Type::Bool,
                    Expr::Cmp { op, lhs: Box::new(lhs), rhs: Box::new(rhs) },
                ))
            } else {
                Ok(TypedExpr::new(
                    Type::Bool,
                    Expr::CmpChain { first: Box::new(first), rest },
                ))
            }
        }
        ast::Expr::BoolOp(b) => {
            let op = match b.op {
                ast::BoolOp::And => BoolOp::And,
                ast::BoolOp::Or => BoolOp::Or,
            };
            if b.values.is_empty() {
                bail!("unsupported_feature: empty BoolOp");
            }
            let mut iter = b.values.iter();
            let first = lower_expr(iter.next().unwrap(), scope, signatures)?;
            let mut acc = first;
            for next in iter {
                let next = lower_expr(next, scope, signatures)?;
                let (l, r) = unify_numeric(acc, next)?;
                let ty = l.ty;
                acc = TypedExpr::new(
                    ty,
                    Expr::BoolOp { op, lhs: Box::new(l), rhs: Box::new(r) },
                );
            }
            Ok(acc)
        }
        ast::Expr::Call(c) => {
            // `super().method(args)` — resolved at compile time to a
            // direct call into the parent class's method with `self` as
            // the receiver. The form `super()` standalone is not
            // supported (we don't synthesize a proxy object).
            if let ast::Expr::Attribute(attr) = c.func.as_ref() {
                if let ast::Expr::Call(inner) = attr.value.as_ref() {
                    if matches!(inner.func.as_ref(), ast::Expr::Name(n) if n.id.as_str() == "super")
                    {
                        if !inner.args.is_empty() || !inner.keywords.is_empty() {
                            bail!(
                                "unsupported_feature: only zero-arg `super()` is supported"
                            );
                        }
                        let cur_class = CURRENT_CLASS
                            .with(|c| *c.borrow())
                            .ok_or_else(|| {
                                anyhow!(
                                    "unsupported_feature: `super()` is only valid inside a method"
                                )
                            })?;
                        let parent = cur_class.parent().ok_or_else(|| {
                            anyhow!(
                                "unsupported_feature: class `{}` has no parent — `super()` is invalid here",
                                cur_class.name()
                            )
                        })?;
                        let method = attr.attr.as_str();
                        let (_resolved_class, mangled) =
                            resolve_method(parent, method, signatures).ok_or_else(|| {
                                anyhow!(
                                    "unsupported_feature: no method `{}` found on `{}` or its ancestors",
                                    method,
                                    parent.name()
                                )
                            })?;
                        let sig = signatures.get(&mangled).unwrap();
                        // self is always the receiver — pull it from scope.
                        let self_ty = *scope.get("self").ok_or_else(|| {
                            anyhow!(
                                "unsupported_feature: `super()` requires a `self` in scope"
                            )
                        })?;
                        let self_expr = TypedExpr::new(self_ty, Expr::Var("self".to_string()));
                        let mut args: Vec<TypedExpr> = Vec::with_capacity(c.args.len() + 1);
                        args.push(coerce(self_expr, sig.params[0].ty)?);
                        if c.args.len() != sig.params.len() - 1 {
                            bail!(
                                "unsupported_feature: super().{} takes {} args, got {}",
                                method,
                                sig.params.len() - 1,
                                c.args.len()
                            );
                        }
                        if !c.keywords.is_empty() {
                            bail!(
                                "unsupported_feature: keyword args on super() method calls not supported"
                            );
                        }
                        for (i, a) in c.args.iter().enumerate() {
                            let raw = lower_expr(a, scope, signatures)?;
                            args.push(coerce(raw, sig.params[i + 1].ty)?);
                        }
                        return Ok(TypedExpr::new(
                            sig.return_ty,
                            Expr::Call { callee: mangled, args },
                        ));
                    }
                }
            }
            // Method-call form: `obj.method(args...)` lowers to a regular
            // call to the mangled function `<ClassName>.<method>` with
            // obj prepended as `self`.
            if let ast::Expr::Attribute(attr) = c.func.as_ref() {
                let obj = lower_expr(&attr.value, scope, signatures)?;
                if let Type::Class(class_id) = obj.ty {
                    let method = attr.attr.as_str();
                    let (_resolved_class, mangled) =
                        resolve_method(class_id, method, signatures).ok_or_else(|| {
                            anyhow!(
                                "unsupported_feature: class `{}` has no method `{}` (and no ancestor defines it)",
                                class_id.name(),
                                method
                            )
                        })?;
                    let sig = signatures.get(&mangled).ok_or_else(|| {
                        anyhow!(
                            "unsupported_feature: class `{}` has no method `{}`",
                            class_id.name(),
                            method
                        )
                    })?;
                    let mut args: Vec<TypedExpr> = Vec::with_capacity(c.args.len() + 1);
                    args.push(coerce(obj, sig.params[0].ty)?);
                    if c.args.len() != sig.params.len() - 1 {
                        bail!(
                            "unsupported_feature: method `{}.{}` takes {} args, got {}",
                            class_id.name(),
                            method,
                            sig.params.len() - 1,
                            c.args.len()
                        );
                    }
                    if !c.keywords.is_empty() {
                        bail!(
                            "unsupported_feature: keyword args on method calls not supported"
                        );
                    }
                    for (i, a) in c.args.iter().enumerate() {
                        let raw = lower_expr(a, scope, signatures)?;
                        args.push(coerce(raw, sig.params[i + 1].ty)?);
                    }
                    return Ok(TypedExpr::new(
                        sig.return_ty,
                        Expr::Call { callee: mangled, args },
                    ));
                }
                bail!(
                    "unsupported_feature: method calls only supported on class instances (got {})",
                    obj.ty.name()
                );
            }
            let callee = match c.func.as_ref() {
                ast::Expr::Name(n) => n.id.as_str().to_string(),
                _ => bail!(
                    "unsupported_feature: only direct calls to top-level functions are supported"
                ),
            };
            // Class constructor: `Foo(args)` if Foo is a registered class.
            if let Some(class_id) = CLASS_REGISTRY.with(|r| r.borrow().get(&callee).copied()) {
                if class_id.is_abstract() {
                    bail!(
                        "unsupported_feature: cannot instantiate abstract class `{}`: missing implementations of {:?}",
                        callee,
                        class_id.abstract_methods()
                    );
                }
                // Resolve __init__ via the inheritance chain — a subclass
                // without its own __init__ uses the parent's. If none
                // exists anywhere AND no args are passed, allocate
                // without calling any init (Python's implicit
                // `object.__init__()`).
                let resolved = resolve_method(class_id, "__init__", signatures);
                let (init_class, init_name) = match resolved {
                    Some(r) => r,
                    None => {
                        if !c.args.is_empty() || !c.keywords.is_empty() {
                            bail!(
                                "unsupported_feature: class `{}` has no __init__ method (and no ancestor defines one), so `{}()` must be called with no arguments",
                                callee,
                                callee
                            );
                        }
                        return Ok(TypedExpr::new(
                            Type::Class(class_id),
                            Expr::ClassNew {
                                class: class_id,
                                init_class: None,
                                args: Vec::new(),
                            },
                        ));
                    }
                };
                let init_sig = signatures.get(&init_name).unwrap();
                if c.args.len() != init_sig.params.len() - 1 {
                    bail!(
                        "unsupported_feature: `{}` __init__ takes {} args, got {}",
                        callee,
                        init_sig.params.len() - 1,
                        c.args.len()
                    );
                }
                if !c.keywords.is_empty() {
                    bail!("unsupported_feature: keyword args on class constructor not supported");
                }
                let mut args: Vec<TypedExpr> = Vec::with_capacity(c.args.len());
                for (i, a) in c.args.iter().enumerate() {
                    let raw = lower_expr(a, scope, signatures)?;
                    args.push(coerce(raw, init_sig.params[i + 1].ty)?);
                }
                return Ok(TypedExpr::new(
                    Type::Class(class_id),
                    Expr::ClassNew { class: class_id, init_class: Some(init_class), args },
                ));
            }
            // Special-case Python builtins.
            if let Some(builtin) = lower_builtin_call(&callee, &c.args, &c.keywords, scope, signatures)? {
                return Ok(builtin);
            }
            let sig = signatures.get(&callee).ok_or_else(|| {
                anyhow!("unsupported_feature: call to undefined function `{}`", callee)
            })?;
            let args = resolve_call_args(&callee, sig, &c.args, &c.keywords, scope, signatures)?;
            Ok(TypedExpr::new(sig.return_ty, Expr::Call { callee, args }))
        }
        ast::Expr::Attribute(attr) => {
            // Field read: `obj.field`. Method references (e.g. assigning
            // `f = obj.method`) are not supported — methods are only
            // callable directly via `obj.method(args)`, handled in Call.
            let obj = lower_expr(&attr.value, scope, signatures)?;
            let class_id = match obj.ty {
                Type::Class(id) => id,
                other => bail!(
                    "unsupported_feature: attribute access on non-class type {} not supported",
                    other.name()
                ),
            };
            let field_name = attr.attr.as_str();
            let field_index = class_id.field_index(field_name).ok_or_else(|| {
                anyhow!(
                    "unsupported_feature: class `{}` has no field `{}`",
                    class_id.name(),
                    field_name
                )
            })?;
            let field_ty = class_id.field_ty(field_name).unwrap();
            Ok(TypedExpr::new(
                field_ty,
                Expr::FieldGet {
                    obj: Box::new(obj),
                    field_index,
                },
            ))
        }
        ast::Expr::Tuple(t) => {
            let elements: Result<Vec<TypedExpr>> = t
                .elts
                .iter()
                .map(|e| lower_expr(e, scope, signatures))
                .collect();
            let elements = elements?;
            let elem_types: Vec<Type> = elements.iter().map(|e| e.ty).collect();
            let id = TupleId::intern(elem_types);
            Ok(TypedExpr::new(
                Type::Tuple(id),
                Expr::TupleLit { elements },
            ))
        }
        ast::Expr::Dict(d) => {
            // `{k: v, ...}` literal. v0.26: I64 → I64 only.
            // Empty `{}` is lowered as `DictLit { entries: [] }` and can be
            // coerced to any concrete `dict[K, V]` via the empty-dict re-tag
            // in `coerce` — same trick as empty lists.
            // dict.keys is Vec<Option<Expr>> where None means **unpack.
            let mut entries: Vec<(TypedExpr, TypedExpr)> = Vec::with_capacity(d.keys.len());
            for (k, v) in d.keys.iter().zip(d.values.iter()) {
                let k = k.as_ref().ok_or_else(|| {
                    anyhow!("unsupported_feature: dict `**` unpacking is not supported")
                })?;
                let lk = lower_expr(k, scope, signatures)?;
                let lv = lower_expr(v, scope, signatures)?;
                let lk = coerce(lk, Type::I64)?;
                let lv = coerce(lv, Type::I64)?;
                entries.push((lk, lv));
            }
            let id = DictId::intern(Type::I64, Type::I64);
            Ok(TypedExpr::new(Type::Dict(id), Expr::DictLit { entries }))
        }
        ast::Expr::Set(s) => {
            // `{e1, e2, ...}` set literal. v0.32: i64 elements only.
            // (Empty `{}` is a dict in Python, not a set — handled in the
            // Dict arm and re-tagged via coerce.)
            let mut elements: Vec<TypedExpr> = Vec::with_capacity(s.elts.len());
            for e in &s.elts {
                let v = lower_expr(e, scope, signatures)?;
                let v = coerce(v, Type::I64)?;
                elements.push(v);
            }
            let id = SetId::intern(Type::I64);
            Ok(TypedExpr::new(Type::Set(id), Expr::SetLit { elements }))
        }
        ast::Expr::ListComp(comp) => {
            // [<elt> for <target> in <iter> (if <cond>)*]
            // Only one generator supported in v0.21.
            if comp.generators.len() != 1 {
                bail!(
                    "unsupported_feature: nested generators in list comprehensions are not supported"
                );
            }
            let gen = &comp.generators[0];
            if gen.is_async {
                bail!("unsupported_feature: async comprehensions are not supported");
            }
            let target_name = match &gen.target {
                ast::Expr::Name(n) => n.id.as_str().to_string(),
                _ => bail!(
                    "unsupported_feature: comprehension target must be a simple name"
                ),
            };

            // Determine target's type from the iter.
            let is_range_call = matches!(
                &gen.iter,
                ast::Expr::Call(c) if matches!(c.func.as_ref(), ast::Expr::Name(n) if n.id.as_str() == "range")
            );

            // Build a fresh inner scope so loop var + accumulator don't leak.
            let mut inner_scope = scope.clone();

            // Synthesise unique names for accumulator and (if iterating
            // over a list) helper vars. Use the global scope size as a
            // unique suffix.
            let uniq = scope.len();
            let acc_name = format!("__compr_acc_{}", uniq);
            let lst_name = format!("__compr_lst_{}", uniq);
            let idx_name = format!("__compr_idx_{}", uniq);

            // Lower the iter and figure out the element type.
            let elem_ty: Type;
            let iter_lowered: Option<TypedExpr>;
            let start_stop_step: Option<(TypedExpr, TypedExpr, TypedExpr)>;
            if is_range_call {
                let (start, stop, step) =
                    parse_and_lower_range(&gen.iter, &inner_scope, signatures)?;
                let step_value = match &step.expr {
                    Expr::ConstI64(v) => *v,
                    _ => bail!("unsupported_feature: range step must be a constant int literal"),
                };
                if step_value <= 0 {
                    bail!("unsupported_feature: range step must be a positive int literal");
                }
                elem_ty = Type::I64;
                iter_lowered = None;
                start_stop_step = Some((start, stop, step));
                inner_scope.insert(target_name.clone(), Type::I64);
            } else {
                let iter = lower_expr(&gen.iter, &inner_scope, signatures)?;
                let elem = match iter.ty {
                    Type::List(id) => id.elem(),
                    other => bail!(
                        "unsupported_feature: comprehension iter must be range(...) or list[T] (got {})",
                        other.name()
                    ),
                };
                elem_ty = elem;
                iter_lowered = Some(iter);
                start_stop_step = None;
                inner_scope.insert(target_name.clone(), elem);
            }

            // Lower the elt expression in the inner scope.
            let elt_lowered = lower_expr(&comp.elt, &inner_scope, signatures)?;
            let result_elem_ty = elt_lowered.ty;
            let acc_list_id = ListId::intern(result_elem_ty);
            let acc_ty = Type::List(acc_list_id);

            // Lower any `if` clauses (Python supports multiple, AND-ed).
            // Combine into a single Bool expression with ad-hoc And.
            let mut filter_cond: Option<TypedExpr> = None;
            for if_e in &gen.ifs {
                let c = lower_expr(if_e, &inner_scope, signatures)?;
                let c = coerce(c, Type::Bool)?;
                filter_cond = Some(match filter_cond {
                    None => c,
                    Some(prev) => TypedExpr::new(
                        Type::Bool,
                        Expr::BoolOp {
                            op: BoolOp::And,
                            lhs: Box::new(coerce(prev, Type::I64)?),
                            rhs: Box::new(coerce(c, Type::I64)?),
                        },
                    ),
                });
            }
            // Renormalise filter_cond back to Bool for the If statement.
            let filter_cond_bool = filter_cond.map(|c| coerce(c, Type::Bool).unwrap());

            // Build the body of the loop:
            //   one_elem = [elt]
            //   if cond: acc = acc + one_elem
            //   else: skip
            let one_elem_list = TypedExpr::new(
                acc_ty,
                Expr::ListLit { elements: vec![elt_lowered] },
            );
            let append = Stmt::Let {
                name: acc_name.clone(),
                value: TypedExpr::new(
                    acc_ty,
                    Expr::ListConcat {
                        lhs: Box::new(TypedExpr::new(acc_ty, Expr::Var(acc_name.clone()))),
                        rhs: Box::new(one_elem_list),
                    },
                ),
            };
            let body_stmt = match filter_cond_bool {
                Some(cond) => Stmt::If {
                    cond,
                    then_body: vec![append],
                    else_body: vec![],
                },
                None => append,
            };

            // Build the surrounding `for` desugar.
            let mut stmts: Vec<Stmt> = Vec::new();
            // acc = []
            stmts.push(Stmt::Let {
                name: acc_name.clone(),
                value: TypedExpr::new(
                    acc_ty,
                    Expr::ListLit { elements: Vec::new() },
                ),
            });

            if let Some((start, stop, step)) = start_stop_step {
                // for target in range(...):
                stmts.push(Stmt::Let { name: target_name.clone(), value: start });
                let cond = TypedExpr::new(
                    Type::Bool,
                    Expr::Cmp {
                        op: CmpOp::Lt,
                        lhs: Box::new(TypedExpr::new(Type::I64, Expr::Var(target_name.clone()))),
                        rhs: Box::new(stop),
                    },
                );
                let mut wbody = vec![body_stmt];
                wbody.push(Stmt::Let {
                    name: target_name.clone(),
                    value: TypedExpr::new(
                        Type::I64,
                        Expr::BinOp {
                            op: BinOp::Add,
                            lhs: Box::new(TypedExpr::new(Type::I64, Expr::Var(target_name.clone()))),
                            rhs: Box::new(step),
                        },
                    ),
                });
                stmts.push(Stmt::While { cond, body: wbody });
            } else {
                // for target in <list>:
                let iter = iter_lowered.unwrap();
                let iter_ty = iter.ty;
                stmts.push(Stmt::Let { name: lst_name.clone(), value: iter });
                stmts.push(Stmt::Let {
                    name: idx_name.clone(),
                    value: TypedExpr::new(Type::I64, Expr::ConstI64(0)),
                });
                let lst_ref = TypedExpr::new(iter_ty, Expr::Var(lst_name.clone()));
                let cond = TypedExpr::new(
                    Type::Bool,
                    Expr::Cmp {
                        op: CmpOp::Lt,
                        lhs: Box::new(TypedExpr::new(Type::I64, Expr::Var(idx_name.clone()))),
                        rhs: Box::new(TypedExpr::new(
                            Type::I64,
                            Expr::ListLen { list: Box::new(lst_ref.clone()) },
                        )),
                    },
                );
                let mut wbody = vec![Stmt::Let {
                    name: target_name.clone(),
                    value: TypedExpr::new(
                        elem_ty,
                        Expr::ListIndex {
                            list: Box::new(lst_ref),
                            index: Box::new(TypedExpr::new(Type::I64, Expr::Var(idx_name.clone()))),
                        },
                    ),
                }];
                wbody.push(body_stmt);
                wbody.push(Stmt::Let {
                    name: idx_name.clone(),
                    value: TypedExpr::new(
                        Type::I64,
                        Expr::BinOp {
                            op: BinOp::Add,
                            lhs: Box::new(TypedExpr::new(Type::I64, Expr::Var(idx_name))),
                            rhs: Box::new(TypedExpr::new(Type::I64, Expr::ConstI64(1))),
                        },
                    ),
                });
                stmts.push(Stmt::While { cond, body: wbody });
            }

            return Ok(TypedExpr::new(
                acc_ty,
                Expr::DoBlock {
                    stmts,
                    result: Box::new(TypedExpr::new(acc_ty, Expr::Var(acc_name))),
                },
            ));
        }
        ast::Expr::List(l) => {
            // List literal: `[a, b, c]`. All elements must be coercible
            // to a common type. Empty `[]` produces a List of element
            // type "Unknown" — we use I64 as a placeholder and rely on
            // the assignment/return context to coerce. (For
            // empty-list-with-annotation we'd ideally infer from the
            // annotation; the AnnAssign path already does that since
            // it lowers RHS first then coerces. Empty `[]` becomes
            // `List(I64)` by default; for `list[float]` it's coerced.)
            if l.elts.is_empty() {
                let id = ListId::intern(Type::I64);
                return Ok(TypedExpr::new(
                    Type::List(id),
                    Expr::ListLit { elements: Vec::new() },
                ));
            }
            let lowered: Result<Vec<TypedExpr>> = l
                .elts
                .iter()
                .map(|e| lower_expr(e, scope, signatures))
                .collect();
            let mut lowered = lowered?;
            // Pick element type as the unification of all element types
            // by repeated unify_numeric (since v0.19 only supports
            // numeric element types).
            let mut elem_ty = lowered[0].ty;
            for next in &lowered[1..] {
                let (a, _) = unify_numeric(
                    TypedExpr::new(elem_ty, Expr::ConstI64(0)), // placeholder for ty
                    next.clone(),
                )?;
                elem_ty = a.ty;
            }
            // Coerce each element to the unified type.
            for e in lowered.iter_mut() {
                *e = coerce(e.clone(), elem_ty)?;
            }
            let id = ListId::intern(elem_ty);
            Ok(TypedExpr::new(
                Type::List(id),
                Expr::ListLit { elements: lowered },
            ))
        }
        ast::Expr::Subscript(s) => {
            let value = lower_expr(&s.value, scope, signatures)?;
            // List subscripting goes through ListIndex (runtime index).
            if let Type::List(_) = value.ty {
                let index = lower_expr(&s.slice, scope, signatures)?;
                let index = coerce(index, Type::I64)?;
                let elem_ty = match value.ty {
                    Type::List(id) => id.elem(),
                    _ => unreachable!(),
                };
                return Ok(TypedExpr::new(
                    elem_ty,
                    Expr::ListIndex {
                        list: Box::new(value),
                        index: Box::new(index),
                    },
                ));
            }
            // Dict subscripting → DictGet.
            if let Type::Dict(id) = value.ty {
                let key = lower_expr(&s.slice, scope, signatures)?;
                let key = coerce(key, id.key())?;
                return Ok(TypedExpr::new(
                    id.val(),
                    Expr::DictGet {
                        dict: Box::new(value),
                        key: Box::new(key),
                    },
                ));
            }
            // String indexing / slicing.
            if value.ty == Type::Str {
                if let ast::Expr::Slice(slc) = s.slice.as_ref() {
                    if slc.step.is_some() {
                        bail!(
                            "unsupported_feature: string slice step (`s[::2]`) is not supported"
                        );
                    }
                    reject_negative_index_expr(slc.lower.as_deref(), "string slice lower bound")?;
                    reject_negative_index_expr(slc.upper.as_deref(), "string slice upper bound")?;
                    let start = match slc.lower.as_deref() {
                        Some(e) => {
                            let v = lower_expr(e, scope, signatures)?;
                            coerce(v, Type::I64)?
                        }
                        None => TypedExpr::new(Type::I64, Expr::ConstI64(0)),
                    };
                    let stop = match slc.upper.as_deref() {
                        Some(e) => {
                            let v = lower_expr(e, scope, signatures)?;
                            coerce(v, Type::I64)?
                        }
                        None => TypedExpr::new(
                            Type::I64,
                            Expr::StrLen { s: Box::new(value.clone()) },
                        ),
                    };
                    return Ok(TypedExpr::new(
                        Type::Str,
                        Expr::StrSlice {
                            s: Box::new(value),
                            start: Box::new(start),
                            stop: Box::new(stop),
                        },
                    ));
                }
                reject_negative_index_expr(Some(s.slice.as_ref()), "string index")?;
                let index = lower_expr(&s.slice, scope, signatures)?;
                let index = coerce(index, Type::I64)?;
                return Ok(TypedExpr::new(
                    Type::Str,
                    Expr::StrIndex {
                        s: Box::new(value),
                        index: Box::new(index),
                    },
                ));
            }
            let id = match value.ty {
                Type::Tuple(id) => id,
                other => bail!(
                    "unsupported_feature: subscripting non-tuple/list type {} is not supported",
                    other.name()
                ),
            };
            let index_value = match s.slice.as_ref() {
                ast::Expr::Constant(c) => match &c.value {
                    ast::Constant::Int(big) => {
                        let v: i64 = big
                            .try_into()
                            .map_err(|_| anyhow!("tuple index doesn't fit in i64"))?;
                        v
                    }
                    _ => bail!("unsupported_feature: tuple index must be an integer literal"),
                },
                ast::Expr::UnaryOp(u) if matches!(u.op, ast::UnaryOp::USub) => {
                    match u.operand.as_ref() {
                        ast::Expr::Constant(c) => match &c.value {
                            ast::Constant::Int(big) => {
                                let v: i64 = big
                                    .try_into()
                                    .map_err(|_| anyhow!("tuple index doesn't fit in i64"))?;
                                -v
                            }
                            _ => bail!("unsupported_feature: tuple index must be an integer literal"),
                        },
                        _ => bail!("unsupported_feature: tuple index must be an integer literal"),
                    }
                }
                _ => bail!(
                    "unsupported_feature: tuple index must be a constant integer literal"
                ),
            };
            let n = id.with_elems(|elems| elems.len()) as i64;
            let idx = if index_value < 0 { n + index_value } else { index_value };
            if idx < 0 || idx >= n {
                bail!(
                    "unsupported_feature: tuple index {} out of range for {}",
                    index_value,
                    value.ty.name()
                );
            }
            let elem_ty = id.with_elems(|elems| elems[idx as usize]);
            Ok(TypedExpr::new(
                elem_ty,
                Expr::TupleIndex {
                    tuple: Box::new(value),
                    index: idx as usize,
                },
            ))
        }
        ast::Expr::JoinedStr(js) => lower_joined_str(&js.values, scope, signatures),
        other => bail!(
            "unsupported_feature: expression form `{}` is not supported",
            expr_kind_name(other)
        ),
    }
}

/// Reject `Type::Class(c)` for an abstract `c` used as a value type.
/// `what` describes the context for the error message ("parameter type",
/// "return type", "local variable type"). Until vtables land, abstract
/// classes can only appear as base-class specifications, never as the
/// static type of a value.
fn reject_abstract_type(ty: Type, what: &str) -> Result<()> {
    if let Type::Class(c) = ty {
        if c.is_abstract() {
            bail!(
                "unsupported_feature: using abstract class `{}` as a {} is deferred to the vtable slice — polymorphism on ABCs needs dynamic dispatch",
                c.name(),
                what
            );
        }
    }
    Ok(())
}

/// Resolve a method on a class by walking the inheritance chain.
/// Returns the mangled name (`<ClassName>.<method>`) of the first
/// matching definition, walking from `class_id` up via `parent()`.
fn resolve_method(
    class_id: ClassId,
    method: &str,
    signatures: &SignatureTable,
) -> Option<(ClassId, String)> {
    let mut cur = Some(class_id);
    while let Some(c) = cur {
        let mangled = format!("{}.{}", c.name(), method);
        if signatures.contains_key(&mangled) {
            return Some((c, mangled));
        }
        cur = c.parent();
    }
    None
}

/// Test whether `descendant` inherits transitively from `ancestor`
/// (including equality).
fn is_subclass_of(descendant: ClassId, ancestor: ClassId) -> bool {
    let mut cur = Some(descendant);
    while let Some(c) = cur {
        if c == ancestor {
            return true;
        }
        cur = c.parent();
    }
    false
}

/// Reject a syntactic negative literal in an index/bound position.
/// Used by string indexing/slicing in v0.31 — Python's negative indexing
/// is deferred. Runtime negative values are not detected; this only
/// catches the literal form `s[-1]` / `s[-3:-1]`.
fn reject_negative_index_expr(e: Option<&ast::Expr>, what: &str) -> Result<()> {
    let Some(e) = e else { return Ok(()) };
    if let ast::Expr::UnaryOp(u) = e {
        if matches!(u.op, ast::UnaryOp::USub) {
            bail!(
                "unsupported_feature: negative {} (`{}`) is not supported (Python negative-indexing is deferred)",
                what,
                "-..."
            );
        }
    }
    Ok(())
}

/// Lower an f-string (`ast::Expr::JoinedStr`) to a chain of `StrConcat`
/// over `StrLit` and `FormatToStr` (for non-Str interpolations) /
/// passthrough (for Str interpolations).
fn lower_joined_str(
    values: &[ast::Expr],
    scope: &Scope,
    signatures: &SignatureTable,
) -> Result<TypedExpr> {
    // Empty f-string `f""` would be a JoinedStr with no values.
    if values.is_empty() {
        return Ok(TypedExpr::new(Type::Str, Expr::StrLit(String::new())));
    }
    let mut segments: Vec<TypedExpr> = Vec::with_capacity(values.len());
    for v in values {
        let seg = match v {
            ast::Expr::Constant(c) => match &c.value {
                ast::Constant::Str(s) => {
                    if !s.is_ascii() {
                        bail!(
                            "unsupported_feature: non-ASCII text in f-string is not yet supported"
                        );
                    }
                    if s.bytes().any(|b| b == b'\\' || b == b'\'') {
                        bail!(
                            "unsupported_feature: backslash or single-quote inside f-string is not yet supported"
                        );
                    }
                    TypedExpr::new(Type::Str, Expr::StrLit(s.clone()))
                }
                _ => bail!(
                    "unsupported_feature: only str literal segments are allowed in f-strings"
                ),
            },
            ast::Expr::FormattedValue(fv) => {
                if !fv.conversion.is_none() {
                    bail!(
                        "unsupported_feature: f-string conversions (`!r`, `!s`, `!a`) are not supported"
                    );
                }
                if fv.format_spec.is_some() {
                    bail!(
                        "unsupported_feature: f-string format specs (`{{x:.2f}}`, etc.) are not supported"
                    );
                }
                let inner = lower_expr(&fv.value, scope, signatures)?;
                match inner.ty {
                    Type::Str => inner,
                    Type::I8 | Type::I16 | Type::I32 | Type::I64 | Type::Bool => {
                        TypedExpr::new(Type::Str, Expr::FormatToStr { inner: Box::new(inner) })
                    }
                    Type::F64 => bail!(
                        "unsupported_feature: f64 interpolation in f-strings is deferred (see specs/slice-v0.30-fstrings.md)"
                    ),
                    Type::Class(cid) => {
                        // Class instance — resolve __repr__ via the
                        // inheritance chain and call it. The result is
                        // a Str that flows into the StrConcat.
                        let (_repr_owner, mangled) = resolve_method(cid, "__repr__", signatures)
                            .ok_or_else(|| {
                                anyhow!(
                                    "unsupported_feature: class `{}` has no `__repr__` method (define one to interpolate it in an f-string)",
                                    cid.name()
                                )
                            })?;
                        let sig = signatures.get(&mangled).unwrap();
                        if sig.params.len() != 1 || sig.return_ty != Type::Str {
                            bail!(
                                "unsupported_feature: `__repr__` on `{}` must take only `self` and return `str`",
                                cid.name()
                            );
                        }
                        let arg = coerce(inner, sig.params[0].ty)?;
                        TypedExpr::new(
                            Type::Str,
                            Expr::Call { callee: mangled, args: vec![arg] },
                        )
                    }
                    other => bail!(
                        "unsupported_feature: cannot interpolate value of type {} in an f-string",
                        other.name()
                    ),
                }
            }
            other => bail!(
                "unsupported_feature: f-string segment kind `{}` is not supported",
                expr_kind_name(other)
            ),
        };
        segments.push(seg);
    }
    // Fold segments left-to-right via StrConcat.
    let mut iter = segments.into_iter();
    let first = iter.next().unwrap();
    let folded = iter.fold(first, |acc, seg| {
        TypedExpr::new(
            Type::Str,
            Expr::StrConcat { lhs: Box::new(acc), rhs: Box::new(seg) },
        )
    });
    Ok(folded)
}

/// Apply a binary op given lowered operands. Handles type promotion
/// and inserts coercions as needed.
fn apply_binop(op: BinOp, lhs: TypedExpr, rhs: TypedExpr) -> Result<TypedExpr> {
    match op {
        BinOp::TrueDiv => {
            // Always F64 result. Promote both to F64.
            let l = coerce(lhs, Type::F64)?;
            let r = coerce(rhs, Type::F64)?;
            Ok(TypedExpr::new(
                Type::F64,
                Expr::BinOp { op, lhs: Box::new(l), rhs: Box::new(r) },
            ))
        }
        BinOp::FloorDiv | BinOp::Mod => {
            // Reject float (Python supports float `%` but we don't yet).
            if lhs.ty == Type::F64 || rhs.ty == Type::F64 {
                bail!(
                    "unsupported_feature: `//` / `%` on float operands not yet supported"
                );
            }
            // Unify int widths.
            let (l, r) = unify_int_widths(lhs, rhs)?;
            let ty = l.ty;
            Ok(TypedExpr::new(
                ty,
                Expr::BinOp { op, lhs: Box::new(l), rhs: Box::new(r) },
            ))
        }
        BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
            if lhs.ty == Type::F64 || rhs.ty == Type::F64 {
                bail!(
                    "unsupported_feature: bitwise ops on float operands are not allowed"
                );
            }
            let (l, r) = unify_int_widths(lhs, rhs)?;
            let ty = l.ty;
            Ok(TypedExpr::new(
                ty,
                Expr::BinOp { op, lhs: Box::new(l), rhs: Box::new(r) },
            ))
        }
        BinOp::Shl | BinOp::Shr => {
            if lhs.ty == Type::F64 || rhs.ty == Type::F64 {
                bail!("unsupported_feature: shift ops on float operands are not allowed");
            }
            // Result type follows lhs width; rhs is coerced to lhs's width.
            let lhs = coerce_int_keep_width(lhs)?;
            let target = lhs.ty;
            let r = coerce(rhs, target)?;
            Ok(TypedExpr::new(
                target,
                Expr::BinOp { op, lhs: Box::new(lhs), rhs: Box::new(r) },
            ))
        }
        BinOp::Pow => {
            // Int**Int → I64 (using runtime helper, always i64 result for
            // simplicity; sub-i64 ints would be widened). Float**Float → F64. Mixed → F64.
            let result_ty = if lhs.ty == Type::F64 || rhs.ty == Type::F64 {
                Type::F64
            } else {
                Type::I64
            };
            let l = coerce(lhs, result_ty)?;
            let r = coerce(rhs, result_ty)?;
            Ok(TypedExpr::new(
                result_ty,
                Expr::BinOp { op, lhs: Box::new(l), rhs: Box::new(r) },
            ))
        }
        BinOp::Add | BinOp::Sub | BinOp::Mul => {
            // List concatenation: list[T] + list[T] = list[T].
            if let (Type::List(a), Type::List(b)) = (lhs.ty, rhs.ty) {
                if op != BinOp::Add {
                    bail!("unsupported_feature: only `+` is defined on lists (no `-`/`*` yet)");
                }
                if a != b {
                    bail!(
                        "unsupported_feature: cannot concatenate {} and {} (element types must match)",
                        lhs.ty.name(),
                        rhs.ty.name()
                    );
                }
                return Ok(TypedExpr::new(
                    lhs.ty,
                    Expr::ListConcat { lhs: Box::new(lhs), rhs: Box::new(rhs) },
                ));
            }
            // String concatenation: str + str = str.
            if lhs.ty == Type::Str && rhs.ty == Type::Str {
                if op != BinOp::Add {
                    bail!(
                        "unsupported_feature: only `+` is defined on strings"
                    );
                }
                return Ok(TypedExpr::new(
                    Type::Str,
                    Expr::StrConcat { lhs: Box::new(lhs), rhs: Box::new(rhs) },
                ));
            }
            if lhs.ty == Type::Str || rhs.ty == Type::Str {
                bail!(
                    "unsupported_feature: arithmetic between {} and {} is not supported",
                    lhs.ty.name(),
                    rhs.ty.name()
                );
            }
            let (l, r) = unify_numeric(lhs, rhs)?;
            let ty = l.ty;
            Ok(TypedExpr::new(
                ty,
                Expr::BinOp { op, lhs: Box::new(l), rhs: Box::new(r) },
            ))
        }
    }
}

/// Unify two int-shaped operands to the wider int width. Bool counts
/// as I64 (Python-ish). Caller must have already verified neither is F64.
fn unify_int_widths(lhs: TypedExpr, rhs: TypedExpr) -> Result<(TypedExpr, TypedExpr)> {
    let lty = if lhs.ty == Type::Bool { Type::I64 } else { lhs.ty };
    let rty = if rhs.ty == Type::Bool { Type::I64 } else { rhs.ty };
    let target = match (lty.int_width(), rty.int_width()) {
        (Some(lw), Some(rw)) => match lw.max(rw) {
            8 => Type::I8,
            16 => Type::I16,
            32 => Type::I32,
            _ => Type::I64,
        },
        _ => bail!(
            "unsupported_feature: cannot unify {} and {} as ints",
            lhs.ty.name(),
            rhs.ty.name()
        ),
    };
    Ok((coerce(lhs, target)?, coerce(rhs, target)?))
}

/// If the expression is a Bool, coerce it to I64; otherwise pass through.
/// Used where we want the int-shaped width of the operand to drive the result.
fn coerce_int_keep_width(e: TypedExpr) -> Result<TypedExpr> {
    if e.ty == Type::Bool {
        coerce(e, Type::I64)
    } else if e.ty.is_int() {
        Ok(e)
    } else {
        bail!("unsupported_feature: expected int-shaped operand, got {}", e.ty.name())
    }
}

fn apply_unop(op: UnaryOp, operand: TypedExpr) -> Result<TypedExpr> {
    match op {
        UnaryOp::Neg | UnaryOp::Pos => {
            // Numeric: keep type; Bool → I64 first.
            let operand = if operand.ty == Type::Bool {
                coerce(operand, Type::I64)?
            } else {
                operand
            };
            let ty = operand.ty;
            Ok(TypedExpr::new(
                ty,
                Expr::UnaryOp { op, operand: Box::new(operand) },
            ))
        }
        UnaryOp::BitNot => {
            // Bitwise not on int operand of any width.
            let operand = coerce_int_keep_width(operand)?;
            let ty = operand.ty;
            Ok(TypedExpr::new(
                ty,
                Expr::UnaryOp { op, operand: Box::new(operand) },
            ))
        }
    }
}

/// Numeric promotion for arithmetic operands.
///
/// Rules (in order):
/// - If either is F64, both become F64.
/// - Otherwise, both become the wider of the two int types (Bool counts
///   as 1-bit; treated as I64 when mixed with anything int-shaped to
///   match Python's `True + 1 == 2` semantics).
fn unify_numeric(lhs: TypedExpr, rhs: TypedExpr) -> Result<(TypedExpr, TypedExpr)> {
    if lhs.ty == Type::F64 || rhs.ty == Type::F64 {
        return Ok((coerce(lhs, Type::F64)?, coerce(rhs, Type::F64)?));
    }
    // Both int-shaped (I*) or Bool. Bool counts as I64 (Python-ish).
    let lty = if lhs.ty == Type::Bool { Type::I64 } else { lhs.ty };
    let rty = if rhs.ty == Type::Bool { Type::I64 } else { rhs.ty };
    let target = match (lty.int_width(), rty.int_width()) {
        (Some(lw), Some(rw)) => match lw.max(rw) {
            8 => Type::I8,
            16 => Type::I16,
            32 => Type::I32,
            _ => Type::I64,
        },
        _ => bail!(
            "unsupported_feature: cannot unify {} and {} for arithmetic",
            lhs.ty.name(),
            rhs.ty.name()
        ),
    };
    Ok((coerce(lhs, target)?, coerce(rhs, target)?))
}

/// For comparisons: same as numeric promotion (Bool → I64, then int+float → float).
fn unify_cmp_operands(lhs: TypedExpr, rhs: TypedExpr) -> Result<(TypedExpr, TypedExpr)> {
    unify_numeric(lhs, rhs)
}

/// Insert a coercion if the expression's type doesn't match the target.
/// Allowed coercions:
/// - between any two int widths (sext or trunc; signed semantics)
/// - Bool ↔ any int (zext / icmp ne 0)
/// - any int → F64 (sitofp)
/// - Bool → F64 (zext to int then sitofp)
/// - F64 → Bool (fcmp one … 0.0)
///
/// F64 → int is rejected (would be lossy; requires explicit cast that
/// we don't have a builtin for yet).
fn coerce(e: TypedExpr, target: Type) -> Result<TypedExpr> {
    if e.ty == target {
        return Ok(e);
    }
    // Special case: empty list literal can be re-tagged as any list type
    // (it has no elements to retype). Lets `lst: list[float] = []` work.
    if let (Type::List(_), Type::List(_)) = (e.ty, target) {
        if let Expr::ListLit { elements } = &e.expr {
            if elements.is_empty() {
                return Ok(TypedExpr::new(target, Expr::ListLit { elements: Vec::new() }));
            }
        }
    }
    // Same trick for empty dict literals: `d: dict[K, V] = {}` works.
    if let (Type::Dict(_), Type::Dict(_)) = (e.ty, target) {
        if let Expr::DictLit { entries } = &e.expr {
            if entries.is_empty() {
                return Ok(TypedExpr::new(target, Expr::DictLit { entries: Vec::new() }));
            }
        }
    }
    // And for empty sets — `s: set[T] = set()` lowers to an empty SetLit.
    if let (Type::Set(_), Type::Set(_)) = (e.ty, target) {
        if let Expr::SetLit { elements } = &e.expr {
            if elements.is_empty() {
                return Ok(TypedExpr::new(target, Expr::SetLit { elements: Vec::new() }));
            }
        }
    }
    // Class subtyping: B inheriting transitively from A can flow into a
    // slot annotated A. Lowered as Expr::Coerce, which codegen emits as a
    // struct-pointer bitcast (no-op at runtime; layout prefix matches).
    if let (Type::Class(b), Type::Class(a)) = (e.ty, target) {
        if is_subclass_of(b, a) {
            return Ok(TypedExpr::new(
                target,
                Expr::Coerce { inner: Box::new(e) },
            ));
        }
    }
    let allowed = match (e.ty, target) {
        // Float → int: lossy, rejected.
        (Type::F64, t) if t.is_int() => {
            bail!(
                "unsupported_feature: implicit float→{} conversion is not allowed (use an explicit cast — coming later)",
                t.name()
            )
        }
        // Int width changes (incl. Bool ↔ int): always allowed via sext/trunc/zext.
        (a, b) if (a.is_int() || a == Type::Bool) && (b.is_int() || b == Type::Bool) => true,
        // Numeric → F64: int / Bool → F64 is allowed.
        (a, Type::F64) if a.is_int() || a == Type::Bool => true,
        // F64 → Bool: allowed (fcmp).
        (Type::F64, Type::Bool) => true,
        _ => false,
    };
    if !allowed {
        bail!(
            "unsupported_feature: cannot coerce {} to {}",
            e.ty.name(),
            target.name()
        );
    }
    Ok(TypedExpr::new(target, Expr::Coerce { inner: Box::new(e) }))
}

/// Recognize and lower a small set of Python builtins. Returns
/// `Ok(Some(_))` if it's a builtin, `Ok(None)` if not, `Err(_)` on
/// invalid use. Builtins handled here:
/// - int(x)    — convert to I64. For F64 inputs uses fptosi (truncate
///                toward zero, matches CPython for finite values).
/// - float(x)  — convert to F64.
/// - bool(x)   — convert to Bool (truthy check).
/// - abs(x)    — absolute value, preserves type (int / float).
/// - min(a, b) — smaller of two same-type values.
/// - max(a, b) — larger of two same-type values.
fn lower_builtin_call(
    name: &str,
    args: &[ast::Expr],
    kwargs: &[ast::Keyword],
    scope: &Scope,
    signatures: &SignatureTable,
) -> Result<Option<TypedExpr>> {
    if !kwargs.is_empty() {
        return Ok(None);
    }
    match name {
        "int" => {
            if args.len() != 1 {
                bail!("unsupported_feature: int() takes exactly 1 argument");
            }
            let inner = lower_expr(&args[0], scope, signatures)?;
            if inner.ty == Type::F64 {
                // Explicit float→int via fptosi.
                Ok(Some(TypedExpr::new(
                    Type::I64,
                    Expr::Coerce { inner: Box::new(inner) },
                )))
            } else {
                Ok(Some(coerce(inner, Type::I64)?))
            }
        }
        "float" => {
            if args.len() != 1 {
                bail!("unsupported_feature: float() takes exactly 1 argument");
            }
            let inner = lower_expr(&args[0], scope, signatures)?;
            Ok(Some(coerce(inner, Type::F64)?))
        }
        "bool" => {
            if args.len() != 1 {
                bail!("unsupported_feature: bool() takes exactly 1 argument");
            }
            let inner = lower_expr(&args[0], scope, signatures)?;
            Ok(Some(coerce(inner, Type::Bool)?))
        }
        "abs" => {
            if args.len() != 1 {
                bail!("unsupported_feature: abs() takes exactly 1 argument");
            }
            let inner = lower_expr(&args[0], scope, signatures)?;
            let inner = if inner.ty == Type::Bool { coerce(inner, Type::I64)? } else { inner };
            let ty = inner.ty;
            if ty != Type::F64 && !ty.is_int() {
                bail!("unsupported_feature: abs() argument must be int or float");
            }
            // Build `(x < 0 and -x) or x` using BoolOp's short-circuit
            // value semantics:
            //   x >= 0: (False and ...) = 0, (0 or x) = x
            //   x <  0: (True  and -x ) = -x, (-x or x) = -x  (since -x is truthy)
            //   x == 0: gives 0 ∎
            let neg = TypedExpr::new(
                ty,
                Expr::UnaryOp {
                    op: UnaryOp::Neg,
                    operand: Box::new(inner.clone()),
                },
            );
            let zero = if ty == Type::F64 {
                TypedExpr::new(Type::F64, Expr::ConstF64(0.0))
            } else {
                TypedExpr::new(ty, Expr::ConstI64(0))
            };
            let is_neg = TypedExpr::new(
                Type::Bool,
                Expr::Cmp {
                    op: CmpOp::Lt,
                    lhs: Box::new(inner.clone()),
                    rhs: Box::new(zero),
                },
            );
            let and_branch = TypedExpr::new(
                ty,
                Expr::BoolOp {
                    op: BoolOp::And,
                    lhs: Box::new(coerce(is_neg, ty)?),
                    rhs: Box::new(neg),
                },
            );
            Ok(Some(TypedExpr::new(
                ty,
                Expr::BoolOp {
                    op: BoolOp::Or,
                    lhs: Box::new(and_branch),
                    rhs: Box::new(inner),
                },
            )))
        }
        // math module functions — recognized as builtins regardless of
        // whether `from math import …` was written. f64 → f64 each.
        "sqrt" | "sin" | "cos" | "tan" | "exp" | "log" | "floor" | "ceil" | "fabs" => {
            if args.len() != 1 {
                bail!("unsupported_feature: math.{}() takes exactly 1 argument", name);
            }
            let inner = lower_expr(&args[0], scope, signatures)?;
            let inner = coerce(inner, Type::F64)?;
            let intrinsic = match name {
                "sqrt" => "llvm.sqrt.f64",
                "sin" => "llvm.sin.f64",
                "cos" => "llvm.cos.f64",
                "exp" => "llvm.exp.f64",
                "log" => "llvm.log.f64",
                "floor" => "llvm.floor.f64",
                "ceil" => "llvm.ceil.f64",
                "fabs" => "llvm.fabs.f64",
                "tan" => "tan", // no LLVM intrinsic; libm
                _ => unreachable!(),
            };
            Ok(Some(TypedExpr::new(
                Type::F64,
                Expr::MathCall { intrinsic, arg: Box::new(inner) },
            )))
        }
        "len" => {
            if args.len() != 1 {
                bail!("unsupported_feature: len() takes exactly 1 argument");
            }
            let inner = lower_expr(&args[0], scope, signatures)?;
            match inner.ty {
                Type::List(_) => Ok(Some(TypedExpr::new(
                    Type::I64,
                    Expr::ListLen { list: Box::new(inner) },
                ))),
                Type::Tuple(id) => {
                    let n = id.with_elems(|elems| elems.len()) as i64;
                    Ok(Some(TypedExpr::new(Type::I64, Expr::ConstI64(n))))
                }
                Type::Str => Ok(Some(TypedExpr::new(
                    Type::I64,
                    Expr::StrLen { s: Box::new(inner) },
                ))),
                Type::Dict(_) => Ok(Some(TypedExpr::new(
                    Type::I64,
                    Expr::DictLen { dict: Box::new(inner) },
                ))),
                Type::Set(_) => Ok(Some(TypedExpr::new(
                    Type::I64,
                    Expr::SetLen { set: Box::new(inner) },
                ))),
                other => bail!(
                    "unsupported_feature: len() not supported on {} (only list/tuple/str/dict/set)",
                    other.name()
                ),
            }
        }
        "set" => {
            // Empty-set constructor `set()`. Reject `set(iter)` for now.
            if !args.is_empty() {
                bail!(
                    "unsupported_feature: set() with arguments is not yet supported (use a literal `{{a, b, c}}` instead)"
                );
            }
            let id = SetId::intern(Type::I64);
            Ok(Some(TypedExpr::new(
                Type::Set(id),
                Expr::SetLit { elements: Vec::new() },
            )))
        }
        "repr" => {
            if args.len() != 1 || !kwargs.is_empty() {
                bail!("unsupported_feature: repr() takes exactly 1 positional argument");
            }
            let inner = lower_expr(&args[0], scope, signatures)?;
            let cid = match inner.ty {
                Type::Class(c) => c,
                other => bail!(
                    "unsupported_feature: repr() is only supported on class instances in v0.35 (got {}); use f-strings for primitives",
                    other.name()
                ),
            };
            let (_owner, mangled) = resolve_method(cid, "__repr__", signatures)
                .ok_or_else(|| {
                    anyhow!(
                        "unsupported_feature: class `{}` has no `__repr__` method",
                        cid.name()
                    )
                })?;
            let sig = signatures.get(&mangled).unwrap();
            if sig.params.len() != 1 || sig.return_ty != Type::Str {
                bail!(
                    "unsupported_feature: `__repr__` on `{}` must take only `self` and return `str`",
                    cid.name()
                );
            }
            let arg = coerce(inner, sig.params[0].ty)?;
            Ok(Some(TypedExpr::new(
                Type::Str,
                Expr::Call { callee: mangled, args: vec![arg] },
            )))
        }
        "min" | "max" => {
            if args.len() != 2 {
                bail!(
                    "unsupported_feature: {}() with {} arguments — only 2-arg form supported",
                    name,
                    args.len()
                );
            }
            let a = lower_expr(&args[0], scope, signatures)?;
            let b = lower_expr(&args[1], scope, signatures)?;
            let (a, b) = unify_numeric(a, b)?;
            let ty = a.ty;
            let cmp_op = if name == "min" { CmpOp::Le } else { CmpOp::Ge };
            let cmp = TypedExpr::new(
                Type::Bool,
                Expr::Cmp {
                    op: cmp_op,
                    lhs: Box::new(a.clone()),
                    rhs: Box::new(b.clone()),
                },
            );
            let and_branch = TypedExpr::new(
                ty,
                Expr::BoolOp {
                    op: BoolOp::And,
                    lhs: Box::new(coerce(cmp, ty)?),
                    rhs: Box::new(a),
                },
            );
            Ok(Some(TypedExpr::new(
                ty,
                Expr::BoolOp {
                    op: BoolOp::Or,
                    lhs: Box::new(and_branch),
                    rhs: Box::new(b),
                },
            )))
        }
        _ => Ok(None),
    }
}

fn resolve_call_args(
    callee: &str,
    sig: &FunctionSig,
    pos_args: &[ast::Expr],
    kw_args: &[ast::Keyword],
    scope: &Scope,
    signatures: &SignatureTable,
) -> Result<Vec<TypedExpr>> {
    let n = sig.params.len();
    if pos_args.len() > n {
        bail!(
            "unsupported_feature: function `{}` takes {} positional arguments but {} were supplied",
            callee,
            n,
            pos_args.len()
        );
    }
    let mut filled: Vec<Option<TypedExpr>> = vec![None; n];
    for (i, a) in pos_args.iter().enumerate() {
        let raw = lower_expr(a, scope, signatures)?;
        filled[i] = Some(coerce(raw, sig.params[i].ty)?);
    }
    for kw in kw_args {
        let name = kw.arg.as_ref().ok_or_else(|| {
            anyhow!(
                "unsupported_feature: `**kwargs` unpacking at call site is not supported (in call to `{}`)",
                callee
            )
        })?;
        let idx = sig
            .params
            .iter()
            .position(|p| p.name == name.as_str())
            .ok_or_else(|| {
                anyhow!(
                    "unsupported_feature: function `{}` has no parameter named `{}`",
                    callee,
                    name
                )
            })?;
        if filled[idx].is_some() {
            bail!(
                "unsupported_feature: multiple values for argument `{}` in call to `{}`",
                name,
                callee
            );
        }
        let raw = lower_expr(&kw.value, scope, signatures)?;
        filled[idx] = Some(coerce(raw, sig.params[idx].ty)?);
    }
    let mut out = Vec::with_capacity(n);
    for (i, slot) in filled.into_iter().enumerate() {
        match slot {
            Some(e) => out.push(e),
            None => match &sig.defaults[i] {
                Some(default) => out.push(default.clone()),
                None => bail!(
                    "unsupported_feature: missing required argument `{}` in call to `{}`",
                    sig.params[i].name,
                    callee
                ),
            },
        }
    }
    Ok(out)
}

/// Parse and lower a `range(...)` call as the iterable of a for-loop.
/// Returns (start, stop, step) all as I64 TypedExprs. Defaults: start=0, step=1.
fn parse_and_lower_range(
    iter: &ast::Expr,
    scope: &Scope,
    signatures: &SignatureTable,
) -> Result<(TypedExpr, TypedExpr, TypedExpr)> {
    let call = match iter {
        ast::Expr::Call(c) => c,
        _ => bail!(
            "unsupported_feature: for-loop iterables other than range(...) are not supported in v0.14"
        ),
    };
    if !matches!(call.func.as_ref(), ast::Expr::Name(n) if n.id.as_str() == "range") {
        bail!(
            "unsupported_feature: for-loop iterables other than range(...) are not supported in v0.14"
        );
    }
    if !call.keywords.is_empty() {
        bail!("unsupported_feature: range() does not accept keyword arguments");
    }
    let zero = TypedExpr::new(Type::I64, Expr::ConstI64(0));
    let one = TypedExpr::new(Type::I64, Expr::ConstI64(1));
    let lower_to_i64 = |e: &ast::Expr| -> Result<TypedExpr> {
        let raw = lower_expr(e, scope, signatures)?;
        coerce(raw, Type::I64)
    };
    match call.args.as_slice() {
        [stop] => Ok((zero, lower_to_i64(stop)?, one)),
        [start, stop] => Ok((lower_to_i64(start)?, lower_to_i64(stop)?, one)),
        [start, stop, step] => Ok((
            lower_to_i64(start)?,
            lower_to_i64(stop)?,
            lower_to_i64(step)?,
        )),
        _ => bail!("unsupported_feature: range() takes 1, 2, or 3 arguments"),
    }
}

fn convert_cmp_op(op: &ast::CmpOp) -> Result<CmpOp> {
    Ok(match op {
        ast::CmpOp::Lt => CmpOp::Lt,
        ast::CmpOp::LtE => CmpOp::Le,
        ast::CmpOp::Gt => CmpOp::Gt,
        ast::CmpOp::GtE => CmpOp::Ge,
        ast::CmpOp::Eq => CmpOp::Eq,
        ast::CmpOp::NotEq => CmpOp::Ne,
        ast::CmpOp::Is | ast::CmpOp::IsNot => bail!(
            "unsupported_feature: `is` / `is not` are not supported (only allowed against `None` in later slices)"
        ),
        // `In`/`NotIn` are handled out-of-band in lower_expr (Compare),
        // since they need RHS-type-driven dispatch (DictHas vs ListIn etc.).
        // Reaching here is an internal bug.
        ast::CmpOp::In | ast::CmpOp::NotIn => unreachable!(
            "in/not in should have been handled before reaching convert_cmp_op"
        ),
    })
}

fn stmt_kind_name(s: &ast::Stmt) -> &'static str {
    match s {
        ast::Stmt::FunctionDef(_) => "FunctionDef",
        ast::Stmt::AsyncFunctionDef(_) => "AsyncFunctionDef",
        ast::Stmt::ClassDef(_) => "ClassDef",
        ast::Stmt::Return(_) => "Return",
        ast::Stmt::Delete(_) => "Delete",
        ast::Stmt::Assign(_) => "Assign",
        ast::Stmt::AugAssign(_) => "AugAssign",
        ast::Stmt::AnnAssign(_) => "AnnAssign",
        ast::Stmt::For(_) => "For",
        ast::Stmt::AsyncFor(_) => "AsyncFor",
        ast::Stmt::While(_) => "While",
        ast::Stmt::If(_) => "If",
        ast::Stmt::With(_) => "With",
        ast::Stmt::AsyncWith(_) => "AsyncWith",
        ast::Stmt::Match(_) => "Match",
        ast::Stmt::Raise(_) => "Raise",
        ast::Stmt::Try(_) => "Try",
        ast::Stmt::TryStar(_) => "TryStar",
        ast::Stmt::Assert(_) => "Assert",
        ast::Stmt::Import(_) => "Import",
        ast::Stmt::ImportFrom(_) => "ImportFrom",
        ast::Stmt::Global(_) => "Global",
        ast::Stmt::Nonlocal(_) => "Nonlocal",
        ast::Stmt::Expr(_) => "Expr",
        ast::Stmt::Pass(_) => "Pass",
        ast::Stmt::Break(_) => "Break",
        ast::Stmt::Continue(_) => "Continue",
        ast::Stmt::TypeAlias(_) => "TypeAlias",
    }
}

fn expr_kind_name(e: &ast::Expr) -> &'static str {
    match e {
        ast::Expr::BoolOp(_) => "BoolOp",
        ast::Expr::NamedExpr(_) => "NamedExpr",
        ast::Expr::BinOp(_) => "BinOp",
        ast::Expr::UnaryOp(_) => "UnaryOp",
        ast::Expr::Lambda(_) => "Lambda",
        ast::Expr::IfExp(_) => "IfExp",
        ast::Expr::Dict(_) => "Dict",
        ast::Expr::Set(_) => "Set",
        ast::Expr::ListComp(_) => "ListComp",
        ast::Expr::SetComp(_) => "SetComp",
        ast::Expr::DictComp(_) => "DictComp",
        ast::Expr::GeneratorExp(_) => "GeneratorExp",
        ast::Expr::Await(_) => "Await",
        ast::Expr::Yield(_) => "Yield",
        ast::Expr::YieldFrom(_) => "YieldFrom",
        ast::Expr::Compare(_) => "Compare",
        ast::Expr::Call(_) => "Call",
        ast::Expr::FormattedValue(_) => "FormattedValue",
        ast::Expr::JoinedStr(_) => "JoinedStr",
        ast::Expr::Constant(_) => "Constant",
        ast::Expr::Attribute(_) => "Attribute",
        ast::Expr::Subscript(_) => "Subscript",
        ast::Expr::Starred(_) => "Starred",
        ast::Expr::Name(_) => "Name",
        ast::Expr::List(_) => "List",
        ast::Expr::Tuple(_) => "Tuple",
        ast::Expr::Slice(_) => "Slice",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;
    use std::path::PathBuf;

    fn parse(src: &str) -> Module {
        parser::parse(src, &PathBuf::from("t.py")).unwrap()
    }

    #[test]
    fn lowers_no_param_main() {
        let p = lower(&parse("def main() -> int:\n    return 42\n"), &PathBuf::from("t.py")).unwrap();
        assert_eq!(p.main().params.len(), 0);
        assert_eq!(p.main().return_ty, Type::I64);
    }

    #[test]
    fn lowers_two_param_main() {
        let p = lower(&parse(
            "def main(a: int, b: int) -> int:\n    return a + b\n",
        ), &PathBuf::from("t.py"))
        .unwrap();
        assert_eq!(p.main().params.len(), 2);
        assert_eq!(p.main().params[0].ty, Type::I64);
    }

    #[test]
    fn allows_float_local_inside_int_main() {
        // float values flow through the program but main return is int.
        // The result of true-div is F64; we check it via comparison.
        let _ = lower(&parse(
            "def helper() -> float:\n    return 1.5 + 2.5\n\ndef main() -> int:\n    x: float = helper()\n    if x > 0.0:\n        return 1\n    else:\n        return 0\n",
        ), &PathBuf::from("t.py"))
        .unwrap();
    }

    #[test]
    fn rejects_implicit_float_to_int_on_return() {
        let m = parse("def main() -> int:\n    return 1.5\n");
        let err = lower(&m, &PathBuf::from("t.py")).unwrap_err();
        assert!(format!("{}", err).contains("float→int"));
    }

    #[test]
    fn accepts_float_main_return() {
        let _ = lower(&parse("def main() -> float:\n    return 1.0\n"), &PathBuf::from("t.py")).unwrap();
    }

    #[test]
    fn rejects_bool_main_return() {
        let m = parse("def main() -> bool:\n    return True\n");
        let err = lower(&m, &PathBuf::from("t.py")).unwrap_err();
        assert!(format!("{}", err).contains("`main` must return"));
    }

    #[test]
    fn lowers_simple_if_else() {
        let p = lower(&parse(
            "def main(a: int) -> int:\n    if a < 0:\n        return -a\n    else:\n        return a\n",
        ), &PathBuf::from("t.py"))
        .unwrap();
        match &p.main().body[0] {
            Stmt::If { cond, .. } => assert_eq!(cond.ty, Type::Bool),
            _ => panic!("expected If"),
        }
    }

    #[test]
    fn rejects_break_outside_loop() {
        let m = parse("def main() -> int:\n    break\n    return 0\n");
        let err = lower(&m, &PathBuf::from("t.py")).unwrap_err();
        assert!(format!("{}", err).contains("`break` outside"));
    }
}
