//! Runtime values and the operators on them, with the semantics of the
//! reference implementation (release 2021.01): an operation that makes no
//! sense yields `undef` (and a warning from the caller) rather than an
//! error.

use std::cell::{OnceCell, RefCell};
use std::cmp::Ordering;
use std::ops::Deref;
use std::rc::Rc;

use c2f_lang::ast::{Expr, Param};
use c2f_lang::dump;

use crate::env::Env;

#[derive(Clone)]
pub enum Value<'p> {
    Undef,
    Bool(bool),
    Number(f64),
    Str(Rc<str>),
    Vector(Rc<List<'p>>),
    Range(Range),
    Function(Rc<Closure<'p>>),
}

/// A vector. Concatenations (`concat`, `each`) share their parts instead
/// of copying them, as the reference's embedded vectors do, so
/// accumulating a list element by element or doubling it stays cheap; the
/// elements are laid out flat the first time they are read.
pub struct List<'p> {
    len: usize,
    /// The parts of a concatenation not yet flattened.
    parts: RefCell<Vec<Part<'p>>>,
    flat: OnceCell<Vec<Value<'p>>>,
}

pub enum Part<'p> {
    Items(Vec<Value<'p>>),
    List(Rc<List<'p>>),
}

/// Below this many elements a spliced list is copied rather than shared.
const SHARE_FROM: usize = 32;

impl<'p> List<'p> {
    pub fn flat(items: Vec<Value<'p>>) -> Self {
        let flat = OnceCell::new();
        let len = items.len();
        let _ = flat.set(items);
        List { len, parts: RefCell::new(Vec::new()), flat }
    }

    /// The number of elements, without flattening.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn flatten(&self) -> Vec<Value<'p>> {
        let parts = std::mem::take(&mut *self.parts.borrow_mut());
        let mut out = Vec::with_capacity(self.len);
        // Depth-first over the shared parts, without recursion: concatenation
        // chains can be as long as the lists are.
        let mut stack: Vec<(Rc<List<'p>>, usize)> = Vec::new();
        for part in parts {
            match part {
                Part::Items(items) => out.extend(items),
                Part::List(list) => {
                    stack.push((list, 0));
                    while let Some((list, i)) = stack.pop() {
                        if let Some(flat) = list.flat.get() {
                            out.extend_from_slice(flat);
                            continue;
                        }
                        let next = match list.parts.borrow().get(i) {
                            None => continue,
                            Some(Part::Items(items)) => {
                                out.extend_from_slice(items);
                                None
                            }
                            Some(Part::List(inner)) => Some(inner.clone()),
                        };
                        stack.push((list, i + 1));
                        if let Some(inner) = next {
                            stack.push((inner, 0));
                        }
                    }
                }
            }
        }
        out
    }
}

impl<'p> Deref for List<'p> {
    type Target = [Value<'p>];

    fn deref(&self) -> &[Value<'p>] {
        self.flat.get_or_init(|| self.flatten())
    }
}

/// Frees long concatenation chains without recursing.
impl Drop for List<'_> {
    fn drop(&mut self) {
        let mut pending = std::mem::take(self.parts.get_mut());
        while let Some(part) = pending.pop() {
            if let Part::List(list) = part {
                if let Ok(mut list) = Rc::try_unwrap(list) {
                    pending.append(list.parts.get_mut());
                }
            }
        }
    }
}

/// Builds a vector from values and spliced vectors.
#[derive(Default)]
pub struct ListBuilder<'p> {
    parts: Vec<Part<'p>>,
    items: Vec<Value<'p>>,
    len: usize,
}

impl<'p> ListBuilder<'p> {
    pub fn push(&mut self, value: Value<'p>) {
        self.items.push(value);
        self.len += 1;
    }

    pub fn extend(&mut self, values: impl IntoIterator<Item = Value<'p>>) {
        for value in values {
            self.push(value);
        }
    }

    /// Appends the elements of `list`, sharing it when it is large.
    pub fn splice(&mut self, list: &Rc<List<'p>>) {
        if list.len() < SHARE_FROM {
            self.extend(list.iter().cloned());
            return;
        }
        if !self.items.is_empty() {
            self.parts.push(Part::Items(std::mem::take(&mut self.items)));
        }
        self.len += list.len();
        self.parts.push(Part::List(list.clone()));
    }

