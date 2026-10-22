//! The move, turn and scale handles (C2F-39): writing an object's
//! `translate`, `rotate` or `scale`, as an absolute value or by a delta.
//!
//! An object's transforms are the *chain* of wrappers written around it,
//! each the single child of the next: in
//! `translate([5, 0, 0]) rotate([0, 0, 45]) cube(2);` the chain is the
//! `translate` then the `rotate`, and the object is the `cube`. The
//! selection may be the object or any statement of its chain: both name
//! the same object. The chain runs through `translate`, `rotate`, `scale`,
//! `mirror`, `multmatrix`, `resize` and `color`, and stops at anything
//! else: a wrapper that also has other children is never part of it, and
//! an extrusion is the object (the transforms inside it are 2D).
//!
//! An edit changes the innermost wrapper of its kind in the chain when it
//! has one, else adds one where the usual order (`translate`, then
//! `rotate`, then `scale`, outermost first) wants it, so that moving a
//! turned object moves it in the world and turning it turns it in place.

use std::ops::Range;

use c2f_lang::ast::Stmt;

use super::props::bound_arg;
use super::{
    Doc, Edit, Loc, Splice, call, call_name, computed, literal, number, placed, scalar, single, vector, wrapped_child,
};

/// A transform the handles write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transform {
    Translate,
    Rotate,
    Scale,
}

impl Transform {
    fn name(self) -> &'static str {
        match self {
            Transform::Translate => "translate",
            Transform::Rotate => "rotate",
            Transform::Scale => "scale",
        }
    }

    fn identity(self) -> f64 {
        if self == Transform::Scale { 1.0 } else { 0.0 }
    }

    fn rank(self) -> u8 {
        match self {
            Transform::Translate => 0,
            Transform::Rotate => 1,
            Transform::Scale => 2,
        }
    }
}

/// Where a wrapper goes in the usual order, outermost first; `None` for
/// `color`, which may sit anywhere.
fn rank(name: &str) -> Option<u8> {
    match name {
        "translate" => Some(0),
        "rotate" => Some(1),
        "scale" | "mirror" | "multmatrix" | "resize" => Some(2),
        _ => None,
    }
}

const CHAIN: [&str; 7] = ["translate", "rotate", "scale", "mirror", "multmatrix", "resize", "color"];

/// An object and the wrappers of its chain, outermost first.
struct Chain<'a> {
    wrappers: Vec<&'a Stmt>,
    object: &'a Stmt,
}

fn chain<'a>(path: &[Loc<'a>]) -> Chain<'a> {
    let mut stmt = placed(path).last().expect("located").stmt;
    let mut wrappers = Vec::new();
    while let Some(child) = wrapped_child(stmt).filter(|_| call_name(stmt).is_some_and(|n| CHAIN.contains(&n))) {
        wrappers.push(stmt);
        stmt = child;
    }
    // The placement may climb through an extrusion the chain stops at:
    // the object is then the extrusion, not what the selection named. A
    // transform with several children, selected whole, is the object and
    // its own innermost wrapper.
    if call_name(stmt).is_some_and(|n| CHAIN.contains(&n)) {
        wrappers.push(stmt);
    }
    Chain { wrappers, object: stmt }
}

impl<'a> Chain<'a> {
    /// The innermost wrapper of kind `kind`.
    fn find(&self, kind: Transform) -> Option<&'a Stmt> {
        self.wrappers.iter().rev().find(|w| call_name(w) == Some(kind.name())).copied()
    }

    /// Where a new wrapper of kind `kind` goes: before the outermost
    /// wrapper that comes after it in the usual order, else right around
    /// the object; after the modifiers of the statement it goes before.
    fn insertion(&self, doc: &Doc, kind: Transform) -> usize {
        let before = self.wrappers.iter().find(|w| call_name(w).and_then(rank).is_some_and(|r| r > kind.rank()));
        doc.after_modifiers(before.copied().unwrap_or(self.object))
    }
}

