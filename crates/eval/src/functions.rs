//! Built-in functions. Arguments are taken by position; a call that makes
//! no sense returns `undef` with a warning, as in the reference
//! implementation.

use crate::env::Env;
use crate::interp::{ArgValue, EResult, Interp};
use crate::math::{Mt19937, cos_degrees, sin_degrees, tan_degrees};
use crate::value::Value;

const NAMES: &[&str] = &[
    "abs", "sign", "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "floor", "round", "ceil", "ln", "log", "pow",
    "sqrt", "exp", "len", "min", "max", "norm", "cross", "concat", "lookup", "str", "chr", "ord", "search", "version",
    "version_num", "rands", "is_undef", "is_bool", "is_num", "is_string", "is_list", "is_function", "parent_module",
    "dxf_dim", "dxf_cross",
];

pub fn exists(name: &str) -> bool {
    NAMES.contains(&name)
}

/// How many arguments a builtin takes; a call with any other count gives
/// undef.
fn arity(name: &str) -> std::ops::RangeInclusive<usize> {
    match name {
        "atan2" | "pow" | "cross" | "lookup" => 2..=2,
        "log" => 1..=2,
        "version" => 0..=usize::MAX,
        "version_num" => 0..=0,
        "parent_module" => 0..=1,
        "search" => 2..=usize::MAX,
        "rands" => 0..=usize::MAX, // checked with its own warnings
        "min" | "max" => 1..=usize::MAX,
        "str" | "chr" | "concat" => 0..=usize::MAX,
        _ => 1..=1,
    }
}

pub fn call<'p>(interp: &mut Interp<'p>, name: &str, args: Vec<ArgValue<'p>>, env: &Env<'p>) -> EResult<Value<'p>> {
    if name == "dxf_dim" || name == "dxf_cross" {
        return Ok(dxf(interp, name, &args, env));
    }
    let args: Vec<Value<'p>> = args.into_iter().map(|(_, v)| v).collect();
    if !arity(name).contains(&args.len()) {
        interp.warn(format!("{name}() number of parameters does not match"));
        return Ok(Value::Undef);
    }
    let arg = |i: usize| args.get(i).cloned().unwrap_or(Value::Undef);
    let num = |i: usize| args.get(i).and_then(Value::as_number);
    let fail = |interp: &mut Interp<'p>| {
        interp.warn(format!("{name}() parameter could not be converted"));
        Value::Undef
    };
    let unary = |f: fn(f64) -> f64| num(0).map(|x| Value::Number(f(x)));

    let result = match name {
        "abs" => unary(f64::abs),
        "sign" => unary(|x| if x < 0.0 { -1.0 } else if x > 0.0 { 1.0 } else { 0.0 }),
        "sin" => unary(sin_degrees),
        "cos" => unary(cos_degrees),
        "tan" => unary(tan_degrees),
        "asin" => unary(|x| x.asin().to_degrees()),
        "acos" => unary(|x| x.acos().to_degrees()),
        "atan" => unary(|x| x.atan().to_degrees()),
        "atan2" => num(0).zip(num(1)).map(|(y, x)| Value::Number(y.atan2(x).to_degrees())),
        "floor" => unary(f64::floor),
        "round" => unary(f64::round),
        "ceil" => unary(f64::ceil),
        "ln" => unary(f64::ln),
        "sqrt" => unary(f64::sqrt),
        "exp" => unary(f64::exp),
        "pow" => num(0).zip(num(1)).map(|(a, b)| Value::Number(a.powf(b))),
        "log" => match args.len() {
            1 => num(0).map(|x| Value::Number(x.log10())),
            _ => num(0).zip(num(1)).map(|(base, x)| Value::Number(x.ln() / base.ln())),
        },
        "len" => match &arg(0) {
            Value::Vector(items) => Some(Value::Number(items.len() as f64)),
            Value::Str(s) => Some(Value::Number(s.chars().count() as f64)),
            _ => None,
        },
        "min" | "max" => extremum(&args, name == "max"),
        "norm" => arg(0).as_numbers().map(|v| Value::Number(v.iter().map(|x| x * x).sum::<f64>().sqrt())),
        "cross" => cross(&arg(0), &arg(1)),
        "concat" => {
            let mut out = crate::value::ListBuilder::default();
            for a in &args {
                match a {
                    Value::Vector(items) => out.splice(items),
                    other => out.push(other.clone()),
                }
            }
            Some(out.finish())
        }
        "lookup" => num(0).and_then(|key| lookup(key, &arg(1))),
        "str" => Some(Value::str(&args.iter().map(Value::to_display).collect::<String>())),
        "chr" => Some(Value::str(&args.iter().map(chr).collect::<String>())),
        "ord" => match &arg(0) {
            Value::Str(s) => s.chars().next().map(|c| Value::Number(c as u32 as f64)),
            _ => None,
        },
        "search" => search(&args),
        "version" => Some(Value::vector(vec![Value::Number(2021.0), Value::Number(1.0), Value::Number(0.0)])),
        "version_num" => Some(Value::Number(20210100.0)),
        "rands" => rands(interp, &args),
        "is_undef" => Some(Value::Bool(arg(0).is_undef())),
        "is_bool" => Some(Value::Bool(matches!(arg(0), Value::Bool(_)))),
        "is_num" => Some(Value::Bool(num(0).is_some_and(|n| !n.is_nan()))),
        "is_string" => Some(Value::Bool(matches!(arg(0), Value::Str(_)))),
        "is_list" => Some(Value::Bool(matches!(arg(0), Value::Vector(_)))),
        "is_function" => Some(Value::Bool(matches!(arg(0), Value::Function(_)))),
        // parent_module(0) is the current module; the default, 1, its caller.
        "parent_module" => {
            let n = match args.first() {
                None => Some(1.0),
                Some(v) => v.as_number().map(f64::trunc),
            };
            let stack = &interp.module_stack;
            match n {
                Some(n) if n >= 0.0 && (n as usize) < stack.len() => Some(Value::str(stack[stack.len() - 1 - n as usize])),
                Some(n) => {
                    interp.warn(format!("Parent module index ({n}) greater than the number of modules on the stack"));
                    Some(Value::Undef)
                }
                None => Some(Value::Undef),
            }
        }
        _ => unreachable!("unknown builtin function {name}"),
    };
    Ok(result.unwrap_or_else(|| fail(interp)))
}

