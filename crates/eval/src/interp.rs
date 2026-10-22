//! The tree-walking interpreter.

use std::rc::Rc;

use c2f_lang::ast::*;
use c2f_lang::dump;
use c2f_lang::scope::Scope;

use crate::env::{self, Env, Found, Frame, Library, ModuleCall};
use crate::node::{Node, NodeKind};
use crate::program::{Program, SourceFile};
use crate::value::{Closure, ListBuilder, Range, Value, undefined_operation};
use crate::{functions, modules};

/// Nested non-tail function calls beyond this abort with "Recursion
/// detected". The reference implementation's limit is its C++ stack (about
/// 11000 for a simple `1 + f(n - 1)` on macOS).
const MAX_CALL_DEPTH: usize = 10_000;
/// Self-calls in tail position run as a loop, at most this many times per
/// call, as in the reference.
const MAX_TAIL_ITERATIONS: usize = 1_000_000;
/// Nested instantiations (modules, transforms, `if`, ...) beyond this abort
/// a user module call. The reference's stack gives out at about
/// 7200 to 7900.
const MAX_INSTANTIATION_DEPTH: usize = 7_500;
/// Body runs of a C-style `for` comprehension before it aborts.
const MAX_LOOP_ITERATIONS: usize = 1_000_000;
/// Recursion stops ("Recursion detected") once this much of the evaluator's
/// stack is in use, whatever the shape of the program; the reference has
/// the same kind of guard on its C++ stack.
const STACK_BUDGET: usize = crate::EVAL_STACK / 10 * 8;
/// Ranges with this many elements are not iterated by a comprehension...
const COMPREHENSION_RANGE_LIMIT: u64 = 1_000_000;
/// ...nor by a module-level `for`, nor selected by `children()`.
const MODULE_RANGE_LIMIT: u64 = 10_000;

/// An error that stops evaluation; the reference then exports nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct Abort(pub String);

pub type EResult<T> = Result<T, Abort>;

/// An evaluated argument: `name = value` or a positional value.
pub type ArgValue<'p> = (Option<&'p str>, Value<'p>);

pub struct Interp<'p> {
    program: &'p Program,
    /// Directory of the main file, against which data files resolve.
    pub base_dir: std::path::PathBuf,
    pub echo: Vec<String>,
    pub warnings: Vec<String>,
    pub pauses: Vec<crate::Pause>,
    /// Names of the user modules being instantiated, innermost last.
    pub module_stack: Vec<&'p str>,
    call_depth: usize,
    instantiation_depth: usize,
    /// Address near the bottom of the evaluator's stack.
    stack_base: usize,
    /// The first `!` node in pre-order, which replaces the whole result.
    pub root: Option<Node>,
    /// A `!` instantiation is being evaluated (or has produced the root).
    root_claimed: bool,
    /// Set by `children(<number>)`: the node returned is the child's own,
    /// so the modifiers written on the `children` call do not apply.
    passthrough: bool,
    builtins: Env<'p>,
    library_tables: Vec<Option<Rc<Library<'p>>>>,
    /// Each used file's top-level scope, collected once.
    library_scopes: Vec<Option<Rc<Scope<'p>>>>,
    /// Call names of callees that are not identifiers, printed once.
    call_names: std::collections::HashMap<usize, Rc<str>>,
    /// The one generator behind `rands()`: seeded calls reseed it, unseeded
    /// calls continue its stream.
    pub rng: crate::math::Mt19937,
}

/// What a function body evaluates to in tail position: a value, or a call
/// of the function to itself, to run as the next loop iteration.
enum Tail<'p> {
    Value(Value<'p>),
    SelfCall(Vec<ArgValue<'p>>, Env<'p>),
}

enum Callable<'p> {
    User {
        params: &'p [Param],
        body: &'p Expr,
        /// Where the function was defined: its lexical scope.
        def_env: Env<'p>,
        /// For a call into a used library: the library instance, which sits
        /// between the caller and the call on the `$` chain.
        library: Option<Env<'p>>,
    },
    Builtin(&'p str),
}

impl<'p> Interp<'p> {
    pub fn new(program: &'p Program) -> Self {
        let builtins = Frame::root(None);
        for (name, value) in [
            ("PI", Value::Number(std::f64::consts::PI)),
            ("$fn", Value::Number(0.0)),
            ("$fa", Value::Number(12.0)),
            ("$fs", Value::Number(2.0)),
            ("$t", Value::Number(0.0)),
            // The reference evaluates in preview mode when exporting CSG.
            ("$preview", Value::Bool(true)),
            // The command-line camera, not the GUI's.
            ("$vpt", Value::vector(vec![Value::Number(0.0); 3])),
            ("$vpr", Value::vector(vec![Value::Number(0.0); 3])),
            ("$vpd", Value::Number(500.0)),
            ("$vpf", Value::Number(22.5)),
        ] {
            builtins.set(name, value);
        }
        // The reference seeds with time + pid; only the seeded stream is
        // reproducible.
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let seed = (now as u32).wrapping_add(std::process::id());
        Interp {
            program,
            base_dir: program.files[0].path.parent().map(|p| p.to_path_buf()).unwrap_or_default(),
            echo: Vec::new(),
            warnings: Vec::new(),
            pauses: Vec::new(),
            module_stack: Vec::new(),
            call_depth: 0,
            instantiation_depth: 0,
            stack_base: stack_address(),
            root: None,
            root_claimed: false,
            passthrough: false,
            builtins,
            library_tables: vec![None; program.files.len()],
            library_scopes: vec![None; program.files.len()],
            call_names: std::collections::HashMap::new(),
            rng: crate::math::Mt19937::new(seed),
        }
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }

    /// A data file named in code running in `env`: where to read it, and
    /// how the CSG names it. Paths resolve against the directory of the
    /// file holding the code (a used library's own), and are named
    /// relative to the main file's directory.
    pub fn data_path(&self, env: &Env<'p>, file: &str) -> (std::path::PathBuf, String) {
        let unit = env::unit_of(env);
        if unit == 0 {
            return (self.base_dir.join(file), file.to_string());
        }
        let dir = self.program.files[unit].path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let full = dir.join(file);
        let base = if self.base_dir.as_os_str().is_empty() { std::path::Path::new(".") } else { &self.base_dir };
        let main_dir = base.canonicalize().unwrap_or(base.to_path_buf());
        // The relative name joins its parts with `/` on every system, as
        // the goldens (measured on macOS) and .scad files write it.
        let shown = full
            .canonicalize()
            .ok()
            .and_then(|f| {
                let relative = f.strip_prefix(&main_dir).ok()?;
                Some(relative.iter().map(|part| part.to_string_lossy()).collect::<Vec<_>>().join("/"))
            })
            .unwrap_or_else(|| full.display().to_string());
        (full, shown)
    }

    /// Fails with "Recursion detected" when the stack is nearly used up.
    fn check_stack(&self, what: &str, name: &str) -> EResult<()> {
        if self.stack_base.saturating_sub(stack_address()) > STACK_BUDGET {
            return Err(Abort(format!("Recursion detected calling {what} '{name}'")));
        }
        Ok(())
    }

    /// Evaluates the main file and returns its top-level nodes.
    pub fn run(&mut self) -> EResult<Vec<Node>> {
        self.stack_base = stack_address();
        let main = &self.program.files[0];
        let env = Frame::main(self.builtins.clone());
        self.attach_libraries(main, &env);
        self.eval_scope(&main.ast.stmts, &env)
    }

    fn attach_libraries(&mut self, file: &'p SourceFile, env: &Env<'p>) {
        for &library in &file.uses {
            let table = self.library_table(library);
            env.add_library(table);
        }
    }

