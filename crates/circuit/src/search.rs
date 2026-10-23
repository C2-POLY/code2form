//! The parts catalogue's search, shared by the app and the MCP server: a
//! query in plain words, French or English, ranks the parts that answer
//! it.
//!
//! A part is read as words: its name and file, its description, its specs
//! and its pins' names, labels and types. Case and accents do not count,
//! nor a plural's `s`; `3,3 V` reads as `3.3v`. A word a part has under
//! another name is found by its synonyms (`moteur` is `motor`, `pas à pas`
//! is `stepper`, `micro` is `microphone`), a word's start finds the longer
//! words it begins (`ampl` finds `amplifier`). Every word of the query
//! must be found, so that a part answers all of it or not at all; a word
//! found in the name weighs more than in the description, then the specs,
//! then the pins.

use c2f_lang::annotations::Annotations;

/// What each field weighs in a part's score.
const NAME: f64 = 4.0;
const DESCRIPTION: f64 = 2.0;
const SPEC: f64 = 1.5;
const PIN: f64 = 1.0;

/// A word found only by its start weighs this much of a whole word.
const PREFIX: f64 = 0.7;

/// The shortest query word that finds the words it begins.
const MIN_PREFIX: usize = 3;

/// What a search reads of a part: its words, each with the weight of the
/// field it is in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    words: Vec<(String, f64)>,
}

impl Document {
    /// The words of a part named `name` (its file's stem) in the file
    /// `file`, with its `description` (its `@description`, or the opening
    /// comment of a file without one) and its annotations.
    pub fn new(name: &str, file: &str, description: &str, annotations: &Annotations) -> Document {
        let mut document = Document::default();
        document.add(name, NAME);
        document.add(file, NAME);
        document.add(description, DESCRIPTION);
        for spec in &annotations.specs {
            document.add(&spec.key, SPEC);
            document.add(&spec.value, SPEC);
        }
        for pin in &annotations.pins {
            document.add(&pin.name, PIN);
            document.add(&pin.kind.name().replace('_', " "), PIN);
            if let Some(label) = &pin.label {
                document.add(label, PIN);
            }
            if let Some(volts) = pin.voltage {
                document.add(&format!("{volts}V"), PIN);
            }
        }
        document
    }

    fn add(&mut self, text: &str, weight: f64) {
        self.words.extend(words(text).into_iter().map(|word| (word, weight)));
    }

    /// How well the part answers the query's words, none when a word is
    /// missing: for each word, the weight of the best field it is in, and
    /// a little for each time it is found, which tells apart parts that
    /// mention it once or all along.
    fn score(&self, query: &[String]) -> Option<f64> {
        let mut total = 0.0;
        for wanted in query {
            let mut best: f64 = 0.0;
            let mut found = 0;
            for (word, weight) in &self.words {
                let fit = if word == wanted {
                    1.0
                } else if wanted.len() >= MIN_PREFIX && word.starts_with(wanted.as_str()) {
                    PREFIX
                } else {
                    continue;
                };
                best = best.max(fit * weight);
                found += 1;
            }
            if found == 0 {
                return None;
            }
            total += best + 0.05 * f64::from(found.min(10));
        }
        Some(total)
    }
}

/// The parts that answer `query`, best first, as their index in
/// `documents` and their score; parts that score the same keep their
/// order. A query without words lists every part, in order, at 0.
pub fn rank(documents: &[Document], query: &str) -> Vec<(usize, f64)> {
    let mut query = words(query);
    let mut seen = std::collections::HashSet::new();
    query.retain(|word| seen.insert(word.clone()));
    let mut ranked: Vec<(usize, f64)> = documents
        .iter()
        .enumerate()
        .filter_map(|(index, document)| Some((index, document.score(&query)?)))
        .collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    ranked
}

/// Words of more than one token, replaced by the one they mean before the
/// tokens are looked up on their own.
const PHRASES: &[(&[&str], &str)] = &[
    (&["pas", "a", "pas"], "stepper"),
    (&["haut", "parleur"], "speaker"),
    (&["haut", "parleurs"], "speaker"),
    (&["pont", "en", "h"], "hbridge"),
    (&["h", "bridge"], "hbridge"),
    (&["h", "bridges"], "hbridge"),
    (&["wi", "fi"], "wifi"),
];

