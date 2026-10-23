//! The figures of a part's sheet that the mechanical checks compute with:
//! a `// @spec` whose key is one of [`KEYS`] holds a quantity, a number
//! and its unit, read here in one unit per kind (grams, millinewton
//! meters, turns per minute, newtons):
//!
//! ```text
//! // @spec mass: 3.3 g
//! // @spec torque: 34 mN·m at 5 V
//! // @spec stall torque: 0.8 kgf·cm
//! // @spec speed: 15 rpm
//! // @spec max radial load: 20 N
//! // @spec dynamic load: 3.3 kN
//! ```
//!
//! The first number followed by a unit of the key's kind is the figure;
//! the words around it (`at 5 V`, `nominal`) are for the reader.

use c2f_lang::annotations::Spec;

/// What a figure measures, and the unit it is read in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Grams.
    Mass,
    /// Millinewton meters.
    Torque,
    /// Turns per minute.
    Speed,
    /// Newtons.
    Force,
}

impl Kind {
    /// The unit its figures are read in.
    pub fn unit(self) -> &'static str {
        match self {
            Kind::Mass => "g",
            Kind::Torque => "mN·m",
            Kind::Speed => "rpm",
            Kind::Force => "N",
        }
    }

    /// Each unit people write for it, as [`fold`] leaves it, and how many
    /// of [`Kind::unit`] one of it makes.
    fn units(self) -> &'static [(&'static str, f64)] {
        match self {
            Kind::Mass => &[("g", 1.0), ("kg", 1000.0), ("mg", 0.001), ("oz", 28.349_523_125)],
            Kind::Torque => &[
                ("mn·m", 1.0),
                ("mnm", 1.0),
                ("n·m", 1000.0),
                ("nm", 1000.0),
                ("n·cm", 10.0),
                ("ncm", 10.0),
                ("g·cm", 0.098_066_5),
                ("gcm", 0.098_066_5),
                ("gf·cm", 0.098_066_5),
                ("kg·cm", 98.066_5),
                ("kgcm", 98.066_5),
                ("kgf·cm", 98.066_5),
                ("oz·in", 7.061_552),
            ],
            Kind::Speed => &[("rpm", 1.0), ("tr/min", 1.0), ("r/min", 1.0), ("rps", 60.0)],
            Kind::Force => &[("n", 1.0), ("kn", 1000.0), ("kgf", 9.806_65), ("gf", 0.009_806_65), ("lbf", 4.448_222)],
        }
    }
}

/// The keys whose figure is a quantity, and its kind.
pub const KEYS: &[(&str, Kind)] = &[
    ("mass", Kind::Mass),
    ("torque", Kind::Torque),
    ("stall torque", Kind::Torque),
    ("holding torque", Kind::Torque),
    ("speed", Kind::Speed),
    ("max radial load", Kind::Force),
    ("max axial load", Kind::Force),
    ("dynamic load", Kind::Force),
    ("static load", Kind::Force),
];

/// The kind of figure a spec's key holds, if it is one of [`KEYS`].
pub fn kind_of(key: &str) -> Option<Kind> {
    let key = key.trim().to_lowercase();
    KEYS.iter().find(|(k, _)| *k == key).map(|&(_, kind)| kind)
}

/// The figure of `spec` in its kind's unit, or why it cannot be read;
/// `None` for a key that holds no quantity.
pub fn figure(spec: &Spec) -> Option<Result<f64, String>> {
    let kind = kind_of(&spec.key)?;
    Some(read(&spec.value, kind).ok_or_else(|| {
        let units: Vec<&str> = kind.units().iter().map(|(u, _)| *u).collect();
        format!("`{}` takes a number and its unit, as `12 {}` ({})", spec.key, kind.unit(), units.join(", "))
    }))
}

/// The figure the specs give for `key`, in its kind's unit.
pub fn spec_figure(specs: &[Spec], key: &str) -> Option<f64> {
    specs.iter().find(|s| s.key.trim().eq_ignore_ascii_case(key)).and_then(|s| figure(s)?.ok())
}

