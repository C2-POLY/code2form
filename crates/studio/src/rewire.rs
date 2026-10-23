//! Rewiring with the mouse: a wire's end moved to another pin, a new wire
//! from pin to pin, a wire removed. Each gesture becomes an edit of the
//! circuit file's text, by the spans the circuit was read with, so that
//! the file keeps its comments, its order and its formatting; and the
//! wiring checks run on the circuit as the edit would leave it, to warn
//! before the drop.

use std::collections::HashSet;

use c2f_circuit::check::{self, Severity};
use c2f_circuit::{Circuit, End, Wire};

use crate::i18n::tr;

/// One end of a wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    From,
    To,
}

/// A change to the wiring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Wire `wire`'s `side` goes to pin `to`.
    Move {
        wire: usize,
        side: Side,
        to: End,
    },
    /// A new wire, last in the mount order.
    Add {
        from: End,
        to: End,
    },
    Remove {
        wire: usize,
    },
}

/// The circuit as `edit` would leave it.
pub fn edited(circuit: &Circuit, edit: Edit) -> Circuit {
    let mut circuit = circuit.clone();
    match edit {
        Edit::Move { wire, side: Side::From, to } => circuit.wires[wire].from = to,
        Edit::Move { wire, side: Side::To, to } => circuit.wires[wire].to = to,
        Edit::Add { from, to } => circuit.wires.push(Wire {
            from,
            to,
            color: None,
            label: None,
            allow: Vec::new(),
            span: Default::default(),
            to_span: Default::default(),
        }),
        Edit::Remove { wire } => {
            circuit.wires.remove(wire);
        }
    }
    circuit
}

/// The first error of the wiring checks that `edit` would bring, if any:
/// errors the circuit has already are not its doing.
pub fn new_error(circuit: &Circuit, edit: Edit) -> Option<String> {
    let errors = |c: &Circuit| -> Vec<String> {
        check::check(c).into_iter().filter(|f| f.severity == Severity::Error).map(|f| f.message).collect()
    };
    let before: HashSet<String> = errors(circuit).into_iter().collect();
    errors(&edited(circuit, edit)).into_iter().find(|message| !before.contains(message))
}

/// `"part.pin"`, as a circuit file writes an end.
fn quoted(circuit: &Circuit, end: End) -> String {
    format!("\"{}\"", circuit.end_name(end))
}

/// The text of the circuit file `src`, which `circuit` was read from, with
/// `edit` made; and where to put the cursor, as a char index.
pub fn apply(src: &str, circuit: &Circuit, edit: Edit) -> Result<(String, usize), String> {
    let changed = || {
        tr(
            "the circuit changed since it was read: try again once it is rendered",
            "le circuit a changé depuis sa lecture : réessayez une fois le rendu fini",
        )
        .to_string()
    };
    match edit {
        Edit::Move { wire, side, to } => {
            let w = circuit.wires.get(wire).ok_or_else(changed)?;
            let (span, was) = match side {
                Side::From => (w.span, w.from),
                Side::To => (w.to_span, w.to),
            };
            let range = span.start as usize..span.end as usize;
            // The value must still be the end the circuit read there.
            let written = src.get(range.clone()).ok_or_else(changed)?;
            if written.trim_matches(['"', '\'']) != circuit.end_name(was) {
                return Err(changed());
            }
            let new = quoted(circuit, to);
            let cursor = src[..range.start].chars().count() + new.chars().count();
            Ok((format!("{}{new}{}", &src[..range.start], &src[range.end..]), cursor))
        }
        Edit::Add { from, to } => {
            let mut out = src.to_string();
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            if !out.is_empty() && !out.ends_with("\n\n") {
                out.push('\n');
            }
            out.push_str(&format!("[[wires]]\nfrom = {}\nto = {}\n", quoted(circuit, from), quoted(circuit, to)));
            let cursor = out.chars().count() - 1;
            Ok((out, cursor))
        }
        Edit::Remove { wire } => {
            let w = circuit.wires.get(wire).ok_or_else(changed)?;
            let range = table(src, w.span.start as usize).ok_or_else(changed)?;
            let cursor = src[..range.start].chars().count();
            Ok((format!("{}{}", &src[..range.start], &src[range.end..]), cursor))
        }
    }
}

/// The bytes a `[[wires]]` table takes, from the blank line before its
/// header (one, when there is one) to the end of its last key: comments
/// and blank lines after it, before the next table, are left, as they
/// tell of what comes next. `inside` is a byte of the table.
fn table(src: &str, inside: usize) -> Option<std::ops::Range<usize>> {
    table_of(src, inside, |header| header == "[[wires]]")
}

