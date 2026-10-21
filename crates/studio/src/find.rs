//! Find and replace in the editor's text: plain-text matches, the current
//! one, and the highlights painted behind them.

use std::ops::Range;

use eframe::egui::Color32;
use eframe::egui::text::LayoutJob;

/// The byte ranges where `query` occurs in `text`, left to right and not
/// overlapping. Without `case_sensitive`, letters match whatever their case.
pub fn find_all(text: &str, query: &str, case_sensitive: bool) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    if case_sensitive {
        return text.match_indices(query).map(|(i, m)| i..i + m.len()).collect();
    }
    let query: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let mut matches = Vec::new();
    let mut from = 0;
    for (start, _) in text.char_indices() {
        if start < from {
            continue;
        }
        if let Some(end) = match_at(text, start, &query) {
            matches.push(start..end);
            from = end;
        }
    }
    matches
}

/// Where a case-insensitive match of the lowercased `query` starting at
/// `start` ends, if there is one.
fn match_at(text: &str, start: usize, query: &[char]) -> Option<usize> {
    let mut at = 0;
    for (i, c) in text[start..].char_indices() {
        for lower in c.to_lowercase() {
            if query.get(at) != Some(&lower) {
                return None;
            }
            at += 1;
        }
        if at == query.len() {
            return Some(start + i + c.len_utf8());
        }
    }
    None
}

/// The first match at or after `byte`, or the first one when none is.
pub fn match_from(matches: &[Range<usize>], byte: usize) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    Some(matches.iter().position(|m| m.start >= byte).unwrap_or(0))
}

/// The match after (or before) `current`, wrapping around the ends.
pub fn step(count: usize, current: Option<usize>, forward: bool) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(match (current, forward) {
        (None, true) => 0,
        (None, false) => count - 1,
        (Some(i), true) => (i + 1) % count,
        (Some(i), false) => (i + count - 1) % count,
    })
}

/// `text` with every match replaced by `replacement`.
pub fn replace_all(text: &str, matches: &[Range<usize>], replacement: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for m in matches {
        out.push_str(&text[at..m.start]);
        out.push_str(replacement);
        at = m.end;
    }
    out.push_str(&text[at..]);
    out
}

/// The bar's count: "3 of 12", or "No results".
pub fn count_label(count: usize, current: Option<usize>) -> String {
    match (count, current) {
        (0, _) => "No results".to_string(),
        (n, Some(i)) => format!("{} of {n}", i + 1),
        (n, None) => format!("{n} found"),
    }
}

/// Gives the matches in a laid-out text a background, `current_color` for
/// the current one, splitting sections where a match starts or ends.
pub fn mark(job: &mut LayoutJob, matches: &[Range<usize>], current: Option<usize>, color: Color32, current_color: Color32) {
    if matches.is_empty() {
        return;
    }
    let mut sections = Vec::with_capacity(job.sections.len() + 2 * matches.len());
    let mut next = 0;
    for section in job.sections.drain(..) {
        let mut at = section.byte_range.start;
        let end = section.byte_range.end;
        while at < end {
            while next < matches.len() && matches[next].end <= at {
                next += 1;
            }
            let (stop, background) = match matches.get(next) {
                Some(m) if m.start <= at => {
                    let color = if current == Some(next) { current_color } else { color };
                    (m.end.min(end), color)
                }
                Some(m) => (m.start.min(end), section.format.background),
                None => (end, section.format.background),
            };
            let mut piece = section.clone();
            piece.leading_space = if at == section.byte_range.start { section.leading_space } else { 0.0 };
            piece.byte_range = at..stop;
            piece.format.background = background;
            sections.push(piece);
            at = stop;
        }
        if section.byte_range.is_empty() {
            sections.push(section);
        }
    }
    job.sections = sections;
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::FontId;
    use eframe::egui::text::TextFormat;

    #[test]
    fn finds_every_occurrence() {
        assert_eq!(find_all("cube(); cube();", "cube", true), vec![0..4, 8..12]);
        assert_eq!(find_all("aaaa", "aa", true), vec![0..2, 2..4], "matches do not overlap");
        assert!(find_all("cube", "", true).is_empty());
        assert!(find_all("cube", "sphere", true).is_empty());
    }

    #[test]
    fn case_sensitivity() {
        let text = "Cube cube CUBE";
        assert_eq!(find_all(text, "cube", true), vec![5..9]);
        assert_eq!(find_all(text, "cube", false), vec![0..4, 5..9, 10..14]);
        assert_eq!(find_all(text, "CuBe", false).len(), 3);
        // Byte ranges stay on character boundaries past non-ASCII text.
        assert_eq!(find_all("é = É;", "é", false), vec![0..2, 5..7]);
    }

    #[test]
    fn steps_wrap_around() {
        assert_eq!(step(3, Some(2), true), Some(0));
        assert_eq!(step(3, Some(0), false), Some(2));
        assert_eq!(step(3, Some(1), true), Some(2));
        assert_eq!(step(3, None, true), Some(0));
        assert_eq!(step(3, None, false), Some(2));
        assert_eq!(step(0, Some(1), true), None);
    }

    #[test]
    fn starts_from_the_cursor() {
        let matches = [0..4, 8..12];
        assert_eq!(match_from(&matches, 0), Some(0));
        assert_eq!(match_from(&matches, 5), Some(1));
        assert_eq!(match_from(&matches, 13), Some(0), "wraps to the first");
        assert_eq!(match_from(&[], 0), None);
    }

    #[test]
    fn replaces_all_matches() {
        let text = "r = 1; f(r); // r";
        let matches = find_all(text, "r", true);
        assert_eq!(replace_all(text, &matches, "radius"), "radius = 1; f(radius); // radius");
        assert_eq!(replace_all(text, &[], "x"), text);
    }

    #[test]
    fn counts() {
        assert_eq!(count_label(12, Some(2)), "3 of 12");
        assert_eq!(count_label(0, None), "No results");
    }

    #[test]
    fn marks_split_sections_at_matches() {
        let font = FontId::monospace(12.0);
        let mut job = LayoutJob::default();
        job.append("cube(", 0.0, TextFormat::simple(font.clone(), Color32::WHITE));
        job.append("1); cube", 0.0, TextFormat::simple(font, Color32::WHITE));
        let (match_bg, current_bg) = (Color32::RED, Color32::GREEN);
        let matches = find_all(&job.text, "e(1", true);
        mark(&mut job, &matches, None, match_bg, current_bg);
        let pieces: Vec<(&str, Color32)> =
            job.sections.iter().map(|s| (&job.text[s.byte_range.clone()], s.format.background)).collect();
        assert_eq!(
            pieces,
            vec![
                ("cub", Color32::TRANSPARENT),
                ("e(", match_bg),
                ("1", match_bg),
                ("); cube", Color32::TRANSPARENT),
            ]
        );

        let mut job = LayoutJob::default();
        job.append("ab ab ab", 0.0, TextFormat::simple(FontId::monospace(12.0), Color32::WHITE));
        mark(&mut job, &find_all("ab ab ab", "ab", true), Some(1), match_bg, current_bg);
        let backgrounds: Vec<Color32> = job.sections.iter().map(|s| s.format.background).collect();
        assert_eq!(
            backgrounds,
            vec![match_bg, Color32::TRANSPARENT, current_bg, Color32::TRANSPARENT, match_bg]
        );
    }
}