    pub fn finish(mut self) -> Value<'p> {
        if self.parts.is_empty() {
            return Value::vector(self.items);
        }
        if !self.items.is_empty() {
            self.parts.push(Part::Items(self.items));
        }
        Value::Vector(Rc::new(List { len: self.len, parts: RefCell::new(self.parts), flat: OnceCell::new() }))
    }
}

/// A function literal and the environment it closes over.
pub struct Closure<'p> {
    pub params: &'p [Param],
    pub body: &'p Expr,
    pub env: Env<'p>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub begin: f64,
    pub step: f64,
    pub end: f64,
}

impl Range {
    /// `[begin : end]`; a reversed pair is swapped, as the reference still
    /// does (with a deprecation warning).
    pub fn new(begin: f64, step: Option<f64>, end: f64) -> (Range, bool) {
        match step {
            Some(step) => (Range { begin, step, end }, false),
            None if begin > end => (Range { begin: end, step: 1.0, end: begin }, true),
            None => (Range { begin, step: 1.0, end }, false),
        }
    }

    /// Number of values, computed exactly as the reference's 32-bit
    /// counter: a quotient one ulp short of an integer still counts it, and
    /// huge counts wrap around.
    pub fn len(&self) -> u64 {
        const MAX: u64 = u32::MAX as u64;
        let (b, s, e) = (self.begin, self.step, self.end);
        if b.is_nan() || s.is_nan() || e.is_nan() {
            return 0;
        }
        if (s < 0.0 && b < e) || (s >= 0.0 && b > e) {
            return 0;
        }
        if b == e || s.is_infinite() {
            return 1;
        }
        if b.is_infinite() || e.is_infinite() || s == 0.0 {
            return MAX;
        }
        let y = next_after((e - b) / s, MAX as f64);
        // x86 truncation: out of i64 range gives INT64_MIN, whose low 32
        // bits are zero.
        let n = if y.abs() < 9_223_372_036_854_775_808.0 { (y as i64) as u32 as u64 } else { 0 };
        if n == MAX { MAX } else { n + 1 }
    }

    pub fn get(&self, i: u64) -> f64 {
        if i == 0 { self.begin } else { self.begin + i as f64 * self.step }
    }

    /// The values; none at all when the step is zero.
    pub fn values(&self) -> impl Iterator<Item = f64> + '_ {
        let len = if self.step == 0.0 { 0 } else { self.len() };
        (0..len).map(|i| self.get(i))
    }

    /// The reference orders ranges by (begin, step, count); empty ranges
    /// are equal to each other and below all others.
    fn compare(&self, other: &Range) -> Ordering {
        match (self.len(), other.len()) {
            (0, 0) => Ordering::Equal,
            (0, _) => Ordering::Less,
            (_, 0) => Ordering::Greater,
            (n1, n2) => self
                .begin
                .partial_cmp(&other.begin)
                .unwrap_or(Ordering::Equal)
                .then(self.step.partial_cmp(&other.step).unwrap_or(Ordering::Equal))
                .then(n1.cmp(&n2)),
        }
    }
}

impl<'p> Value<'p> {
    pub fn str(s: &str) -> Self {
        Value::Str(s.into())
    }