/// [`table`] for any table whose header line, trimmed, `header` accepts.
pub fn table_of(src: &str, inside: usize, header_is: impl Fn(&str) -> bool) -> Option<std::ops::Range<usize>> {
    // Lines, with where each starts.
    let mut starts = vec![0];
    starts.extend(src.match_indices('\n').map(|(i, _)| i + 1).filter(|&i| i < src.len()));
    let line_of = |byte: usize| starts.iter().rposition(|&s| s <= byte).unwrap_or(0);
    let line = |i: usize| {
        let end = starts.get(i + 1).copied().unwrap_or(src.len());
        &src[starts[i]..end]
    };
    let is_header = |i: usize| line(i).trim_start().starts_with('[');
    let at = line_of(inside.min(src.len().saturating_sub(1)));
    let header = (0..=at).rev().find(|&i| is_header(i))?;
    if !header_is(line(header).trim()) {
        return None;
    }
    let next = (header + 1..starts.len()).find(|&i| is_header(i)).unwrap_or(starts.len());
    // The last line of the table that is a key, not a comment or blank.
    let last = (header..next).rev().find(|&i| {
        let text = line(i).trim();
        !text.is_empty() && !text.starts_with('#')
    })?;
    let end = starts.get(last + 1).copied().unwrap_or(src.len());
    let blank_before = header > 0 && line(header - 1).trim().is_empty();
    let start = if blank_before { starts[header - 1] } else { starts[header] };
    Some(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wiring::tests::circuit;

    const SRC: &str = r#"
[parts.a]
part = "board.c2f"

[parts.b]
part = "board.c2f"
at = [0, 30, 0]

[[wires]]
from = "a.A"
to = "b.A"
color = "red"
label = "5 V"

[[wires]]
from = "a.A"   # the first pin
to = "b.B"
color = "Red"
label = "5 V"

# Signals, from here on.

[[wires]]
from = "a.B"
to = "b.B"
"#;

    /// The test circuit, read from `SRC`, its spans pointing into it.
    fn read() -> Circuit {
        let path = circuit().parts[0].path.with_file_name("test.c2fc");
        c2f_circuit::parse(SRC, &path, &[]).unwrap()
    }

    fn end(part: usize, pin: usize) -> End {
        End { part, pin }
    }

    #[test]
    fn moving_an_end_rewrites_its_value_only() {
        let c = read();
        let (text, cursor) = apply(SRC, &c, Edit::Move { wire: 1, side: Side::From, to: end(0, 2) }).unwrap();
        assert_eq!(text, SRC.replace("from = \"a.A\"   # the first pin", "from = \"a.C\"   # the first pin"));
        assert_eq!(&text[..cursor], &text[..text.find("a.C\"").unwrap() + 4]);
        let (text, _) = apply(SRC, &c, Edit::Move { wire: 2, side: Side::To, to: end(1, 2) }).unwrap();
        assert!(text.ends_with("[[wires]]\nfrom = \"a.B\"\nto = \"b.C\"\n"), "{text}");
        // Read again, the move is there.
        let again = c2f_circuit::parse(&text, &c.parts[0].path.with_file_name("test.c2fc"), &[]).unwrap();
        assert_eq!(again.wires[2].to, end(1, 2));
        // A text that changed since is not edited.
        let older = SRC.replacen("a.A", "a.B", 1);
        assert!(apply(&older, &c, Edit::Move { wire: 0, side: Side::From, to: end(0, 2) }).is_err());
    }

    #[test]
    fn a_new_wire_comes_last_after_a_blank_line() {
        let c = read();
        let (text, _) = apply(SRC, &c, Edit::Add { from: end(0, 2), to: end(1, 2) }).unwrap();
        assert_eq!(text, format!("{SRC}\n[[wires]]\nfrom = \"a.C\"\nto = \"b.C\"\n"));
        let (text, _) = apply("x = 1", &c, Edit::Add { from: end(0, 2), to: end(1, 2) }).unwrap();
        assert_eq!(text, "x = 1\n\n[[wires]]\nfrom = \"a.C\"\nto = \"b.C\"\n");
    }

    #[test]
    fn removing_a_wire_takes_its_table_and_leaves_the_comments() {
        let c = read();
        let (text, _) = apply(SRC, &c, Edit::Remove { wire: 1 }).unwrap();
        assert_eq!(
            text,
            SRC.replace(
                "\n[[wires]]\nfrom = \"a.A\"   # the first pin\nto = \"b.B\"\ncolor = \"Red\"\nlabel = \"5 V\"\n",
                ""
            )
        );
        assert!(text.contains("label = \"5 V\"\n\n# Signals, from here on.\n\n[[wires]]\nfrom = \"a.B\""), "{text}");
        // The last one, to the end of the file.
        let (text, _) = apply(SRC, &c, Edit::Remove { wire: 2 }).unwrap();
        assert!(text.ends_with("# Signals, from here on.\n"), "{text}");
        let again = c2f_circuit::parse(&text, &c.parts[0].path.with_file_name("test.c2fc"), &[]).unwrap();
        assert_eq!(again.wires.len(), 2);
    }

    #[test]
    fn the_checks_run_on_the_circuit_as_the_drop_would_leave_it() {
        let dir = std::env::temp_dir().join(format!("code2form-rewire-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("supply.c2f"),
            "// @pin V5 [0, 0, 0] power_out 5V\n// @pin GND [1, 0, 0] ground\ncube(1);\n",
        )
        .unwrap();
        std::fs::write(dir.join("chip.c2f"), "// @pin IO [0, 0, 0] io 3.3V\n// @pin GND [1, 0, 0] ground\ncube(1);\n")
            .unwrap();
        let src = "[parts.s]\npart = \"supply.c2f\"\n\n[parts.c]\npart = \"chip.c2f\"\n\n[[wires]]\nfrom = \"s.GND\"\nto = \"c.GND\"\n";
        let c = c2f_circuit::parse(src, &dir.join("test.c2fc"), &[]).unwrap();
        let (supply, chip) = (|pin| End { part: 0, pin }, |pin| End { part: 1, pin });
        // 5 V onto a 3.3 V pin is an error; ground to ground is not.
        let error = new_error(&c, Edit::Add { from: supply(0), to: chip(0) });
        assert!(error.as_deref().is_some_and(|e| e.contains("3.3 V")), "{error:?}");
        assert_eq!(new_error(&c, Edit::Move { wire: 0, side: Side::To, to: chip(1) }), None);
        assert_eq!(new_error(&c, Edit::Remove { wire: 0 }), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