/// `dxf_dim(file, name, layer, origin, scale)` and
/// `dxf_cross(file, layer, origin, scale)`, which take named arguments.
fn dxf<'p>(interp: &mut Interp<'p>, name: &str, args: &[ArgValue<'p>], _env: &Env<'p>) -> Value<'p> {
    let order: &[&str] =
        if name == "dxf_dim" { &["file", "name", "layer", "origin", "scale"] } else { &["file", "layer", "origin", "scale"] };
    let get = |param: &str| {
        let position = order.iter().position(|p| *p == param)?;
        args.iter()
            .find(|(n, _)| *n == Some(param))
            .or_else(|| args.iter().filter(|(n, _)| n.is_none()).nth(position))
            .map(|(_, v)| v.clone())
    };
    let text = |param: &str| get(param).and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
    let origin = get("origin").and_then(|v| v.as_vec_n(2)).map_or([0.0, 0.0], |v| [v[0], v[1]]);
    let scale = get("scale").and_then(|v| v.as_number()).unwrap_or(1.0);
    let path = interp.data_path(_env, &text("file")).0;
    let result = if name == "dxf_dim" {
        crate::dxf::dim(&path, &text("name"), &text("layer"), origin, scale).map(Value::Number)
    } else {
        crate::dxf::cross(&path, &text("layer"), origin, scale)
            .map(|[x, y]| Value::vector(vec![Value::Number(x), Value::Number(y)]))
    };
    result.unwrap_or_else(|message| {
        interp.warn(message);
        Value::Undef
    })
}

/// `max(v)` orders the elements of one vector with the generic value
/// order (vectors and strings lexicographically; values that do not
/// compare keep the earlier one). `max(a, b, ...)` takes numbers only.
fn extremum<'p>(args: &[Value<'p>], max: bool) -> Option<Value<'p>> {
    let better = |a: &Value<'p>, b: &Value<'p>| {
        let wanted = if max { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Less };
        match (a, b) {
            (Value::Number(x), Value::Number(y)) => if max { y > x } else { y < x },
            _ => b.compare(a) == Some(wanted),
        }
    };
    let candidates: Vec<Value<'p>> = match args {
        [Value::Vector(items)] => items.to_vec(),
        [single @ Value::Number(_)] => vec![single.clone()],
        _ if args.iter().all(|a| matches!(a, Value::Number(_))) => args.to_vec(),
        _ => return None,
    };
    let mut best = candidates.first()?.clone();
    for candidate in &candidates[1..] {
        if better(&best, candidate) {
            best = candidate.clone();
        }
    }
    Some(best)
}

fn cross<'p>(a: &Value<'p>, b: &Value<'p>) -> Option<Value<'p>> {
    if let (Some(a), Some(b)) = (a.as_vec_n(2), b.as_vec_n(2)) {
        return Some(Value::Number(a[0] * b[1] - a[1] * b[0]));
    }
    let (a, b) = (a.as_vec_n(3)?, b.as_vec_n(3)?);
    Some(Value::vector(vec![
        Value::Number(a[1] * b[2] - a[2] * b[1]),
        Value::Number(a[2] * b[0] - a[0] * b[2]),
        Value::Number(a[0] * b[1] - a[1] * b[0]),
    ]))
}