    /// What a used file exports: its top-level function and module
    /// definitions (not its variables).
    fn library_table(&mut self, index: usize) -> Rc<Library<'p>> {
        if let Some(table) = &self.library_tables[index] {
            return table.clone();
        }
        let scope = Scope::collect(&self.program.files[index].ast.stmts);
        let table = Rc::new(Library {
            file: index,
            functions: scope.functions.iter().map(|f| (f.name.name.as_str(), *f)).collect(),
            modules: scope.modules.iter().map(|m| (m.name.name.as_str(), *m)).collect(),
        });
        self.library_tables[index] = Some(table.clone());
        table
    }

    /// A fresh instance of a used library for one call into it from another
    /// file. As in the reference, its top-level assignments run again on
    /// every such call, seeing the caller's `$` variables; its own `$`
    /// assignments then override the caller's for the whole call.
    fn library_instance(&mut self, index: usize, caller: &Env<'p>, name: &str) -> EResult<Env<'p>> {
        // Library inits can call into each other forever; count them as
        // calls.
        self.check_stack("function", name)?;
        if self.call_depth >= MAX_CALL_DEPTH {
            return Err(Abort(format!("Recursion detected calling function '{name}'")));
        }
        self.call_depth += 1;
        let result = self.library_instance_inner(index, caller);
        self.call_depth -= 1;
        result
    }

    fn library_instance_inner(&mut self, index: usize, caller: &Env<'p>) -> EResult<Env<'p>> {
        let file = &self.program.files[index];
        let table = self.library_table(index);
        let env = Frame::library(self.builtins.clone(), dynamic_parent(caller), index, table);
        self.attach_libraries(file, &env);
        let scope = match &self.library_scopes[index] {
            Some(scope) => scope.clone(),
            None => {
                let scope = Rc::new(Scope::collect(&file.ast.stmts));
                self.library_scopes[index] = Some(scope.clone());
                scope
            }
        };
        for (_, a) in &scope.assignments {
            let value = self.eval(&a.value, &env)?;
            env.set(&a.name.name, value);
        }
        Ok(env)
    }

    // ---- scopes and instances ----

    /// Runs one scope in `frame`: definitions, then assignments in order,
    /// then instances.
    pub fn eval_scope(&mut self, stmts: &'p [Stmt], frame: &Env<'p>) -> EResult<Vec<Node>> {
        let scope = Scope::collect(stmts);
        for f in &scope.functions {
            frame.define_function(f);
        }
        for m in &scope.modules {
            frame.define_module(m);
        }
        for (_, a) in &scope.assignments {
            let value = self.eval(&a.value, frame)?;
            frame.set(&a.name.name, value);
        }
        let mut nodes = Vec::new();
        for inst in &scope.instances {
            if let Some(node) = self.instantiate(inst, frame)? {
                nodes.push(node);
            }
        }
        Ok(nodes)
    }

    /// Children of a builtin module, in their own frame.
    pub fn eval_children(&mut self, stmts: &'p [Stmt], env: &Env<'p>) -> EResult<Vec<Node>> {
        self.eval_scope(stmts, &Frame::child(env))
    }

    fn instantiate(&mut self, inst: &'p Instance, env: &Env<'p>) -> EResult<Option<Node>> {
        // The root is the first `!` in pre-order: claim it before the
        // subtree, whose own `!`s then lose.
        let claims_root = inst.modifiers.root && !self.root_claimed;
        if inst.modifiers.root && !claims_root {
            self.warn("More than one Root Modifier (!)");
        }
        if claims_root {
            self.root_claimed = true;
        }
        self.instantiation_depth += 1;
        let node = self.instantiate_inner(inst, env);
        self.instantiation_depth -= 1;
        let passthrough = std::mem::take(&mut self.passthrough);
        let node = match node? {
            Some(node) if !passthrough => node,
            other => {
                if claims_root {
                    self.root_claimed = false;
                }
                return Ok(other);
            }
        };
        if claims_root {
            // The root is shown as it is, without its own `#` or `%`.
            self.root = Some(node.clone());
        }
        let mut node = node;
        node.highlight |= inst.modifiers.highlight;
        node.background |= inst.modifiers.background;
        Ok(Some(node))
    }

    fn instantiate_inner(&mut self, inst: &'p Instance, env: &Env<'p>) -> EResult<Option<Node>> {
        match &inst.kind {
            // A false `if` without `else` produces no node at all.
            InstanceKind::If { cond, then, otherwise } => {
                let branch = if self.eval(cond, env)?.truthy() { Some(then) } else { otherwise.as_ref() };
                match branch {
                    Some(stmts) => Ok(Some(Node::group(self.eval_children(stmts, env)?))),
                    None => Ok(None),
                }
            }
            InstanceKind::Call { name, args, children } => self.call_module(&name.name, args, children, env),
        }
    }

    fn call_module(
        &mut self,
        name: &'p str,
        args: &'p [Arg],
        children: &'p [Stmt],
        env: &Env<'p>,
    ) -> EResult<Option<Node>> {
        // `children` and `intersection_for` are plain names a user module
        // may take over; `for`, `let`, `echo` and `assert` are keywords.
        let overridden = matches!(name, "children" | "intersection_for" | "child" | "assign")
            && env::lookup_module(env, name).is_some();
        match name {
            _ if overridden => {}
            "for" => return self.module_for(args, children, env, false),
            "intersection_for" => return self.module_for(args, children, env, true),
            // Deprecated: like `let`, but every value is computed before any
            // is bound.
            "assign" => {
                let values = self.eval_args(args, env)?;
                let frame = Frame::child(env);
                for (name, value) in values {
                    if let Some(name) = name {
                        frame.set(name, value);
                    }
                }
                return Ok(Some(Node::group(self.eval_children(children, &frame)?)));
            }
            // Deprecated `children` that selects the first child by default.
            "child" => return self.module_children(args, env, true),
            "let" => {
                let frame = Frame::child(env);
                self.bind_sequential(args, &frame)?;
                return Ok(Some(Node::group(self.eval_children(children, &frame)?)));
            }
            "echo" => {
                let values = self.eval_args(args, env)?;
                self.echo_values(&values);
                return self.optional_group(children, env);
            }
            "assert" => {
                self.assert(args, env)?;
                return self.optional_group(children, env);
            }
            "children" => return self.module_children(args, env, false),
            _ => {}
        }
        match env::lookup_module(env, name) {
            Some(Found::Local(def, def_env)) => {
                let dynamic = dynamic_parent(env);
                return self.call_user_module(def, &def_env, &dynamic, args, children, env).map(Some);
            }
            Some(Found::Library(def, index)) => {
                self.check_instantiation_depth(name)?;
                let library = self.library_instance(index, env, name)?;
                return self.call_user_module(def, &library, &library, args, children, env).map(Some);
            }
            None => {}
        }
        let values = self.eval_args(args, env)?;
        match modules::instantiate(self, name, values, children, env)? {
            Some(node) => Ok(node),
            None => {
                self.warn(format!("Ignoring unknown module '{name}'"));
                Ok(None)
            }
        }
    }

    fn check_instantiation_depth(&self, name: &str) -> EResult<()> {
        if self.instantiation_depth > MAX_INSTANTIATION_DEPTH {
            return Err(Abort(format!("Recursion detected calling module '{name}'")));
        }
        self.check_stack("module", name)
    }

    /// `echo` and `assert` make a group only when their children produce
    /// nodes (a block of assignments alone still runs).
    fn optional_group(&mut self, children: &'p [Stmt], env: &Env<'p>) -> EResult<Option<Node>> {
        let nodes = self.eval_children(children, env)?;
        Ok((!nodes.is_empty()).then(|| Node::group(nodes)))
    }

    /// A user module call: lexically inside `def_env`, dynamically inside
    /// `dynamic`.
    fn call_user_module(
        &mut self,
        def: &'p ModuleDef,
        def_env: &Env<'p>,
        dynamic: &Env<'p>,
        args: &'p [Arg],
        children: &'p [Stmt],
        env: &Env<'p>,
    ) -> EResult<Node> {
        let name = def.name.name.as_str();
        self.check_instantiation_depth(name)?;
        let instances: Rc<[&'p Instance]> = Scope::collect(children).instances.into();
        let child_count = instances.len();
        let call = ModuleCall { children: instances, env: self.children_env(children, env)? };
        let frame = Frame::call(def_env, dynamic, Some(call));
        self.bind_defaults(&def.params, &frame, def_env)?;
        let values = self.eval_args(args, env)?;
        self.bind_args(&def.params, values, &frame);
        frame.set("$children", Value::Number(child_count as f64));
        self.module_stack.push(name);
        frame.set("$parent_modules", Value::Number(self.module_stack.len() as f64));
        let result = self.eval_scope(std::slice::from_ref(&*def.body), &frame);
        self.module_stack.pop();
        Ok(Node::group(result?))
    }

    /// The scope of a module call's child block: its assignments run once,
    /// at the instantiation, in the caller's scope (before the arguments).
    fn children_env(&mut self, children: &'p [Stmt], env: &Env<'p>) -> EResult<Env<'p>> {
        let frame = Frame::child(env);
        for (_, a) in &Scope::collect(children).assignments {
            let value = self.eval(&a.value, &frame)?;
            frame.set(&a.name.name, value);
        }
        Ok(frame)
    }

    fn module_for(
        &mut self,
        args: &'p [Arg],
        children: &'p [Stmt],
        env: &Env<'p>,
        intersection: bool,
    ) -> EResult<Option<Node>> {
        let mut nodes = Vec::new();
        if !args.is_empty() {
            self.for_bindings(args, env, MODULE_RANGE_LIMIT, false, &mut |interp, frame| {
                nodes.extend(interp.eval_scope(children, frame)?);
                Ok(())
            })?;
        }
        let kind = if intersection { NodeKind::Intersection } else { NodeKind::Group };
        Ok(Some(Node::new(kind, nodes)))
    }

    /// Nested loops over `name = iterable` bindings; later bindings see the
    /// variables of earlier ones, and an unnamed binding still iterates. A
    /// range of `range_limit` elements or more is not iterated. With
    /// `shared`, each level reuses one frame for all its iterations (list
    /// comprehensions do, so closures made in them see the final values);
    /// otherwise every iteration gets a fresh frame.
    fn for_bindings(
        &mut self,
        bindings: &'p [Arg],
        env: &Env<'p>,
        range_limit: u64,
        shared: bool,
        body: &mut dyn FnMut(&mut Self, &Env<'p>) -> EResult<()>,
    ) -> EResult<()> {
        let Some((first, rest)) = bindings.split_first() else {
            return body(self, &Frame::child(env));
        };
        let iterable = self.eval(&first.value, env)?;
        let name = first.name.as_ref().map(|n| n.name.as_str());
        let shared_frame = Frame::child(env);
        for item in self.iterate(&iterable, range_limit) {
            let frame = if shared { shared_frame.clone() } else { Frame::child(env) };
            if let Some(name) = name {
                frame.set(name, item);
            }
            if rest.is_empty() {
                body(self, &frame)?;
            } else {
                self.for_bindings(rest, &frame, range_limit, shared, body)?;
            }
        }
        Ok(())
    }

    /// What `for` iterates over: range values, vector elements, string
    /// characters, nothing for `undef`, or the value itself.
    fn iterate(&mut self, value: &Value<'p>, range_limit: u64) -> Vec<Value<'p>> {
        match value {
            Value::Range(r) => {
                let len = r.len();
                if len >= range_limit {
                    self.warn(format!("Bad range parameter in for statement: too many elements ({len})"));
                    return Vec::new();
                }
                r.values().map(Value::Number).collect()
            }
            Value::Vector(items) => items.to_vec(),
            Value::Str(s) => s.chars().map(|c| Value::str(c.encode_utf8(&mut [0; 4]))).collect(),
            Value::Undef => Vec::new(),
            other => vec![other.clone()],
        }
    }

    fn module_children(&mut self, args: &'p [Arg], env: &Env<'p>, first_by_default: bool) -> EResult<Option<Node>> {
        let Some(call) = env::enclosing_call(env) else { return Ok(None) };
        let (instances, call_env) = (call.children.clone(), call.env.clone());
        let values = self.eval_args(args, env)?;
        let count = instances.len();

        // Which children, before any is evaluated. Only the first argument
        // counts, whatever its name.
        let selection = match values.into_iter().next().map(|(_, v)| v) {
            // The deprecated child() takes only a number, else the first.
            Some(Value::Number(i)) if first_by_default => Selection::One(children_index(i)),
            _ if first_by_default => Selection::One(0),
            None => Selection::Many((0..count as i64).collect()),
            Some(Value::Number(i)) => Selection::One(children_index(i)),
            Some(Value::Vector(items)) => Selection::Many(
                items
                    .iter()
                    .filter_map(|item| match item {
                        Value::Number(i) => Some(children_index(*i)),
                        other => {
                            self.warn(format!(
                                "Bad parameter type ({}) for children, only accept: empty, number, vector, range.",
                                other.to_display()
                            ));
                            None
                        }
                    })
                    .collect(),
            ),
            Some(Value::Range(r)) => {
                let len = r.len();
                if len >= MODULE_RANGE_LIMIT {
                    self.warn(format!("Bad range parameter for children: too many elements ({len})"));
                    return Ok(None);
                }
                Selection::Many(r.values().map(children_index).collect())
            }
            Some(other) => {
                self.warn(format!(
                    "Bad parameter type ({}) for children, only accept: empty, number, vector, range",
                    other.repr()
                ));
                return Ok(None);
            }
        };
        let frame = Frame::call(&call_env, &dynamic_parent(env), None);
        let indices = match &selection {
            Selection::One(index) => vec![*index],
            Selection::Many(indices) => indices.clone(),
        };
        let mut nodes = Vec::new();
        for index in indices {
            if index < 0 {
                self.warn(format!("Negative children index ({index}) not allowed"));
            } else if index as usize >= count {
                self.warn(format!("Children index ({index}) out of bounds ({count} children)"));
            } else if let Some(node) = self.instantiate(instances[index as usize], &frame)? {
                nodes.push(node);
            }
        }
        // A number selects the child itself; everything else wraps.
        match selection {
            Selection::One(_) => {
                self.passthrough = true;
                Ok(nodes.pop())
            }
            Selection::Many(_) => Ok(Some(Node::group(nodes))),
        }
    }

    // ---- arguments ----

    pub fn eval_args(&mut self, args: &'p [Arg], env: &Env<'p>) -> EResult<Vec<ArgValue<'p>>> {
        args.iter().map(|a| Ok((a.name.as_ref().map(|n| n.name.as_str()), self.eval(&a.value, env)?))).collect()
    }

    /// Sets every parameter of a call to its default (or undef), in order,
    /// before any argument is looked at, as the reference does. A default
    /// sees the definition's scope lexically, and the call (with the
    /// defaults before it) then the caller for `$` variables.
    fn bind_defaults(&mut self, params: &'p [Param], frame: &Env<'p>, def_env: &Env<'p>) -> EResult<()> {
        let defaults_env = Frame::call(def_env, frame, None);
        for param in params {
            let value = match &param.default {
                Some(default) => self.eval(default, &defaults_env)?,
                None => Value::Undef,
            };
            frame.set(&param.name.name, value);
        }
        Ok(())
    }

    /// Binds call arguments over the defaults, in order: the n-th
    /// positional argument sets the n-th parameter even if it was named
    /// before. A named argument that is no parameter is still bound (with a
    /// warning).
    fn bind_args(&mut self, params: &'p [Param], args: Vec<ArgValue<'p>>, frame: &Env<'p>) {
        let mut positional = 0;
        for (name, value) in args {
            match name {
                None => {
                    match params.get(positional) {
                        Some(param) => frame.set(&param.name.name, value),
                        None => self.warn("Too many unnamed arguments supplied"),
                    }
                    positional += 1;
                }
                Some(name) => {
                    if !name.starts_with('$') && !params.iter().any(|p| p.name.name == name) {
                        self.warn(format!("variable {name} not specified as parameter"));
                    }
                    frame.set(name, value);
                }
            }
        }
    }

    /// `let (a = 1, b = a + 1)`: each binding sees the previous ones. A
    /// name given twice keeps its first value (the second is still
    /// evaluated, for its side effects).
    fn bind_sequential(&mut self, args: &'p [Arg], frame: &Env<'p>) -> EResult<()> {
        let mut seen: Vec<&str> = Vec::new();
        for arg in args {
            let value = self.eval(&arg.value, frame)?;
            let Some(name) = &arg.name else { continue };
            if seen.contains(&name.name.as_str()) {
                self.warn(format!("Ignoring duplicate variable assignment {}", name.name));
                continue;
            }
            seen.push(&name.name);
            frame.set(&name.name, value);
        }
        Ok(())
    }

    /// The assignments of a C-style `for` header, in order, each seeing the
    /// ones before it; a name given twice keeps its first assignment.
    fn assign_once(&mut self, args: &'p [Arg], frame: &Env<'p>) -> EResult<()> {
        self.bind_sequential(args, frame)
    }

    pub fn echo_values(&mut self, values: &[ArgValue<'p>]) {
        let parts: Vec<String> = values
            .iter()
            .map(|(name, value)| match name {
                Some(name) => format!("{name} = {}", value.repr()),
                None => value.repr(),
            })
            .collect();
        self.echo.push(format!("ECHO: {}", parts.join(", ")));
    }

    fn assert(&mut self, args: &'p [Arg], env: &Env<'p>) -> EResult<()> {
        let values = self.eval_args(args, env)?;
        let pick = |name: &str, position: usize| {
            values
                .iter()
                .find(|(n, _)| *n == Some(name))
                .or_else(|| values.iter().filter(|(n, _)| n.is_none()).nth(position))
                .map(|(_, v)| v.clone())
        };
        let condition = pick("condition", 0).unwrap_or(Value::Undef);
        if condition.truthy() {
            return Ok(());
        }
        let source = args
            .iter()
            .find(|a| a.name.as_ref().is_some_and(|n| n.name == "condition"))
            .or_else(|| args.iter().find(|a| a.name.is_none()))
            .map_or(String::new(), |a| dump::expr(&a.value));
        let message = match pick("message", 1) {
            Some(message) => format!(": {}", message.repr()),
            None => String::new(),
        };
        Err(Abort(format!("Assertion '{source}' failed{message}")))
    }

    // ---- expressions ----

    pub fn eval(&mut self, e: &'p Expr, env: &Env<'p>) -> EResult<Value<'p>> {
        Ok(match &e.kind {
            ExprKind::Undef => Value::Undef,
            ExprKind::Bool(b) => Value::Bool(*b),
            ExprKind::Number(n) => Value::Number(*n),
            ExprKind::Str(s) => Value::str(s),
            ExprKind::Ident(name) => match lookup_variable(env, name) {
                Some(value) => value,
                None => {
                    self.warn(format!("Ignoring unknown variable '{name}'"));
                    Value::Undef
                }
            },
            ExprKind::Paren(inner) => self.eval(inner, env)?,
            ExprKind::Vector(elements) => {
                let mut items = ListBuilder::default();
                for element in elements {
                    self.element(element, env, &mut items)?;
                }
                items.finish()
            }
            // Evaluated begin, end, then step; a non-number part (silently)
            // makes the whole range undef and skips the parts after it.
            ExprKind::Range { start, step, end } => {
                let Some(begin) = self.eval(start, env)?.as_number() else { return Ok(Value::Undef) };
                let Some(end) = self.eval(end, env)?.as_number() else { return Ok(Value::Undef) };
                let step = match step {
                    Some(step) => match self.eval(step, env)?.as_number() {
                        Some(step) => Some(step),
                        None => return Ok(Value::Undef),
                    },
                    None => None,
                };
                let (range, swapped) = Range::new(begin, step, end);
                if swapped {
                    self.warn("DEPRECATED: Using ranges of the form [begin:end] with begin value greater than the end value is deprecated");
                }
                Value::Range(range)
            }
            ExprKind::Unary { op, operand } => {
                let value = self.eval(operand, env)?;
                match op {
                    UnaryOp::Not => Value::Bool(!value.truthy()),
                    UnaryOp::Plus => value,
                    UnaryOp::Neg => value.neg().unwrap_or_else(|| {
                        self.warn(format!("undefined operation (-{})", value.type_name()));
                        Value::Undef
                    }),
                }
            }
            ExprKind::Binary { op, lhs, rhs } => self.binary(*op, lhs, rhs, env)?,
            ExprKind::Ternary { cond, then, otherwise } => {
                if self.eval(cond, env)?.truthy() {
                    self.eval(then, env)?
                } else {
                    self.eval(otherwise, env)?
                }
            }
            ExprKind::Call { callee, args } => self.call_expr(callee, args, env)?,
            ExprKind::Index { target, index } => {
                let target = self.eval(target, env)?;
                let index = self.eval(index, env)?;
                target.index(&index)
            }
            ExprKind::Member { target, name } => {
                let target = self.eval(target, env)?;
                match (&target, name.name.as_str()) {
                    (Value::Vector(_), "x") => target.index(&Value::Number(0.0)),
                    (Value::Vector(_), "y") => target.index(&Value::Number(1.0)),
                    (Value::Vector(_), "z") => target.index(&Value::Number(2.0)),
                    (Value::Range(r), "begin") => Value::Number(r.begin),
                    (Value::Range(r), "step") => Value::Number(r.step),
                    (Value::Range(r), "end") => Value::Number(r.end),
                    _ => Value::Undef,
                }
            }
            ExprKind::FunctionLit { params, body } => {
                Value::Function(Rc::new(Closure { params, body, env: env.clone() }))
            }
            // Outside a function body nothing is a self-call: no call has an
            // empty name.
            ExprKind::Let { .. } | ExprKind::Assert { .. } | ExprKind::Echo { .. } => match self.tail(e, env, "")? {
                Tail::Value(v) => v,
                Tail::SelfCall(..) => unreachable!("no call is named \"\""),
            },
        })
    }

    fn binary(&mut self, op: BinaryOp, lhs: &'p Expr, rhs: &'p Expr, env: &Env<'p>) -> EResult<Value<'p>> {
        let a = self.eval(lhs, env)?;
        match op {
            BinaryOp::And => return Ok(Value::Bool(a.truthy() && self.eval(rhs, env)?.truthy())),
            BinaryOp::Or => return Ok(Value::Bool(a.truthy() || self.eval(rhs, env)?.truthy())),
            _ => {}
        }
        let b = self.eval(rhs, env)?;
        let result = match op {
            BinaryOp::Eq => Some(Value::Bool(a.equals(&b))),
            BinaryOp::Ne => Some(Value::Bool(!a.equals(&b))),
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                // Numbers compare directly so that NaN is never ordered.
                match (&a, &b) {
                    (Value::Number(x), Value::Number(y)) => Some(Value::Bool(match op {
                        BinaryOp::Lt => x < y,
                        BinaryOp::Le => x <= y,
                        BinaryOp::Gt => x > y,
                        _ => x >= y,
                    })),
                    _ => a.compare(&b).map(|ord| {
                        Value::Bool(match op {
                            BinaryOp::Lt => ord.is_lt(),
                            BinaryOp::Le => ord.is_le(),
                            BinaryOp::Gt => ord.is_gt(),
                            _ => ord.is_ge(),
                        })
                    }),
                }
            }
            BinaryOp::Add => a.add(&b),
            BinaryOp::Sub => a.sub(&b),
            BinaryOp::Mul => match a.mul(&b) {
                Ok(v) => Some(v),
                Err(message) => {
                    self.warn(message);
                    return Ok(Value::Undef);
                }
            },
            BinaryOp::Div => a.div(&b),
            BinaryOp::Mod => a.rem(&b),
            BinaryOp::Pow => a.pow(&b),
            BinaryOp::And | BinaryOp::Or => unreachable!(),
        };
        Ok(result.unwrap_or_else(|| {
            self.warn(undefined_operation(op.symbol(), &a, &b));
            Value::Undef
        }))
    }

    /// Evaluates a function body in tail position. As in the reference, a
    /// call whose name is the name the current function was called by is a
    /// self-call, whatever that name refers to: it comes back with its
    /// arguments so the caller can loop instead of recursing.
    fn tail(&mut self, e: &'p Expr, env: &Env<'p>, current: &str) -> EResult<Tail<'p>> {
        match &e.kind {
            ExprKind::Paren(inner) => self.tail(inner, env, current),
            ExprKind::Ternary { cond, then, otherwise } => {
                let branch = if self.eval(cond, env)?.truthy() { then } else { otherwise };
                self.tail(branch, env, current)
            }
            ExprKind::Let { args, body } => {
                let frame = Frame::child(env);
                self.bind_sequential(args, &frame)?;
                self.tail(body, &frame, current)
            }
            ExprKind::Echo { args, body } => {
                let values = self.eval_args(args, env)?;
                self.echo_values(&values);
                match body {
                    Some(body) => self.tail(body, env, current),
                    None => Ok(Tail::Value(Value::Undef)),
                }
            }
            ExprKind::Assert { args, body } => {
                self.assert(args, env)?;
                match body {
                    Some(body) => self.tail(body, env, current),
                    None => Ok(Tail::Value(Value::Undef)),
                }
            }
            ExprKind::Call { callee, args } if self.is_call_named(callee, current) => {
                Ok(Tail::SelfCall(self.eval_args(args, env)?, env.clone()))
            }
            _ => self.eval(e, env).map(Tail::Value),
        }
    }

    /// How the reference names a call: the identifier, or the parenthesized
    /// callee expression (`([g][0])`), printed once per callee.
    fn call_name(&mut self, callee: &'p Expr) -> Rc<str> {
        if let ExprKind::Ident(name) = &unparen(callee).kind {
            return name.as_str().into();
        }
        let key = callee as *const Expr as usize;
        self.call_names.entry(key).or_insert_with(|| format!("({})", dump::expr(callee)).into()).clone()
    }

    fn is_call_named(&mut self, callee: &'p Expr, name: &str) -> bool {
        match &unparen(callee).kind {
            ExprKind::Ident(ident) => ident == name,
            // Only a printed callee can be named like `(...)`.
            _ => name.starts_with('(') && &*self.call_name(callee) == name,
        }
    }

    /// `callee(args)`.
    fn call_expr(&mut self, callee: &'p Expr, args: &'p [Arg], env: &Env<'p>) -> EResult<Value<'p>> {
        let Some(callable) = self.resolve(callee, env)? else {
            return Ok(Value::Undef);
        };
        match callable {
            Callable::Builtin(name) => {
                let values = self.eval_args(args, env)?;
                functions::call(self, name, values, env)
            }
            user => {
                let name = self.call_name(callee);
                self.call(user, name.to_string(), args, env)
            }
        }
    }

    /// Finds what `callee(...)` calls. A variable holding a function wins
    /// over a function definition, which wins over a builtin. Resolving into
    /// a used library instantiates it, before the arguments are evaluated.
    fn resolve(&mut self, callee: &'p Expr, env: &Env<'p>) -> EResult<Option<Callable<'p>>> {
        let target = unparen(callee);
        if let ExprKind::Ident(name) = &target.kind {
            if name.starts_with('$') {
                self.warn("Can't call function on dynamically scoped variable");
                return Ok(None);
            }
            if let Some(Value::Function(closure)) = env::lookup(env, name) {
                return Ok(Some(closure_callable(&closure)));
            }
            match env::lookup_function(env, name) {
                Some(Found::Local(def, def_env)) => {
                    return Ok(Some(Callable::User { params: &def.params, body: &def.body, def_env, library: None }));
                }
                Some(Found::Library(def, index)) => {
                    let library = self.library_instance(index, env, name)?;
                    return Ok(Some(Callable::User {
                        params: &def.params,
                        body: &def.body,
                        def_env: library.clone(),
                        library: Some(library),
                    }));
                }
                None => {}
            }
            if functions::exists(name) {
                return Ok(Some(Callable::Builtin(name)));
            }
            self.warn(format!("Ignoring unknown function '{name}'"));
            return Ok(None);
        }
        match self.eval(target, env)? {
            Value::Function(closure) => Ok(Some(closure_callable(&closure))),
            other => {
                self.warn(format!("Can't call function on {}", other.type_name()));
                Ok(None)
            }
        }
    }

    /// Runs a user function called by the name `name`, looping through its
    /// tail self-calls.
    fn call(&mut self, callable: Callable<'p>, name: String, args: &'p [Arg], caller: &Env<'p>) -> EResult<Value<'p>> {
        let Callable::User { params, body, def_env, library } = callable else {
            unreachable!("builtins are called directly")
        };
        if self.call_depth >= MAX_CALL_DEPTH {
            return Err(Abort(format!("Recursion detected calling function '{name}'")));
        }
        self.check_stack("function", &name)?;
        self.call_depth += 1;
        let dynamic = library.unwrap_or_else(|| dynamic_parent(caller));
        let result = self.call_loop(params, body, &def_env, &dynamic, &name, args, caller);
        self.call_depth -= 1;
        result
    }

    /// The body runs in one frame for the whole call: a tail self-call
    /// rebinds the parameters in place, as the reference does, so closures
    /// made in any iteration see the last values, undeclared named
    /// arguments stay, and `$` variables set by `let`s on the way to a
    /// self-call are kept for the iterations after it.
    #[allow(clippy::too_many_arguments)]
    fn call_loop(
        &mut self,
        params: &'p [Param],
        body: &'p Expr,
        def_env: &Env<'p>,
        dynamic: &Env<'p>,
        name: &str,
        args: &'p [Arg],
        caller: &Env<'p>,
    ) -> EResult<Value<'p>> {
        let frame = Frame::call(def_env, dynamic, None);
        self.bind_defaults(params, &frame, def_env)?;
        let values = self.eval_args(args, caller)?;
        self.bind_args(params, values, &frame);
        let mut iterations = 0;
        loop {
            match self.tail(body, &frame, name)? {
                Tail::Value(v) => return Ok(v),
                Tail::SelfCall(next, call_env) => {
                    iterations += 1;
                    if iterations > MAX_TAIL_ITERATIONS {
                        return Err(Abort(format!("Recursion detected calling function '{name}'")));
                    }
                    for (var, value) in specials_between(&call_env, &frame) {
                        frame.set(var, value);
                    }
                    self.bind_defaults(params, &frame, def_env)?;
                    self.bind_args(params, next, &frame);
                }
            }
        }
    }

    // ---- list comprehensions ----

    fn element(&mut self, element: &'p Element, env: &Env<'p>, out: &mut ListBuilder<'p>) -> EResult<()> {
        match element {
            Element::Expr(e) => out.push(self.eval(e, env)?),
            Element::Comp(c) => self.comp(c, env, out)?,
        }
        Ok(())
    }

    fn comp(&mut self, c: &'p Comp, env: &Env<'p>, out: &mut ListBuilder<'p>) -> EResult<()> {
        match &c.kind {
            // Unlike the module-level `for ()`, `[for () x]` yields x once.
            CompKind::For { bindings, body } => {
                self.for_bindings(bindings, env, COMPREHENSION_RANGE_LIMIT, true, &mut |interp, frame| {
                    interp.element(body, frame, out)
                })
            }
            // One frame for the whole loop; the update assignments run in
            // order, each seeing the ones before it.
            CompKind::ForC { init, cond, update, body } => {
                let frame = Frame::child(env);
                self.assign_once(init, &frame)?;
                let mut iterations = 0;
                while self.eval(cond, &frame)?.truthy() {
                    self.element(body, &frame, out)?;
                    // The 1,000,001st body still runs, then the loop aborts.
                    iterations += 1;
                    if iterations > MAX_LOOP_ITERATIONS {
                        return Err(Abort("For loop counter exceeded limit".into()));
                    }
                    self.assign_once(update, &frame)?;
                }
                Ok(())
            }
            CompKind::Each(body) => {
                let mut inner = ListBuilder::default();
                self.element(body, env, &mut inner)?;
                let inner = inner.finish();
                for value in inner.as_vector().unwrap_or(&[]) {
                    match value {
                        Value::Vector(items) => out.splice(items),
                        Value::Range(_) | Value::Str(_) => out.extend(self.iterate(value, COMPREHENSION_RANGE_LIMIT)),
                        Value::Undef => {}
                        other => out.push(other.clone()),
                    }
                }
                Ok(())
            }
            CompKind::If { cond, then, otherwise } => {
                if self.eval(cond, env)?.truthy() {
                    self.element(then, env, out)
                } else if let Some(otherwise) = otherwise {
                    self.element(otherwise, env, out)
                } else {
                    Ok(())
                }
            }
            CompKind::Let { bindings, body } => {
                let frame = Frame::child(env);
                self.bind_sequential(bindings, &frame)?;
                self.element(body, &frame, out)
            }
        }
    }
}

/// What `children(...)` selects: one child by number (returned as is), or
/// a list (wrapped in a group).
enum Selection {
    One(i64),
    Many(Vec<i64>),
}

/// An index for `children()`: C's `(int)` cast, so NaN, infinities and
/// values out of `int` range become `INT_MIN`.
fn children_index(i: f64) -> i64 {
    if (-2_147_483_648.0..2_147_483_648.0).contains(&i) { i.trunc() as i64 } else { i32::MIN as i64 }
}

fn closure_callable<'p>(closure: &Rc<Closure<'p>>) -> Callable<'p> {
    Callable::User { params: closure.params, body: closure.body, def_env: closure.env.clone(), library: None }
}