/// A wrapper's value as written: its three components and where it is
/// written, or where to write it when the call has no argument.
struct Current {
    value: [f64; 3],
    at: Range<usize>,
    /// Written as a single number (`rotate(30)`, `scale(2)`), a style a
    /// rewrite keeps when it can.
    scalar: bool,
}

fn current(doc: &Doc, wrapper: &Stmt, kind: Transform) -> Result<Current, String> {
    let (name, args, _) = call(wrapper).expect("a wrapper is a call");
    let param = if kind == Transform::Rotate { "a" } else { "v" };
    if kind == Transform::Rotate && bound_arg(doc, name, args, "v").is_some() {
        return Err("Cette rotation tourne autour d'un axe (v) : modifiez-la dans le code.".into());
    }
    let Some(arg) = bound_arg(doc, name, args, param) else {
        if !args.is_empty() {
            return Err("Cette transformation n'a pas de valeur à modifier.".into());
        }
        let (_, close) = doc.parens(name).ok_or(super::STALE)?;
        return Ok(Current { value: [kind.identity(); 3], at: close..close, scalar: false });
    };
    let at = arg.value.span.range();
    if let Some(n) = scalar(&arg.value) {
        return match kind {
            Transform::Rotate => Ok(Current { value: [0.0, 0.0, n], at, scalar: true }),
            Transform::Scale => Ok(Current { value: [n; 3], at, scalar: true }),
            Transform::Translate => Err(computed(doc.src, &arg.value)),
        };
    }
    match literal(&arg.value) {
        Some(super::Value::Vector(v)) if (1..=3).contains(&v.len()) => {
            let mut value = [kind.identity(); 3];
            value[..v.len()].copy_from_slice(&v);
            Ok(Current { value, at, scalar: false })
        }
        _ => Err(computed(doc.src, &arg.value)),
    }
}

/// The code of a value, keeping a single number where the source wrote
/// one and the value still is one.
fn value_code(kind: Transform, value: [f64; 3], scalar: bool) -> String {
    match kind {
        Transform::Rotate if scalar && value[0] == 0.0 && value[1] == 0.0 => number(value[2]),
        Transform::Scale if scalar && value[0] == value[1] && value[1] == value[2] => number(value[0]),
        _ => vector(&value),
    }
}

fn rewrite(
    doc: &Doc,
    wrapper: &Stmt,
    kind: Transform,
    value: [f64; 3],
    selected: &Range<usize>,
) -> Result<Edit, String> {
    let cur = current(doc, wrapper, kind)?;
    let splice = Splice::new(cur.at.clone(), value_code(kind, value, cur.scalar));
    single(doc.src, splice, selected, true, false)
}

fn wrap(doc: &Doc, at: usize, kind: Transform, value: [f64; 3], selected: &Range<usize>) -> Result<Edit, String> {
    let splice = Splice::insert(at, format!("{}({}) ", kind.name(), vector(&value)));
    single(doc.src, splice, selected, true, false)
}

/// Sets the object's `kind` transform to `value`: rewrites the innermost
/// wrapper of that kind in its chain, or adds one. A wrapper whose value
/// the source computes is refused, as is a `rotate` around an axis `v`.
/// The selection stays on the statement at `range`.
pub fn set_transform(src: &str, range: Range<usize>, kind: Transform, value: [f64; 3]) -> Result<Edit, String> {
    let doc = Doc::parse(src)?;
    let path = doc.locate_instance(&range)?;
    let chain = chain(&path);
    match chain.find(kind) {
        Some(wrapper) => rewrite(&doc, wrapper, kind, value, &range),
        None => wrap(&doc, chain.insertion(&doc, kind), kind, value, &range),
    }
}

/// The object's `kind` transform as its innermost wrapper of that kind
/// writes it, the identity when it has none, for the handles to start
/// from; refused as [`set_transform`] refuses.
pub fn transform_of(src: &str, range: Range<usize>, kind: Transform) -> Result<[f64; 3], String> {
    let doc = Doc::parse(src)?;
    let path = doc.locate_instance(&range)?;
    match chain(&path).find(kind) {
        Some(wrapper) => Ok(current(&doc, wrapper, kind)?.value),
        None => Ok([kind.identity(); 3]),
    }
}