/// Piecewise-linear interpolation in a table of `[key, value]` pairs,
/// clamped at both ends.
fn lookup<'p>(key: f64, table: &Value<'p>) -> Option<Value<'p>> {
    let pairs: Vec<(f64, f64)> = table.as_vector()?.iter().filter_map(|p| p.as_vec_n(2).map(|v| (v[0], v[1]))).collect();
    let &(mut low_p, mut low_v) = pairs.first()?;
    let (mut high_p, mut high_v) = (low_p, low_v);
    for &(p, v) in &pairs[1..] {
        if p <= key && (p > low_p || low_p > key) {
            (low_p, low_v) = (p, v);
        }
        if p >= key && (p < high_p || high_p < key) {
            (high_p, high_v) = (p, v);
        }
    }
    if key <= low_p {
        return Some(Value::Number(high_v));
    }
    if key >= high_p {
        return Some(Value::Number(low_v));
    }
    let f = (key - low_p) / (high_p - low_p);
    Some(Value::Number(high_v * f + low_v * (1.0 - f)))
}

fn chr(value: &Value) -> String {
    let code = |n: f64| char::from_u32(n as i64 as u32).filter(|_| (1.0..1_114_112.0).contains(&n));
    match value {
        Value::Number(n) => code(*n).map(String::from).unwrap_or_default(),
        Value::Vector(items) => items.iter().map(chr).collect(),
        Value::Range(r) if r.len() < 1_000_000 => r.values().filter_map(code).collect(),
        _ => String::new(),
    }
}

/// `search(match, table, num_returns = 1, index_col = 0)`.
fn search<'p>(args: &[Value<'p>]) -> Option<Value<'p>> {
    let needle = args.first()?;
    let haystack = args.get(1)?;
    if needle.is_undef() {
        return None;
    }
    let num_returns = args.get(2).and_then(Value::as_number).unwrap_or(1.0) as i64;
    let column = args.get(3).and_then(Value::as_number).unwrap_or(0.0) as usize;

    let candidates: Vec<Value<'p>> = match haystack {
        Value::Str(s) => s.chars().map(|c| Value::str(c.encode_utf8(&mut [0; 4]))).collect(),
        Value::Vector(items) => items.to_vec(),
        _ => return Some(Value::vector(Vec::new())),
    };
    // A plain row only matches as column 0.
    let key = |item: &Value<'p>, needle_is_vector: bool| match item {
        Value::Vector(row) if !needle_is_vector => row.get(column).cloned().unwrap_or(Value::Undef),
        _ if column != 0 && !needle_is_vector => Value::Undef,
        other => other.clone(),
    };
    let matches = |needle: &Value<'p>| -> Vec<Value<'p>> {
        let is_vector = matches!(needle, Value::Vector(_));
        let found = candidates.iter().enumerate().filter(|(_, item)| key(item, is_vector).equals(needle));
        let found = found.map(|(i, _)| Value::Number(i as f64));
        if num_returns <= 0 { found.collect() } else { found.take(num_returns as usize).collect() }
    };

    let needles: Vec<Value<'p>> = match needle {
        Value::Str(s) => return Some(search_chars(s, haystack, num_returns, column)),
        Value::Vector(items) => items.to_vec(),
        // A single number or other value yields a flat list of positions.
        other => return Some(Value::vector(matches(other))),
    };
    let mut out = Vec::new();
    for needle in &needles {
        let found = matches(needle);
        if num_returns == 1 {
            out.push(found.into_iter().next().unwrap_or_else(|| Value::vector(Vec::new())));
        } else {
            out.push(Value::vector(found));
        }
    }
    Some(Value::vector(out))
}

/// Searches each character of `needle`: among the characters of a string,
/// or against the first character of column `column` of a table, every
/// row of which must have that column (else nothing is found at all).
/// A character found nowhere is skipped when `num_returns` is 1.
fn search_chars<'p>(needle: &str, haystack: &Value<'p>, num_returns: i64, column: usize) -> Value<'p> {
    let keys: Vec<Option<char>> = match haystack {
        Value::Str(s) => s.chars().map(Some).collect(),
        Value::Vector(rows) => {
            let mut keys = Vec::with_capacity(rows.len());
            for row in rows.iter() {
                match row.as_vector().and_then(|r| r.get(column)) {
                    Some(cell) => keys.push(cell.as_str().and_then(|s| s.chars().next())),
                    None => return Value::vector(Vec::new()),
                }
            }
            keys
        }
        _ => return Value::Undef,
    };
    let mut out = Vec::new();
    for c in needle.chars() {
        let found = keys.iter().enumerate().filter(|(_, k)| **k == Some(c)).map(|(i, _)| Value::Number(i as f64));
        let found: Vec<Value<'p>> =
            if num_returns <= 0 { found.collect() } else { found.take(num_returns as usize).collect() };
        match (num_returns, found.first()) {
            (1, Some(first)) => out.push(first.clone()),
            (1, None) => {}
            _ => out.push(Value::vector(found)),
        }
    }
    Value::vector(out)
}

