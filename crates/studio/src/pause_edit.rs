//! The changes the layer view makes to the source for the print's pauses:
//! a new `pause()` call, a height moved, a call removed. Each returns the
//! new text and where to put the cursor (a char index), or why it cannot.

use std::ops::Range;

/// A height as the source writes it: at most three decimals, no trailing
/// zeros (`18.4`, `3`).
pub fn number(z: f32) -> String {
    let text = format!("{:.3}", z);
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".to_string() } else { text.to_string() }
}

/// Adds `pause(z);` on a line of its own at the end of the source.
pub fn add(src: &str, z: f32) -> (String, usize) {
    let mut out = src.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("pause({});\n", number(z)));
    let cursor = out.chars().count() - 1;
    (out, cursor)
}

/// Writes `z` in place of the height written as a number at `range`.
pub fn set_height(src: &str, range: Range<usize>, z: f32) -> Result<(String, usize), String> {
    let written = src.get(range.clone()).ok_or("the source changed")?;
    if written.parse::<f64>().is_err() {
        return Err("the source changed".into());
    }
    let new = number(z);
    let cursor = src[..range.start].chars().count() + new.chars().count();
    Ok((format!("{}{new}{}", &src[..range.start], &src[range.end..]), cursor))
}

/// Removes the `pause(...);` statement whose name starts at byte `call`,
/// with its line when it stands alone on it.
pub fn remove(src: &str, call: usize) -> Result<(String, usize), String> {
    let end = statement_end(src, call).ok_or("the source changed")?;
    if !after_statement(&src[..call]) {
        return Err("this pause is the body of an `if` or a loop: remove it in the code".into());
    }
    let line_start = src[..call].rfind('\n').map_or(0, |i| i + 1);
    let line_end = src[end..].find('\n').map_or(src.len(), |i| end + i + 1);
    let alone = src[line_start..call].trim().is_empty() && src[end..line_end].trim().is_empty();
    let (from, to) = if alone {
        (line_start, line_end)
    } else {
        (call, end + (src[end..].len() - src[end..].trim_start_matches([' ', '\t']).len()))
    };
    let cursor = src[..from].chars().count();
    Ok((format!("{}{}", &src[..from], &src[to..]), cursor))
}

/// The byte after `pause(...)` and its `;`, or `None` if `call` does not
/// start such a call.
fn statement_end(src: &str, call: usize) -> Option<usize> {
    let rest = src.get(call..)?.strip_prefix("pause")?;
    let open = call + 5 + (rest.len() - rest.trim_start().len());
    if src.as_bytes().get(open) != Some(&b'(') {
        return None;
    }
    let bytes = src.as_bytes();
    let (mut depth, mut i) = (0usize, open);
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                i = src[i..].find('\n').map_or(bytes.len(), |n| i + n);
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = src[i + 2..].find("*/").map_or(bytes.len(), |n| i + 2 + n + 1);
            }
            _ => {}
        }
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let after = i + 1;
    let rest = &src[after..];
    let spaces = rest.len() - rest.trim_start_matches([' ', '\t']).len();
    Some(if rest[spaces..].starts_with(';') { after + spaces + 1 } else { after })
}

/// Whether what comes before a call ends a statement or opens a block, so
/// that the call is a statement of its own and not the body of an `if`,
/// `for` or module call. Line comments are skipped.
fn after_statement(before: &str) -> bool {
    for line in before.lines().rev() {
        let code = line.find("//").map_or(line, |i| &line[..i]).trim_end();
        if let Some(last) = code.chars().last() {
            return matches!(last, ';' | '{' | '}');
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_short() {
        assert_eq!(number(18.400_002), "18.4");
        assert_eq!(number(3.0), "3");
        assert_eq!(number(0.25), "0.25");
        assert_eq!(number(-0.0001), "0");
    }

    #[test]
    fn a_pause_goes_on_its_own_line_at_the_end() {
        assert_eq!(add("cube(10);", 3.2).0, "cube(10);\npause(3.2);\n");
        let (text, cursor) = add("cube(10);\n", 3.0);
        assert_eq!(text, "cube(10);\npause(3);\n");
        assert_eq!(text.chars().nth(cursor), Some('\n'));
        assert_eq!(add("", 1.0).0, "pause(1);\n");
    }

    #[test]
    fn a_height_is_rewritten_in_place() {
        let src = "pause(3.2, \"Lid\");";
        assert_eq!(set_height(src, 6..9, 4.4).unwrap().0, "pause(4.4, \"Lid\");");
        assert!(set_height(src, 6..12, 4.4).is_err(), "not a number");
        assert!(set_height(src, 6..99, 4.4).is_err());
    }

    #[test]
    fn a_pause_alone_on_its_line_goes_with_it() {
        let src = "cube(10);\n  pause(3, \"a ) ; b\"); // magnets\nsphere(2);\n";
        let call = src.find("pause").unwrap();
        // A comment after the call stays with the line it is on.
        assert_eq!(remove(src, call).unwrap().0, "cube(10);\n  // magnets\nsphere(2);\n");
        let src = "cube(10);\n    pause(3);\nsphere(2);\n";
        assert_eq!(remove(src, src.find("pause").unwrap()).unwrap().0, "cube(10);\nsphere(2);\n");
        let src = "cube(10); pause(3);  sphere(2);";
        assert_eq!(remove(src, src.find("pause").unwrap()).unwrap().0, "cube(10); sphere(2);");
        let src = "module m() {\n  // lid\n  pause(1)\n}\n";
        assert_eq!(remove(src, src.find("pause").unwrap()).unwrap().0, "module m() {\n  // lid\n}\n");
    }

    #[test]
    fn a_pause_inside_a_statement_stays() {
        let src = "if (lid)\n  pause(3);\n";
        assert!(remove(src, src.find("pause").unwrap()).is_err());
        let src = "for (z = [1, 2]) pause(z);";
        assert!(remove(src, src.find("pause").unwrap()).is_err());
        // Offsets from an older version of the source.
        assert!(remove("cube(1);", 0).is_err());
        assert!(remove("pause(1", 0).is_err());
    }
}
