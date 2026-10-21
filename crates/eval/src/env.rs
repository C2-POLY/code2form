//! Environments. Ordinary names resolve lexically (through `parent`);
//! `$`-names resolve dynamically, through the frames that led to this
//! evaluation (`caller`), which is how `$fn` set by a caller reaches the
//! primitives it instantiates.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use c2f_lang::ast::{FunctionDef, Instance, ModuleDef};

use crate::value::Value;

pub type Env<'p> = Rc<Frame<'p>>;

/// Frames with more variables than this also index them by name.
const INDEX_FROM: usize = 16;

#[derive(Default)]
pub struct Frame<'p> {
    vars: RefCell<Vec<(&'p str, Value<'p>)>>,
    /// Position of each variable in `vars`, once there are many.
    index: RefCell<Option<HashMap<&'p str, usize>>>,
    functions: RefCell<HashMap<&'p str, &'p FunctionDef>>,
    modules: RefCell<HashMap<&'p str, &'p ModuleDef>>,
    /// Libraries brought in by `use`, on a file's root frame, in search
    /// order.
    libraries: RefCell<Vec<Rc<Library<'p>>>>,
    pub parent: Option<Env<'p>>,
    pub caller: Option<Env<'p>>,
    /// Set on the frame of a user module call, for `children()`.
    pub call: Option<ModuleCall<'p>>,
    /// On a file's top-level frame: which file (index into the program).
    pub unit: Option<usize>,
    /// On a used library's top-level frame: its definitions, shared by all
    /// its instances instead of being copied into each.
    exports: Option<Rc<Library<'p>>>,
}

/// The definitions a used file exports. Calling one from another file
/// instantiates the library afresh (see `Interp::library_instance`).
pub struct Library<'p> {
    pub file: usize,
    pub functions: HashMap<&'p str, &'p FunctionDef>,
    pub modules: HashMap<&'p str, &'p ModuleDef>,
}

/// Where a function or module definition was found.
pub enum Found<'p, T> {
    /// In this file, defined in that frame.
    Local(T, Env<'p>),
    /// In the used library with this file index: a cross-file call.
    Library(T, usize),
}

/// What `children()` needs from the module instantiation it is inside.
pub struct ModuleCall<'p> {
    /// The child instances, in order.
    pub children: Rc<[&'p Instance]>,
    /// Where the instantiation was written: children evaluate there.
    pub env: Env<'p>,
}