/// Results `rands()` will produce at most; the reference has no limit and
/// runs out of memory instead.
const MAX_RANDS: f64 = 10_000_000.0;

/// `rands(min, max, count, seed)`, with the reference's argument handling:
/// one shared generator, reseeded by every seeded call and continued by the
/// others; infinite bounds clamp to half the largest double; bounds in the
/// wrong order swap.
fn rands<'p>(interp: &mut Interp<'p>, args: &[Value<'p>]) -> Option<Value<'p>> {
    if !(3..=4).contains(&args.len()) {
        interp.warn("rands() number of parameters does not match");
        return Some(Value::Undef);
    }
    let bound = |interp: &mut Interp<'p>, v: f64, which: &str, reset: f64| {
        if v.is_finite() {
            return v;
        }
        interp.warn(format!("rands() range {which} cannot be infinite"));
        reset
    };
    let Some(min) = args[0].as_number() else { return Some(Value::Undef) };
    let min = bound(interp, min, "min", -f64::MAX / 2.0);
    let Some(max) = args[1].as_number() else { return Some(Value::Undef) };
    let max = bound(interp, max, "max", f64::MAX / 2.0);
    let (min, max) = if max < min { (max, min) } else { (min, max) };
    let Some(count) = args[2].as_number() else { return Some(Value::Undef) };
    let mut count = count.abs();
    if !count.is_finite() {
        interp.warn("rands() cannot create an infinite number of results");
        count = 1.0;
    }
    if count >= MAX_RANDS {
        interp.warn(format!("rands() will not create {} results", crate::value::number(count)));
        return Some(Value::Undef);
    }
    if let Some(seed) = args.get(3) {
        let Some(seed) = seed.as_number() else { return Some(Value::Undef) };
        interp.rng = Mt19937::new(seed_hash(seed) as u32);
    }
    let values = (0..count as usize)
        .map(|_| Value::Number(if min == max { min } else { interp.rng.uniform(min, max) }))
        .collect();
    Some(Value::vector(values))
}

/// How the reference turns a seed into 32 bits: the numeric hash of Python
/// (modulo 2^31 - 1, without its -1 to -2 adjustment), truncated.
fn seed_hash(x: f64) -> i64 {
    const BITS: u32 = 31;
    const MODULUS: u64 = (1 << BITS) - 1;
    if x.is_nan() {
        return 0;
    }
    if x.is_infinite() {
        return if x > 0.0 { 314_159 } else { -314_159 };
    }
    let (mut mantissa, mut exponent) = frexp(x.abs());
    let mut hash: u64 = 0;
    while mantissa != 0.0 {
        hash = ((hash << 28) & MODULUS) | (hash >> (BITS - 28));
        mantissa *= 268_435_456.0;
        exponent -= 28;
        let digit = mantissa as u64;
        mantissa -= digit as f64;
        hash += digit;
        if hash >= MODULUS {
            hash -= MODULUS;
        }
    }
    let shift = if exponent >= 0 { exponent % 31 } else { 31 - 1 - ((-1 - exponent) % 31) } as u32;
    hash = ((hash << shift) & MODULUS) | (hash >> (BITS - shift));
    if x < 0.0 { -(hash as i64) } else { hash as i64 }
}

/// `x = mantissa * 2^exponent` with the mantissa in [0.5, 1).
fn frexp(x: f64) -> (f64, i32) {
    if x == 0.0 || !x.is_finite() {
        return (x, 0);
    }
    let (x, bias) = if x.abs() < f64::MIN_POSITIVE { (x * 2f64.powi(54), -54) } else { (x, 0) };
    let bits = x.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1022;
    let mantissa = f64::from_bits((bits & !(0x7ff << 52)) | (1022 << 52));
    (mantissa, exponent + bias)
}

#[cfg(test)]
mod tests {
    use super::seed_hash;

    #[test]
    fn seed_hash_matches_python_modulo_mersenne_31() {
        assert_eq!(seed_hash(42.0), 42);
        assert_eq!(seed_hash(0.0), 0);
        assert_eq!(seed_hash(-7.0), -7);
        // 0.5 = 1 / 2: the inverse of 2 modulo 2^31 - 1.
        assert_eq!(seed_hash(0.5), 1 << 30);
        assert_eq!(seed_hash(2147483647.0), 0);
        assert_eq!(seed_hash(f64::INFINITY), 314_159);
    }
}