    pub fn vector(items: Vec<Value<'p>>) -> Self {
        Value::Vector(Rc::new(List::flat(items)))
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Undef => "undefined",
            Value::Bool(_) => "bool",
            Value::Number(_) => "number",
            Value::Str(_) => "string",
            Value::Vector(_) => "vector",
            Value::Range(_) => "range",
            Value::Function(_) => "function",
        }
    }

    pub fn is_undef(&self) -> bool {
        matches!(self, Value::Undef)
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_vector(&self) -> Option<&[Value<'p>]> {
        match self {
            Value::Vector(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// A number, or 0 for anything else (the reference's `toDouble`).
    pub fn to_f64(&self) -> f64 {
        self.as_number().unwrap_or(0.0)
    }

    /// All-number vector of exactly `n` elements.
    pub fn as_vec_n(&self, n: usize) -> Option<Vec<f64>> {
        let items = self.as_vector()?;
        if items.len() != n {
            return None;
        }
        items.iter().map(Value::as_number).collect()
    }

    /// All-number vector of any length.
    pub fn as_numbers(&self) -> Option<Vec<f64>> {
        self.as_vector()?.iter().map(Value::as_number).collect()
    }

    pub fn truthy(&self) -> bool {
        match self {
            Value::Undef => false,
            Value::Bool(b) => *b,
            Value::Number(n) => *n != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::Vector(v) => !v.is_empty(),
            Value::Range(_) | Value::Function(_) => true,
        }
    }

    /// How `echo` and vectors print a value: strings quoted, unescaped.
    pub fn repr(&self) -> String {
        let mut out = String::new();
        self.write_repr(&mut out);
        out
    }

    fn write_repr(&self, out: &mut String) {
        match self {
            Value::Str(s) => {
                out.push('"');
                out.push_str(s);
                out.push('"');
            }
            Value::Vector(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    item.write_repr(out);
                }
                out.push(']');
            }
            other => out.push_str(&other.to_display()),
        }
    }

    /// How `str()` converts a value: like [`repr`](Self::repr) but a
    /// top-level string is not quoted.
    pub fn to_display(&self) -> String {
        match self {
            Value::Undef => "undef".into(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => number(*n),
            Value::Str(s) => s.to_string(),
            Value::Vector(_) => self.repr(),
            Value::Range(r) => format!("[{} : {} : {}]", number(r.begin), number(r.step), number(r.end)),
            Value::Function(f) => format!("function({}) {}", dump::params(f.params), dump::expr(f.body)),
        }
    }

    // ---- operators ----

    pub fn equals(&self, other: &Value<'p>) -> bool {
        match (self, other) {
            (Value::Undef, Value::Undef) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Number(a), Value::Number(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Vector(a), Value::Vector(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.equals(y))
            }
            (Value::Range(a), Value::Range(b)) => a.compare(b) == Ordering::Equal,
            (Value::Function(a), Value::Function(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// Ordering for `<`, `>`, ...; `None` when the types do not compare.
    pub fn compare(&self, other: &Value<'p>) -> Option<Ordering> {
        match (self, other) {
            (Value::Number(a), Value::Number(b)) => a.partial_cmp(b),
            (Value::Str(a), Value::Str(b)) => Some(a.cmp(b)),
            (Value::Bool(a), Value::Bool(b)) => Some(a.cmp(b)),
            (Value::Range(a), Value::Range(b)) => Some(a.compare(b)),
            (Value::Vector(a), Value::Vector(b)) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    match x.compare(y)? {
                        Ordering::Equal => {}
                        unequal => return Some(unequal),
                    }
                }
                Some(a.len().cmp(&b.len()))
            }
            _ => None,
        }
    }

    pub fn add(&self, other: &Value<'p>) -> Option<Value<'p>> {
        elementwise(self, other, |a, b| a + b, Value::add)
    }

    pub fn sub(&self, other: &Value<'p>) -> Option<Value<'p>> {
        elementwise(self, other, |a, b| a - b, Value::sub)
    }

    pub fn neg(&self) -> Option<Value<'p>> {
        match self {
            Value::Number(n) => Some(Value::Number(-n)),
            Value::Vector(items) => {
                Some(Value::vector(items.iter().map(|v| v.neg().unwrap_or(Value::Undef)).collect()))
            }
            _ => None,
        }
    }

    pub fn mul(&self, other: &Value<'p>) -> Result<Value<'p>, String> {
        match (self, other) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a * b)),
            (Value::Number(_), Value::Vector(items)) | (Value::Vector(items), Value::Number(_)) => {
                let scalar = if let Value::Number(_) = self { self } else { other };
                Ok(Value::vector(items.iter().map(|v| v.mul(scalar).unwrap_or(Value::Undef)).collect()))
            }
            (Value::Vector(a), Value::Vector(b)) => multiply_vectors(a, b),
            _ => Err(undefined_operation("*", self, other)),
        }
    }

    pub fn div(&self, other: &Value<'p>) -> Option<Value<'p>> {
        match (self, other) {
            (Value::Number(a), Value::Number(b)) => Some(Value::Number(a / b)),
            (Value::Vector(items), Value::Number(_)) => {
                Some(Value::vector(items.iter().map(|v| v.div(other).unwrap_or(Value::Undef)).collect()))
            }
            (Value::Number(_), Value::Vector(items)) => {
                Some(Value::vector(items.iter().map(|v| self.div(v).unwrap_or(Value::Undef)).collect()))
            }
            _ => None,
        }
    }

    pub fn rem(&self, other: &Value<'p>) -> Option<Value<'p>> {
        match (self, other) {
            (Value::Number(a), Value::Number(b)) => Some(Value::Number(a % b)),
            _ => None,
        }
    }

    pub fn pow(&self, other: &Value<'p>) -> Option<Value<'p>> {
        match (self, other) {
            (Value::Number(a), Value::Number(b)) => Some(Value::Number(a.powf(*b))),
            _ => None,
        }
    }

    /// `v[i]`: indices are truncated; anything else yields `undef`.
    pub fn index(&self, index: &Value<'p>) -> Value<'p> {
        let Value::Number(i) = index else { return Value::Undef };
        if i.is_nan() || *i <= -1.0 || i.is_infinite() {
            return Value::Undef;
        }
        let i = *i as u64;
        match self {
            Value::Vector(items) => items.get(i as usize).cloned().unwrap_or(Value::Undef),
            Value::Str(s) => s.chars().nth(i as usize).map_or(Value::Undef, |c| Value::str(c.encode_utf8(&mut [0; 4]))),
            // A range indexes as its triple, not its values: [0:3][2] is 3.
            Value::Range(r) => match i {
                0 => Value::Number(r.begin),
                1 => Value::Number(r.step),
                2 => Value::Number(r.end),
                _ => Value::Undef,
            },
            _ => Value::Undef,
        }
    }
}

/// The next double after `x` in the direction of `toward`.
fn next_after(x: f64, toward: f64) -> f64 {
    if x.is_nan() || x == toward {
        return x;
    }
    if x == 0.0 {
        return if toward > 0.0 { f64::from_bits(1) } else { -f64::from_bits(1) };
    }
    let bits = x.to_bits();
    let up = (toward > x) == (x > 0.0);
    f64::from_bits(if up { bits + 1 } else { bits - 1 })
}

pub fn undefined_operation(op: &str, a: &Value, b: &Value) -> String {
    format!("undefined operation ({} {op} {})", a.type_name(), b.type_name())
}

fn elementwise<'p>(
    a: &Value<'p>,
    b: &Value<'p>,
    numbers: fn(f64, f64) -> f64,
    recurse: fn(&Value<'p>, &Value<'p>) -> Option<Value<'p>>,
) -> Option<Value<'p>> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => Some(Value::Number(numbers(*x, *y))),
        (Value::Vector(x), Value::Vector(y)) => Some(Value::vector(
            x.iter().zip(y.iter()).map(|(p, q)| recurse(p, q).unwrap_or(Value::Undef)).collect(),
        )),
        _ => None,
    }
}

/// Dot product, matrix-vector, vector-matrix or matrix-matrix product.
fn multiply_vectors<'p>(a: &[Value<'p>], b: &[Value<'p>]) -> Result<Value<'p>, String> {
    let numbers = |v: &[Value]| v.iter().map(Value::as_number).collect::<Option<Vec<f64>>>();
    let matrix = |v: &[Value]| {
        v.iter().map(|row| row.as_vector().and_then(numbers)).collect::<Option<Vec<Vec<f64>>>>()
    };
    let dot = |x: &[f64], y: &[f64]| x.iter().zip(y).map(|(p, q)| p * q).sum::<f64>();
    let column = |m: &[Vec<f64>], j: usize| m.iter().map(|row| row[j]).collect::<Vec<f64>>();
    let rectangular = |m: &[Vec<f64>]| m.first().is_some_and(|first| m.iter().all(|row| row.len() == first.len()));
    let fail = || Err("undefined operation (vector * vector)".to_string());

    if a.is_empty() || b.is_empty() {
        return fail();
    }
    if let (Some(x), Some(y)) = (numbers(a), numbers(b)) {
        if x.len() != y.len() {
            return Err(format!("vector*vector requires matching lengths ({} != {})", x.len(), y.len()));
        }
        return Ok(Value::Number(dot(&x, &y)));
    }
    match (matrix(a), numbers(b)) {
        (Some(m), Some(v)) if !m.is_empty() => {
            if m.iter().any(|row| row.len() != v.len()) {
                return fail();
            }
            return Ok(Value::vector(m.iter().map(|row| Value::Number(dot(row, &v))).collect()));
        }
        _ => {}
    }
    match (numbers(a), matrix(b)) {
        (Some(v), Some(m)) if rectangular(&m) => {
            if m.len() != v.len() {
                return fail();
            }
            let cols = m[0].len();
            return Ok(Value::vector((0..cols).map(|j| Value::Number(dot(&v, &column(&m, j)))).collect()));
        }
        _ => {}
    }
    match (matrix(a), matrix(b)) {
        (Some(x), Some(y)) if rectangular(&x) && rectangular(&y) => {
            if x[0].len() != y.len() {
                return fail();
            }
            let cols = y[0].len();
            Ok(Value::vector(
                x.iter()
                    .map(|row| Value::vector((0..cols).map(|j| Value::Number(dot(row, &column(&y, j)))).collect()))
                    .collect(),
            ))
        }
        _ => fail(),
    }
}

/// Numbers as the reference prints values: six significant digits, and no
/// negative zero.
pub fn number(n: f64) -> String {
    if n == 0.0 { "0".into() } else { dump::number(n) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(n: f64) -> Value<'static> {
        Value::Number(n)
    }

    fn vec(items: &[f64]) -> Value<'static> {
        Value::vector(items.iter().map(|&n| num(n)).collect())
    }

    #[test]
    fn range_lengths() {
        let r = |b, s, e| Range { begin: b, step: s, end: e }.len();
        assert_eq!(r(0.0, 1.0, 2.0), 3);
        assert_eq!(r(0.0, 0.5, 2.0), 5);
        assert_eq!(r(0.0, -1.0, -3.0), 4);
        assert_eq!(r(5.0, 1.0, 1.0), 0);
        assert_eq!(r(1.0, 1.0, 1.0), 1);
        // 0.3 / 0.1 is one ulp below 3, which still counts.
        assert_eq!(r(0.0, 0.1, 0.3), 4);
        assert_eq!(r(0.3, -0.1, 0.0), 4);
        // The 32-bit count wraps: the reference gives [0, 1].
        assert_eq!(r(0.0, 1.0, 4294967298.0), 2);
        assert_eq!(r(1.0, f64::INFINITY, 1.0), 1);
        let zero_step = Range { begin: 1.0, step: 0.0, end: 1.0 };
        assert_eq!((zero_step.len(), zero_step.values().count()), (1, 0));
        let infinite_step = Range { begin: 0.0, step: f64::INFINITY, end: 5.0 };
        assert_eq!(infinite_step.values().collect::<Vec<_>>(), [0.0]);
        assert_eq!(Range::new(5.0, None, 1.0), (Range { begin: 1.0, step: 1.0, end: 5.0 }, true));
    }

    #[test]
    fn products() {
        assert_eq!(vec(&[1.0, 2.0]).mul(&vec(&[3.0, 4.0])).unwrap().repr(), "11");
        let m = Value::vector(vec![vec(&[1.0, 2.0]), vec(&[3.0, 4.0])]);
        assert_eq!(m.mul(&vec(&[1.0, 1.0])).unwrap().repr(), "[3, 7]");
        assert_eq!(vec(&[1.0, 2.0]).mul(&m).unwrap().repr(), "[7, 10]");
        assert_eq!(m.mul(&m).unwrap().repr(), "[[7, 10], [15, 22]]");
        assert!(vec(&[1.0, 2.0, 3.0]).mul(&vec(&[1.0, 2.0])).is_err());
    }

    #[test]
    fn indexing_truncates() {
        let v = vec(&[1.0, 2.0, 3.0]);
        assert_eq!(v.index(&num(1.7)).repr(), "2");
        assert_eq!(v.index(&num(-0.5)).repr(), "1");
        assert!(v.index(&num(-1.0)).is_undef());
        assert_eq!(Value::str("abc").index(&num(1.5)).repr(), "\"b\"");
    }
}