/// Moves the object by `delta`: its `translate` gets the sum, or a new
/// `translate(delta)` goes around it.
pub fn translate_by(src: &str, range: Range<usize>, delta: [f64; 3]) -> Result<Edit, String> {
    by(src, range, Transform::Translate, delta, |a, b| a + b)
}

/// Scales the object by `factors`: its `scale` gets the product, or a new
/// `scale(factors)` goes around it.
pub fn scale_by(src: &str, range: Range<usize>, factors: [f64; 3]) -> Result<Edit, String> {
    by(src, range, Transform::Scale, factors, |a, b| a * b)
}

fn by(
    src: &str,
    range: Range<usize>,
    kind: Transform,
    delta: [f64; 3],
    combine: fn(f64, f64) -> f64,
) -> Result<Edit, String> {
    let doc = Doc::parse(src)?;
    let path = doc.locate_instance(&range)?;
    let chain = chain(&path);
    match chain.find(kind) {
        Some(wrapper) => {
            let v = current(&doc, wrapper, kind)?.value;
            let value = [combine(v[0], delta[0]), combine(v[1], delta[1]), combine(v[2], delta[2])];
            rewrite(&doc, wrapper, kind, value, &range)
        }
        None => wrap(&doc, chain.insertion(&doc, kind), kind, delta, &range),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{at, parses, same_solid};
    use super::*;

    #[test]
    fn a_bare_object_gets_a_wrapper() {
        let src = "cube(2);\n";
        let edit = set_transform(src, 0..8, Transform::Translate, [1.0, 2.0, 3.5]).unwrap();
        assert_eq!(edit.text, "translate([1, 2, 3.5]) cube(2);\n");
        assert_eq!(&edit.text[edit.selection], "cube(2);");
        same_solid(&edit.text, "translate([1, 2, 3.5]) cube([2, 2, 2]);");
        // A modifier stays first.
        let edit = set_transform("#cube(2);", 0..9, Transform::Scale, [2.0, 1.0, 1.0]).unwrap();
        assert_eq!(edit.text, "#scale([2, 1, 1]) cube(2);");
    }

    #[test]
    fn the_wrapper_is_found_from_the_object_or_itself() {
        let src = "translate([1, 0, 0]) cube(2); // body\n";
        let whole = at(src, "translate([1, 0, 0]) cube(2);", 0);
        let inner = at(src, "cube(2);", 0);
        for (range, selected) in [(whole, "translate([5, 0, -1]) cube(2);"), (inner.clone(), "cube(2);")] {
            let edit = set_transform(src, range, Transform::Translate, [5.0, 0.0, -1.0]).unwrap();
            assert_eq!(edit.text, "translate([5, 0, -1]) cube(2); // body\n");
            assert_eq!(&edit.text[edit.selection], selected);
        }
        assert_eq!(transform_of(src, inner, Transform::Translate).unwrap(), [1.0, 0.0, 0.0]);
    }

    #[test]
    fn nested_transforms_change_the_innermost_of_their_kind() {
        let src = "translate([1, 0, 0])\n    rotate([0, 0, 45])\n        translate([2, 0, 0]) cube(1);\n";
        let range = at(src, "cube(1);", 0);
        let edit = set_transform(src, range.clone(), Transform::Translate, [3.0, 0.0, 0.0]).unwrap();
        assert_eq!(edit.text, src.replace("[2, 0, 0]", "[3, 0, 0]"));
        let edit = translate_by(src, range.clone(), [0.5, 0.0, 0.0]).unwrap();
        assert_eq!(edit.text, src.replace("[2, 0, 0]", "[2.5, 0, 0]"));
        let edit = set_transform(src, range, Transform::Rotate, [0.0, 0.0, 90.0]).unwrap();
        assert_eq!(edit.text, src.replace("[0, 0, 45]", "[0, 0, 90]"));
    }

    #[test]
    fn a_wrapper_with_other_children_is_left_alone() {
        let src = "translate([1, 0, 0]) {\n    cube(1);\n    sphere(1);\n}\n";
        let edit = set_transform(src, at(src, "cube(1);", 0), Transform::Translate, [0.0, 4.0, 0.0]).unwrap();
        assert_eq!(edit.text, "translate([1, 0, 0]) {\n    translate([0, 4, 0]) cube(1);\n    sphere(1);\n}\n");
        // Selected whole, the group is the object.
        let edit = set_transform(src, 0..src.len() - 1, Transform::Translate, [0.0, 4.0, 0.0]).unwrap();
        assert!(edit.text.starts_with("translate([0, 4, 0]) {"));
    }

    #[test]
    fn new_wrappers_follow_the_usual_order() {
        let src = "rotate([0, 0, 45]) cube(1);";
        let edit = set_transform(src, at(src, "cube(1);", 0), Transform::Translate, [5.0, 0.0, 0.0]).unwrap();
        assert_eq!(edit.text, "translate([5, 0, 0]) rotate([0, 0, 45]) cube(1);");
        same_solid(&edit.text, "translate([5, 0, 0]) rotate([0, 0, 45]) cube([1, 1, 1]);");
        let src = "translate([5, 0, 0]) color(\"red\") cube(1);";
        let edit = set_transform(src, at(src, "cube(1);", 0), Transform::Rotate, [0.0, 0.0, 30.0]).unwrap();
        assert_eq!(edit.text, "translate([5, 0, 0]) color(\"red\") rotate([0, 0, 30]) cube(1);");
        let src = "translate([5, 0, 0]) scale(2) cube(1);";
        let edit = set_transform(src, 0..src.len(), Transform::Rotate, [90.0, 0.0, 0.0]).unwrap();
        assert_eq!(edit.text, "translate([5, 0, 0]) rotate([90, 0, 0]) scale(2) cube(1);");
    }

    #[test]
    fn extrusions_are_objects() {
        let src = "linear_extrude(2) translate([1, 0]) square(3);";
        let edit = set_transform(src, at(src, "square(3);", 0), Transform::Translate, [0.0, 0.0, 4.0]).unwrap();
        assert_eq!(edit.text, "translate([0, 0, 4]) linear_extrude(2) translate([1, 0]) square(3);");
        same_solid(&edit.text, "translate([1, 0, 4]) cube([3, 3, 2]);");
    }

    #[test]
    fn computed_transforms_are_refused() {
        let src = "width = 10;\ntranslate([width / 2, 0, 0]) cube(1);";
        let err = set_transform(src, at(src, "cube(1);", 0), Transform::Translate, [0.0; 3]).unwrap_err();
        assert_eq!(err, "La valeur est calculée ([width / 2, 0, 0]) : remplacez-la par un nombre pour la modifier.");
        assert!(translate_by(src, at(src, "cube(1);", 0), [1.0, 0.0, 0.0]).is_err());
        assert!(transform_of(src, at(src, "cube(1);", 0), Transform::Translate).is_err());
        let src = "rotate(a = 30, v = [1, 1, 0]) cube(1);";
        assert!(set_transform(src, at(src, "cube(1);", 0), Transform::Rotate, [0.0; 3]).is_err());
    }

    #[test]
    fn deltas_compose() {
        let src = "scale([2, 1, 1]) cube(1);";
        let edit = scale_by(src, at(src, "cube(1);", 0), [1.5, 2.0, 1.0]).unwrap();
        assert_eq!(edit.text, "scale([3, 2, 1]) cube(1);");
        same_solid(&edit.text, "cube([3, 2, 1]);");
        let edit = translate_by("translate([1, 2]) cube(1);", 0..26, [1.0, 1.0, 1.0]).unwrap();
        assert_eq!(edit.text, "translate([2, 3, 1]) cube(1);");
        let edit = translate_by("translate() cube(1);", 0..20, [1.0, 0.0, 0.0]).unwrap();
        assert_eq!(edit.text, "translate([1, 0, 0]) cube(1);");
        parses(&edit.text);
    }
}