/// The first number of `text` followed by a unit of `kind`, in its unit.
pub fn read(text: &str, kind: Kind) -> Option<f64> {
    let text = fold(text);
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let starts = chars[i].is_ascii_digit() || (chars[i] == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit));
        let after_word = i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '.');
        if !starts || after_word {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
            i += 1;
        }
        let number: f64 = match chars[start..i].iter().collect::<String>().parse() {
            Ok(n) => n,
            Err(_) => continue,
        };
        let mut j = i;
        while j < chars.len() && chars[j] == ' ' {
            j += 1;
        }
        let unit: String = chars[j..].iter().take_while(|c| !c.is_whitespace() && !matches!(c, ',' | ';' | ')' | '(')).collect();
        if let Some(&(_, scale)) = kind.units().iter().find(|(u, _)| *u == unit) {
            return Some(number * scale);
        }
    }
    None
}

/// Lowercase, the decimal comma as a point, and the ways to write a
/// product of units (`mN.m`, `mN*m`, `mN m`, `mN-m`) as `·`.
fn fold(text: &str) -> String {
    let lower = text.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    let mut out = String::with_capacity(lower.len());
    for (i, &c) in chars.iter().enumerate() {
        let prev = i.checked_sub(1).map(|p| chars[p]);
        let next = chars.get(i + 1).copied();
        let between_digits = prev.is_some_and(|p| p.is_ascii_digit()) && next.is_some_and(|n| n.is_ascii_digit());
        let between_letters = prev.is_some_and(char::is_alphabetic) && next.is_some_and(char::is_alphabetic);
        match c {
            ',' if between_digits => out.push('.'),
            '.' | '*' | '-' | '⋅' | '•' if between_letters => out.push('·'),
            ' ' if between_letters && unit_product(&chars, i) => out.push('·'),
            c => out.push(c),
        }
    }
    out
}

/// Whether the space at `i` joins two units, as in `mN m` or `kgf cm`,
/// rather than a unit and the next word.
fn unit_product(chars: &[char], i: usize) -> bool {
    let before: String = chars[..i].iter().rev().take_while(|c| c.is_alphabetic()).collect::<Vec<_>>().into_iter().rev().collect();
    let after: String = chars[i + 1..].iter().take_while(|c| c.is_alphabetic()).collect();
    matches!(before.as_str(), "mn" | "n" | "kgf" | "kg" | "gf" | "g" | "oz") && matches!(after.as_str(), "m" | "cm" | "in")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() < 1e-6 * b.abs().max(1.0))
    }

    #[test]
    fn figures_are_read_in_one_unit_per_kind() {
        assert!(close(read("3.3 g", Kind::Mass), 3.3));
        assert!(close(read("about 0,2 kg with its cable", Kind::Mass), 200.0));
        assert!(close(read("34 mN·m at 5 V", Kind::Torque), 34.0));
        assert!(close(read("34 mN.m", Kind::Torque), 34.0));
        assert!(close(read("34 mN m", Kind::Torque), 34.0));
        assert!(close(read("0.8 kgf·cm", Kind::Torque), 78.4532));
        assert!(close(read("300 gf.cm", Kind::Torque), 29.41995));
        assert!(close(read("0.034 N·m", Kind::Torque), 34.0));
        assert!(close(read("15 rpm at 5 V", Kind::Speed), 15.0));
        assert!(close(read("12 tr/min", Kind::Speed), 12.0));
        assert!(close(read("3.3 kN", Kind::Force), 3300.0));
        assert!(close(read("20 N", Kind::Force), 20.0));
        // The first number with a unit of the kind, not the voltage before it.
        assert!(close(read("at 5 V: 34 mN·m", Kind::Torque), 34.0));
        assert_eq!(read("34", Kind::Torque), None);
        assert_eq!(read("M3 screws", Kind::Mass), None);
    }

    #[test]
    fn a_known_key_must_hold_its_quantity() {
        let spec = |key: &str, value: &str| Spec { key: key.into(), value: value.into(), span: c2f_lang::Span::new(0, 0) };
        assert!(close(figure(&spec("Mass", "3.3 g")).unwrap().ok(), 3.3));
        assert!(figure(&spec("torque", "strong")).unwrap().is_err());
        assert!(figure(&spec("supply", "5 V")).is_none());
        let specs = [spec("mass", "12 g"), spec("stall torque", "0.3 N·m")];
        assert!(close(spec_figure(&specs, "stall torque"), 300.0));
        assert_eq!(spec_figure(&specs, "speed"), None);
    }
}
