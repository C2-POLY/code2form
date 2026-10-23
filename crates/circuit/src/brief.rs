//! A design's brief: what the plan decided before anything was drawn,
//! written in Markdown beside the design, for the user to read and approve
//! and for the agent to hold the design to. Its free text says the idea,
//! the sizes, the pieces, the components and the mechanical review; its
//! requirements are lines a tool can measure:
//!
//! ```markdown
//! # Watch winder
//!
//! ## Requirements
//!
//! - mass < 400 g
//! - tipping > 15°
//! - torque margin >= 2
//! - radial load margin >= 2
//! - size <= 220 x 220 x 250 mm
//! - part "Cradle"
//! - component "electronics/xiao_esp32s3.c2f"
//! - prints without supports
//! ```
//!
//! A requirement no tool can measure ("prints without supports") is kept
//! and reported as such. The section may be called `Exigences`, and
//! French words read as their English ones (`masse`, `basculement`,
//! `marge de couple`, `taille`, `pièce`, `composant`).
//!
//! What the tools compute from is written in fenced JSON blocks named
//! after them: ```` ```json mass ```` (the arguments of `mass_properties`:
//! material, infill, extra masses) and ```` ```json loads ```` (those of
//! `loads`: the shaft, its loads, its supports and its motor).

use crate::quantity::{Kind, read};

/// How a measure is compared with its bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Less,
    AtMost,
    More,
    AtLeast,
}

impl Op {
    pub fn holds(self, value: f64, bound: f64) -> bool {
        match self {
            Op::Less => value < bound,
            Op::AtMost => value <= bound,
            Op::More => value > bound,
            Op::AtLeast => value >= bound,
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Op::Less => "<",
            Op::AtMost => "<=",
            Op::More => ">",
            Op::AtLeast => ">=",
        }
    }
}

/// What a requirement asks.
#[derive(Clone, Debug, PartialEq)]
pub enum Rule {
    /// The total mass, in grams.
    Mass(Op, f64),
    /// The smallest tipping angle, in degrees.
    Tipping(Op, f64),
    /// The motor's torque over the torque needed.
    TorqueMargin(Op, f64),
    /// The motor's largest radial load over the load it carries.
    RadialLoadMargin(Op, f64),
    /// Each piece within this size, in millimeters (a printer's bed and
    /// height).
    Size([f64; 3]),
    /// The design has this `part()`.
    Part(String),
    /// The circuit uses this part file.
    Component(String),
    /// Read by the user, measured by no tool.
    Other,
}

/// A line of the requirements: its text, what it asks, and its line in
/// the brief (from 1).
#[derive(Clone, Debug, PartialEq)]
pub struct Requirement {
    pub text: String,
    pub rule: Rule,
    pub line: usize,
}

impl Requirement {
    /// Whether a tool can measure it.
    pub fn checkable(&self) -> bool {
        self.rule != Rule::Other
    }
}

/// A brief read: its title, its requirements and the tools' JSON blocks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Brief {
    pub title: Option<String>,
    pub requirements: Vec<Requirement>,
    /// The ```` ```json mass ```` block.
    pub mass: Option<serde_json::Value>,
    /// The ```` ```json loads ```` block.
    pub loads: Option<serde_json::Value>,
    /// What is wrong with it, with its line (from 1).
    pub errors: Vec<(usize, String)>,
}

/// Reads a brief's Markdown.
pub fn read_brief(text: &str) -> Brief {
    let mut brief = Brief::default();
    let mut in_requirements = false;
    let mut block: Option<(usize, String, String)> = None;
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let trimmed = line.trim();
        if let Some((start, name, body)) = &mut block {
            if trimmed.starts_with("```") {
                let (start, name, body) = (*start, std::mem::take(name), std::mem::take(body));
                block = None;
                match serde_json::from_str::<serde_json::Value>(&body) {
                    Ok(value) if value.is_object() => match name.as_str() {
                        "mass" => brief.mass = Some(value),
                        _ => brief.loads = Some(value),
                    },
                    Ok(_) => brief.errors.push((start, format!("the `{name}` block is a JSON object, as its tool's arguments"))),
                    Err(err) => brief.errors.push((start, format!("the `{name}` block is not JSON: {err}"))),
                }
            } else {
                body.push_str(line);
                body.push('\n');
            }
            continue;
        }
        if let Some(info) = trimmed.strip_prefix("```") {
            let mut words = info.split_whitespace();
            if words.next() == Some("json") {
                if let Some(name @ ("mass" | "loads")) = words.next() {
                    block = Some((number, name.to_string(), String::new()));
                    continue;
                }
            }
            continue;
        }
        if let Some(title) = trimmed.strip_prefix("# ") {
            brief.title.get_or_insert_with(|| title.trim().to_string());
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("## ") {
            let heading = fold(heading);
            in_requirements = heading.starts_with("requirement") || heading.starts_with("exigence");
            continue;
        }
        if in_requirements {
            let item = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* "));
            if let Some(item) = item {
                let item = item.trim_start_matches("[ ]").trim_start_matches("[x]").trim();
                brief.requirements.push(Requirement { text: item.to_string(), rule: rule(item), line: number });
            }
        }
    }
    if let Some((start, name, _)) = block {
        brief.errors.push((start, format!("the `{name}` block is not closed with ```")));
    }
    brief
}