/// Each word and the one it is searched as: French and English words of
/// the domain, and the short forms people type. Words are without accents
/// or a plural's `s`, as [`words`] leaves them.
const SYNONYMS: &[(&str, &str)] = &[
    ("temperature", "temperature"),
    ("temp", "temperature"),
    ("moteur", "motor"),
    ("step", "stepper"),
    ("stepping", "stepper"),
    ("capteur", "sensor"),
    ("detecteur", "sensor"),
    ("micro", "microphone"),
    ("mic", "microphone"),
    ("loudspeaker", "speaker"),
    ("enceinte", "speaker"),
    ("ampli", "amplifier"),
    ("amplificateur", "amplifier"),
    ("amp", "amplifier"),
    ("carte", "board"),
    ("condensateur", "capacitor"),
    ("capa", "capacitor"),
    ("tactile", "touch"),
    ("toucher", "touch"),
    ("pilote", "driver"),
    ("alimentation", "supply"),
    ("alim", "supply"),
    ("power", "supply"),
    ("masse", "ground"),
    ("gnd", "ground"),
    ("bouton", "button"),
    ("batterie", "battery"),
    ("pile", "battery"),
    ("accu", "battery"),
    ("ecran", "display"),
    ("afficheur", "display"),
    ("screen", "display"),
    ("son", "audio"),
    ("sound", "audio"),
    ("tension", "voltage"),
    ("courant", "current"),
    ("microcontroleur", "microcontroller"),
    ("mcu", "microcontroller"),
    ("connecteur", "connector"),
    ("antenne", "antenna"),
    ("trou", "hole"),
    ("broche", "pin"),
    ("resistance", "resistor"),
    ("lumiere", "light"),
    ("sortie", "output"),
    ("entree", "input"),
    ("taille", "size"),
    ("dimension", "size"),
    ("fil", "wire"),
    ("cable", "wire"),
    ("cuivre", "copper"),
    ("plaque", "plate"),
];

/// Words that say nothing of a part.
const STOP_WORDS: &[&str] = &[
    "a", "an", "the", "of", "for", "with", "and", "or", "to", "in", "on", "by", "de", "du", "des", "d", "l", "le",
    "la", "les", "un", "une", "pour", "avec", "et", "ou", "en", "au", "aux", "sur", "par",
];

/// Units glued to the number before them: `5 V` is the word `5v`.
const UNITS: &[&str] = &["v", "mv", "a", "ma", "w", "mw", "mm", "khz", "mhz", "ghz", "db", "ohm"];

/// The words of `text` as a search compares them: lowercase and without
/// accents, a number and its unit as one word, each phrase and synonym
/// replaced by the word it means, and the words that say nothing left out.
pub fn words(text: &str) -> Vec<String> {
    let mut tokens = tokens(&fold(text));
    // A number and its unit.
    let mut i = 0;
    while i + 1 < tokens.len() {
        if tokens[i].starts_with(|c: char| c.is_ascii_digit())
            && tokens[i].chars().all(|c| c.is_ascii_digit() || c == '.')
            && UNITS.contains(&tokens[i + 1].as_str())
        {
            let unit = tokens.remove(i + 1);
            tokens[i].push_str(&unit);
        }
        i += 1;
    }
    let mut words = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let rest = &tokens[i..];
        let starts = |phrase: &[&str]| phrase.len() <= rest.len() && phrase.iter().zip(rest).all(|(a, b)| a == b);
        if let Some((phrase, meaning)) = PHRASES.iter().find(|(phrase, _)| starts(phrase)) {
            words.push(meaning.to_string());
            i += phrase.len();
            continue;
        }
        let token = singular(&tokens[i]);
        i += 1;
        if STOP_WORDS.contains(&token) {
            continue;
        }
        let word = SYNONYMS.iter().find(|(word, _)| *word == token).map_or(token, |(_, meaning)| meaning);
        words.push(word.to_string());
    }
    words
}

/// A word without the `s` of its plural: `moteurs` and `pins` are found as
/// `moteur` and `pin`. Short words keep theirs (`bus`, `i2s`), and so do
/// words in `ss` (`class`).
fn singular(word: &str) -> &str {
    match word.strip_suffix('s') {
        Some(stem) if word.len() > 3 && !stem.ends_with('s') && word.chars().all(|c| c.is_ascii_alphabetic()) => stem,
        _ => word,
    }
}

/// The text in lowercase, its accented letters as plain ones, and a
/// decimal comma between digits as a point.
fn fold(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (i, &c) in chars.iter().enumerate() {
        let between_digits = i > 0 && chars[i - 1].is_ascii_digit() && chars.get(i + 1).is_some_and(char::is_ascii_digit);
        if c == ',' && between_digits {
            out.push('.');
            continue;
        }
        for c in c.to_lowercase() {
            match c {
                'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' => out.push('a'),
                'é' | 'è' | 'ê' | 'ë' => out.push('e'),
                'í' | 'ì' | 'î' | 'ï' => out.push('i'),
                'ó' | 'ò' | 'ô' | 'ö' | 'õ' => out.push('o'),
                'ú' | 'ù' | 'û' | 'ü' => out.push('u'),
                'ç' => out.push('c'),
                'ñ' => out.push('n'),
                'ÿ' => out.push('y'),
                'œ' => out.push_str("oe"),
                'æ' => out.push_str("ae"),
                c => out.push(c),
            }
        }
    }
    out
}