impl<'p> Frame<'p> {
    /// A frame for a nested block: lexical and dynamic parent are the same.
    pub fn child(parent: &Env<'p>) -> Env<'p> {
        Rc::new(Frame { parent: Some(parent.clone()), caller: Some(parent.clone()), ..Frame::default() })
    }

    /// A frame for a call: lexically inside `definition`, dynamically
    /// inside `caller`.
    pub fn call(definition: &Env<'p>, caller: &Env<'p>, call: Option<ModuleCall<'p>>) -> Env<'p> {
        Rc::new(Frame { parent: Some(definition.clone()), caller: Some(caller.clone()), call, ..Frame::default() })
    }

    pub fn root(parent: Option<Env<'p>>) -> Env<'p> {
        Rc::new(Frame { parent, ..Frame::default() })
    }

    /// The top level of the main file.
    pub fn main(builtins: Env<'p>) -> Env<'p> {
        Rc::new(Frame { parent: Some(builtins), unit: Some(0), ..Frame::default() })
    }

    /// The top level of used library `unit` for one call into it:
    /// lexically under the builtins only, dynamically inside the caller.
    pub fn library(builtins: Env<'p>, caller: Env<'p>, unit: usize, exports: Rc<Library<'p>>) -> Env<'p> {
        Rc::new(Frame {
            parent: Some(builtins),
            caller: Some(caller),
            unit: Some(unit),
            exports: Some(exports),
            ..Frame::default()
        })
    }

    fn position(&self, name: &str) -> Option<usize> {
        match &*self.index.borrow() {
            Some(index) => index.get(name).copied(),
            None => self.vars.borrow().iter().position(|(n, _)| *n == name),
        }
    }

    /// Defines or overwrites a variable in this frame.
    pub fn set(&self, name: &'p str, value: Value<'p>) {
        if let Some(i) = self.position(name) {
            self.vars.borrow_mut()[i].1 = value;
            return;
        }
        let mut vars = self.vars.borrow_mut();
        vars.push((name, value));
        let mut index = self.index.borrow_mut();
        match &mut *index {
            Some(index) => {
                index.insert(name, vars.len() - 1);
            }
            None if vars.len() > INDEX_FROM => {
                *index = Some(vars.iter().enumerate().map(|(i, (n, _))| (*n, i)).collect());
            }
            None => {}
        }
    }

    pub fn get_local(&self, name: &str) -> Option<Value<'p>> {
        let i = self.position(name)?;
        Some(self.vars.borrow()[i].1.clone())
    }

    /// This frame's own variables, in definition order.
    pub fn locals(&self) -> Vec<(&'p str, Value<'p>)> {
        self.vars.borrow().clone()
    }

    pub fn has_special_vars(&self) -> bool {
        self.vars.borrow().iter().any(|(n, _)| n.starts_with('$'))
    }

    /// Defines a function; a later definition of the name replaces it.
    pub fn define_function(&self, f: &'p FunctionDef) {
        self.functions.borrow_mut().insert(&f.name.name, f);
    }

    pub fn define_module(&self, m: &'p ModuleDef) {
        self.modules.borrow_mut().insert(&m.name.name, m);
    }

    pub fn add_library(&self, library: Rc<Library<'p>>) {
        self.libraries.borrow_mut().push(library);
    }

    fn local_function(&self, name: &str) -> Option<&'p FunctionDef> {
        let own = self.functions.borrow().get(name).copied();
        own.or_else(|| self.exports.as_ref()?.functions.get(name).copied())
    }

    fn local_module(&self, name: &str) -> Option<&'p ModuleDef> {
        let own = self.modules.borrow().get(name).copied();
        own.or_else(|| self.exports.as_ref()?.modules.get(name).copied())
    }
}

/// Looks a variable up: lexically for plain names, dynamically for `$`.
pub fn lookup<'p>(env: &Env<'p>, name: &str) -> Option<Value<'p>> {
    let dynamic = name.starts_with('$');
    let mut frame = Some(env);
    while let Some(f) = frame {
        if let Some(v) = f.get_local(name) {
            return Some(v);
        }
        frame = if dynamic { f.caller.as_ref().or(f.parent.as_ref()) } else { f.parent.as_ref() };
    }
    None
}

/// A named function: in scope, or else in a used library.
pub fn lookup_function<'p>(env: &Env<'p>, name: &str) -> Option<Found<'p, &'p FunctionDef>> {
    lookup_definition(env, name, Frame::local_function, |lib| lib.functions.get(name).copied())
}

pub fn lookup_module<'p>(env: &Env<'p>, name: &str) -> Option<Found<'p, &'p ModuleDef>> {
    lookup_definition(env, name, Frame::local_module, |lib| lib.modules.get(name).copied())
}

fn lookup_definition<'p, T>(
    env: &Env<'p>,
    name: &str,
    local: fn(&Frame<'p>, &str) -> Option<T>,
    exported: impl Fn(&Library<'p>) -> Option<T>,
) -> Option<Found<'p, T>> {
    let mut frame = Some(env);
    while let Some(f) = frame {
        if let Some(def) = local(f, name) {
            return Some(Found::Local(def, f.clone()));
        }
        // `use` is not transitive: only a library's own definitions count.
        for library in f.libraries.borrow().iter() {
            if let Some(def) = exported(library) {
                return Some(Found::Library(def, library.file));
            }
        }
        frame = f.parent.as_ref();
    }
    None
}

/// The file whose code is running in `env`.
pub fn unit_of(env: &Env) -> usize {
    let mut frame = Some(env);
    while let Some(f) = frame {
        if let Some(unit) = f.unit {
            return unit;
        }
        frame = f.parent.as_ref();
    }
    0
}

/// The nearest enclosing user module call, for `children()`.
pub fn enclosing_call<'a, 'p>(env: &'a Env<'p>) -> Option<&'a ModuleCall<'p>> {
    let mut frame = Some(env);
    while let Some(f) = frame {
        if let Some(call) = &f.call {
            return Some(call);
        }
        frame = f.parent.as_ref();
    }
    None
}