fn unparen(e: &Expr) -> &Expr {
    match &e.kind {
        ExprKind::Paren(inner) => unparen(inner),
        _ => e,
    }
}

fn stack_address() -> usize {
    let marker = 0u8;
    std::ptr::addr_of!(marker) as usize
}

/// `$` variables of the `let` frames from `inner` out to (not including)
/// the function frame `outer`, outermost first.
fn specials_between<'p>(inner: &Env<'p>, outer: &Env<'p>) -> Vec<(&'p str, Value<'p>)> {
    let mut chain = Vec::new();
    let mut current = inner.clone();
    while !Rc::ptr_eq(&current, outer) {
        chain.push(current.clone());
        match current.caller.clone() {
            Some(caller) => current = caller,
            None => break,
        }
    }
    chain
        .iter()
        .rev()
        .flat_map(|frame| frame.locals().into_iter().filter(|(name, _)| name.starts_with('$')))
        .collect()
}

/// `$children` belongs to the module whose body is being evaluated, so it
/// resolves lexically: children blocks see their own context's count.
fn lookup_variable<'p>(env: &Env<'p>, name: &str) -> Option<Value<'p>> {
    if name != "$children" {
        return env::lookup(env, name);
    }
    let mut frame = Some(env);
    while let Some(f) = frame {
        if let Some(v) = f.get_local(name) {
            return Some(v);
        }
        frame = f.parent.as_ref();
    }
    None
}

/// The nearest frame on the dynamic chain that defines a `$` variable.
/// Skipping the others keeps `$` lookups short.
fn dynamic_parent<'p>(env: &Env<'p>) -> Env<'p> {
    let mut current = env;
    while !current.has_special_vars() {
        match &current.caller {
            Some(caller) => current = caller,
            None => break,
        }
    }
    current.clone()
}