/// The runs of letters and digits, a point between two digits kept in the
/// number.
fn tokens(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut current = String::new();
    for (i, &c) in chars.iter().enumerate() {
        let decimal = c == '.'
            && i > 0
            && chars[i - 1].is_ascii_digit()
            && chars.get(i + 1).is_some_and(char::is_ascii_digit);
        if c.is_alphanumeric() || decimal {
            current.push(c);
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn words_ignore_case_accents_plurals_and_stop_words() {
        assert_eq!(words("Capteur de TEMPÉRATURE"), ["sensor", "temperature"]);
        assert_eq!(words("Moteurs pas-à-pas"), ["motor", "stepper"]);
        assert_eq!(words("haut-parleur 8 ohm, 3,3 V or 5V"), ["speaker", "8ohm", "3.3v", "5v"]);
        assert_eq!(words("I2S bus, class-D, ESP32-S3"), ["i2s", "bus", "class", "esp32", "s3"]);
        assert_eq!(words("pont en H, ampli"), ["hbridge", "amplifier"]);
        assert_eq!(words("H-bridge micro"), ["hbridge", "microphone"]);
    }

    fn document(name: &str, description: &str, source: &str) -> Document {
        let (annotations, errors) = c2f_lang::annotations::read(source);
        assert!(errors.is_empty(), "{errors:?}");
        Document::new(name, &format!("electronics/{name}.c2f"), description, &annotations)
    }

    #[test]
    fn every_word_must_be_found_and_the_name_weighs_most() {
        let documents = [
            document("speaker", "A small speaker", ""),
            document("amp", "An amplifier for a speaker", "// @pin + [0, 0, 0] output \"speaker +\"\n"),
            document("probe", "A probe", "// @spec range: -40 to 125 °C\n// @pin VDD [0, 0, 0] power_in 3.3V\n"),
        ];
        let names = |query: &str| -> Vec<usize> { rank(&documents, query).into_iter().map(|(i, _)| i).collect() };
        assert_eq!(names("Speaker"), [0, 1]);
        assert_eq!(names("speaker amplifier"), [1]);
        assert_eq!(names("ampl"), [1]);
        assert_eq!(names("3,3 V"), [2]);
        assert_eq!(names("alimentation"), [2]);
        assert_eq!(names("125"), [2]);
        assert_eq!(names("drum"), Vec::<usize>::new());
        assert_eq!(names(""), [0, 1, 2]);
        assert_eq!(names("  de la  "), [0, 1, 2]);
    }

    /// The library's parts, as the catalogue lists them: by name.
    fn library() -> (Vec<String>, Vec<Document>) {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library/electronics");
        let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
        files.retain(|p| p.extension().is_some_and(|e| e == "c2f"));
        files.sort();
        let mut names = Vec::new();
        let mut documents = Vec::new();
        for path in files {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            let source = std::fs::read_to_string(&path).unwrap();
            let (annotations, errors) = c2f_lang::annotations::read(&source);
            assert!(errors.is_empty(), "{name}: {errors:?}");
            let description = annotations.description.as_ref().map(|d| d.text.clone()).expect("a description");
            documents.push(Document::new(&name, &format!("electronics/{name}.c2f"), &description, &annotations));
            names.push(name);
        }
        (names, documents)
    }

    #[test]
    fn the_library_answers_in_french_and_english() {
        let (names, documents) = library();
        let found = |query: &str| -> Vec<&str> {
            rank(&documents, query).into_iter().map(|(i, _)| names[i].as_str()).collect()
        };
        let first = |query: &str| found(query).first().copied();
        assert_eq!(first("micro I2S"), Some("inmp441"));
        assert_eq!(first("microphone"), Some("inmp441"));
        assert_eq!(first("driver 5V"), Some("drv8833"));
        assert_eq!(first("driver moteur"), Some("drv8833"));
        assert_eq!(first("pont en H"), Some("drv8833"));
        assert_eq!(first("ampli"), Some("max98357a"));
        assert_eq!(first("amplificateur audio"), Some("max98357a"));
        assert_eq!(first("haut-parleur"), Some("speaker"));
        assert_eq!(first("moteur"), Some("dc_motor"));
        assert_eq!(first("tactile"), Some("touch_plate"));
        assert_eq!(first("xiao"), Some("xiao_esp32s3"));
        let sorted = |query: &str| {
            let mut names = found(query);
            names.sort();
            names
        };
        assert_eq!(sorted("carte ESP32-S3"), ["esp32s3_devkitc1", "xiao_esp32s3"]);
        assert_eq!(sorted("condensateurs"), ["capacitor_ceramic", "capacitor_radial"]);
        // The library has no stepper motor: only the driver that can run
        // one answers, not the DC motor.
        assert_eq!(found("moteur pas à pas"), ["drv8833"]);
        // Nor a temperature sensor.
        assert_eq!(found("capteur de température"), Vec::<&str>::new());
    }
}