/// What a requirement's text asks.
fn rule(text: &str) -> Rule {
    let folded = fold(text);
    let after = |names: &[&str]| names.iter().find_map(|name| folded.strip_prefix(name).map(str::trim));
    let quoted = || {
        let start = text.find('"')? + 1;
        let end = start + text[start..].find('"')?;
        Some(text[start..end].to_string())
    };
    if let Some(rest) = after(&["total mass", "masse totale", "mass", "masse"]) {
        if let Some((op, rest)) = op(rest) {
            if let Some(grams) = read(rest, Kind::Mass) {
                return Rule::Mass(op, grams);
            }
        }
    }
    if let Some(rest) = after(&["tipping angle", "angle de basculement", "tipping", "basculement"]) {
        if let Some((op, rest)) = op(rest) {
            if let Some(degrees) = number(rest.trim_end_matches(['°', ' ']).trim_end_matches("degrees").trim_end_matches("degres").trim_end_matches("deg").trim()) {
                return Rule::Tipping(op, degrees);
            }
        }
    }
    if let Some(rest) = after(&["torque margin", "marge de couple"]) {
        if let Some((op, rest)) = op(rest) {
            if let Some(x) = number(rest.trim_start_matches(['x', '×']).trim_end_matches(['x', '×']).trim()) {
                return Rule::TorqueMargin(op, x);
            }
        }
    }
    if let Some(rest) = after(&["radial load margin", "marge de charge radiale"]) {
        if let Some((op, rest)) = op(rest) {
            if let Some(x) = number(rest.trim_start_matches(['x', '×']).trim_end_matches(['x', '×']).trim()) {
                return Rule::RadialLoadMargin(op, x);
            }
        }
    }
    if let Some(rest) = after(&["size", "taille"]) {
        if let Some((Op::Less | Op::AtMost, rest)) = op(rest) {
            let rest = rest.trim_end_matches("mm").trim();
            let sides: Vec<Option<f64>> = rest.split(['x', '×']).map(|s| number(s.trim())).collect();
            if let [Some(x), Some(y), Some(z)] = sides[..] {
                return Rule::Size([x, y, z]);
            }
        }
    }
    if folded.starts_with("part ") || folded.starts_with("piece ") {
        if let Some(name) = quoted() {
            return Rule::Part(name);
        }
    }
    if folded.starts_with("component ") || folded.starts_with("composant ") {
        if let Some(file) = quoted() {
            return Rule::Component(file);
        }
    }
    Rule::Other
}

/// The comparison at the start of `text`, and what follows it.
fn op(text: &str) -> Option<(Op, &str)> {
    let text = text.trim_start_matches(':').trim();
    for (symbol, op) in [("<=", Op::AtMost), ("≤", Op::AtMost), (">=", Op::AtLeast), ("≥", Op::AtLeast), ("<", Op::Less), (">", Op::More)] {
        if let Some(rest) = text.strip_prefix(symbol) {
            return Some((op, rest.trim()));
        }
    }
    None
}

fn number(text: &str) -> Option<f64> {
    text.replace(',', ".").parse().ok().filter(|n: &f64| n.is_finite())
}

/// Lowercase, without the accents French requirements are written with.
fn fold(text: &str) -> String {
    text.trim()
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'à' | 'â' => 'a',
            'î' | 'ï' => 'i',
            'ô' => 'o',
            'û' | 'ù' => 'u',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BRIEF: &str = "\
# Watch winder

The watch turns on a cradle at the front of the shaft.

## Requirements

- mass < 400 g
- tipping > 15°
- torque margin >= 2
- radial load margin ≥ 2×
- size <= 220 x 220 x 250 mm
- part \"Cradle\"
- component \"electronics/xiao_esp32s3.c2f\"
- prints without supports

## Loads

```json loads
{ \"ratio\": 1, \"target_margin\": 2 }
```
";

    #[test]
    fn requirements_read_as_rules() {
        let brief = read_brief(BRIEF);
        assert_eq!(brief.title.as_deref(), Some("Watch winder"));
        assert!(brief.errors.is_empty(), "{:?}", brief.errors);
        let rules: Vec<&Rule> = brief.requirements.iter().map(|r| &r.rule).collect();
        assert_eq!(
            rules,
            [
                &Rule::Mass(Op::Less, 400.0),
                &Rule::Tipping(Op::More, 15.0),
                &Rule::TorqueMargin(Op::AtLeast, 2.0),
                &Rule::RadialLoadMargin(Op::AtLeast, 2.0),
                &Rule::Size([220.0, 220.0, 250.0]),
                &Rule::Part("Cradle".into()),
                &Rule::Component("electronics/xiao_esp32s3.c2f".into()),
                &Rule::Other,
            ]
        );
        assert_eq!(brief.requirements[0].line, 7);
        assert_eq!(brief.loads.as_ref().unwrap()["ratio"], 1);
        assert!(brief.mass.is_none());
    }

    #[test]
    fn french_requirements_read_the_same() {
        let brief = read_brief("# Remontoir\n## Exigences\n- masse totale < 0,4 kg\n- basculement > 15 degrés\n- marge de couple ≥ 2\n- pièce \"Berceau\"\n");
        let rules: Vec<&Rule> = brief.requirements.iter().map(|r| &r.rule).collect();
        assert_eq!(rules[0], &Rule::Mass(Op::Less, 400.0));
        assert_eq!(rules[2], &Rule::TorqueMargin(Op::AtLeast, 2.0));
        assert_eq!(rules[1], &Rule::Tipping(Op::More, 15.0));
        assert_eq!(rules[3], &Rule::Part("Berceau".into()));
    }

    #[test]
    fn a_broken_block_is_an_error_with_its_line() {
        let brief = read_brief("# A\n```json mass\n{ material: PLA }\n```\n```json loads\n[1]\n```\n");
        assert_eq!(brief.errors.len(), 2, "{:?}", brief.errors);
        assert_eq!((brief.errors[0].0, brief.errors[1].0), (2, 5));
        assert!(read_brief("# A\n```json loads\n{}\n").errors[0].1.contains("not closed"));
    }
}
