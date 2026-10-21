//! How the reference implementation groups the statements of one scope (a
//! file, a module body, a block of module children).
//!
//! Functions and modules are hoisted. Assignments run in order, but a name
//! assigned twice keeps the position of its first assignment and the value
//! of its last. Module instances run after every assignment. Braces do not
//! open a scope, and `*`-disabled instances do not exist.

use std::collections::HashMap;

use crate::ast::*;

#[derive(Default)]
pub struct Scope<'a> {
    pub functions: Vec<&'a FunctionDef>,
    pub modules: Vec<&'a ModuleDef>,
    /// The statement of the assignment that won, for its span.
    pub assignments: Vec<(&'a Stmt, &'a Assignment)>,
    pub instances: Vec<&'a Instance>,
    /// Position of each assigned name in `assignments`.
    assigned: HashMap<&'a str, usize>,
}

impl<'a> Scope<'a> {
    pub fn collect(stmts: &'a [Stmt]) -> Self {
        let mut scope = Scope::default();
        scope.add(stmts);
        scope
    }

    fn add(&mut self, stmts: &'a [Stmt]) {
        for stmt in stmts {
            match &stmt.kind {
                StmtKind::Empty | StmtKind::Include(_) | StmtKind::Use(_) => {}
                StmtKind::Block(inner) => self.add(inner),
                StmtKind::Function(f) => self.functions.push(f),
                StmtKind::Module(m) => self.modules.push(m),
                StmtKind::Assign(a) => match self.assigned.get(a.name.name.as_str()) {
                    Some(&i) => self.assignments[i] = (stmt, a),
                    None => {
                        self.assigned.insert(&a.name.name, self.assignments.len());
                        self.assignments.push((stmt, a));
                    }
                },
                StmtKind::Instance(i) if i.modifiers.disabled => {}
                StmtKind::Instance(i) => self.instances.push(i),
            }
        }
    }

    pub fn len(&self) -> usize {
        self.functions.len() + self.modules.len() + self.assignments.len() + self.instances.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
