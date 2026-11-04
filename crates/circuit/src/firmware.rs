//! A first firmware for a circuit: a PlatformIO project for the board the
//! circuit's parts are wired to, that knows each wire and sets up each
//! part, then tries them in turn, to check the wiring before writing the
//! object's own program.
//!
//! The board is the part that says what it is (`// @board`); a pin of
//! another part wired to one of its GPIOs (a board pin whose label starts
//! `GPIO4`) becomes a constant of `include/pins.h`, named after the part
//! and its pin (`AMP_BCLK`). Each part gives the library that drives it
//! (`// @driver arduino …`), which goes to `platformio.ini`, and the code
//! that sets it up (`// @init global | setup | test: …`), which goes to
//! `src/main.cpp`, its `{part}` and `{pin:NAME}` filled in. A part one of
//! whose lines names a pin not wired to a GPIO is left out, said in a
//! comment, as the rest of its code may need that line; an `optional`
//! line is left out alone.
//!
//! Only `include/pins.h` follows the circuit: generated again, it is
//! rewritten, and the project's other files, the user's from then on, are
//! left as they are.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use c2f_lang::annotations::{InitSection, PinKind};

use crate::check::{self, Severity};
use crate::{Circuit, End};

/// The framework the firmware is written for, as PlatformIO names it.
pub const FRAMEWORK: &str = "arduino";

/// A firmware's files, before they are written.
#[derive(Clone, Debug, PartialEq)]
pub struct Firmware {
    /// The part that is the board, by its name in the circuit.
    pub board: String,
    pub pins_h: String,
    pub platformio_ini: String,
    pub main_cpp: String,
    /// What was left out: lines of set-up code whose pins are not wired
    /// to the board, parts wired to it that say nothing of their driver.
    pub notes: Vec<String>,
}

/// A board pin a part's pin is wired to.
struct Wired {
    /// The part's pin.
    end: End,
    /// `AMP_BCLK`.
    constant: String,
    gpio: u32,
    /// The board's pin, as `xiao.D3`, and the wire's color.
    board_pin: String,
    color: Option<String>,
}

/// The firmware of `circuit`, read from `file_name`. Fails, with the
/// messages of the circuit's check, on an error or a reserved pin wired
/// (unless a wire allows it), and when no part, or more than one, is a
/// board.
pub fn firmware(circuit: &Circuit, file_name: &str) -> Result<Firmware, Vec<String>> {
    let blocking: Vec<String> = check::check(circuit)
        .into_iter()
        .filter(|f| f.severity == Severity::Error || f.rule == "reserved")
        .map(|f| f.message)
        .collect();
    if !blocking.is_empty() {
        return Err(blocking);
    }
    let boards: Vec<usize> = (0..circuit.parts.len()).filter(|&i| circuit.parts[i].annotations.board.is_some()).collect();
    let board_index = match boards[..] {
        [one] => one,
        [] => return Err(vec!["no part of the circuit is a board a firmware runs on (`// @board`): add one, as electronics/xiao_esp32s3.c2f".into()]),
        _ => {
            let names: Vec<&str> = boards.iter().map(|&i| circuit.parts[i].name.as_str()).collect();
            return Err(vec![format!("several parts are boards ({}): the firmware runs on one", names.join(", "))]);
        }
    };
    let board = &circuit.parts[board_index];
    let target = board.annotations.board.as_ref().expect("a board");

    // Each pin of another part, wired to a GPIO of the board, its
    // constant unique.
    let mut wired: Vec<Wired> = Vec::new();
    let mut constants: Vec<String> = Vec::new();
    for (index, part) in circuit.parts.iter().enumerate().filter(|(i, _)| *i != board_index) {
        for pin in 0..part.annotations.pins.len() {
            let end = End { part: index, pin };
            let net = check::net_of(circuit, end);
            let Some((board_end, gpio)) = net.iter().filter(|e| e.part == board_index).find_map(|&e| Some((e, gpio(circuit, e)?)))
            else {
                continue;
            };
            let wire = circuit.wires.iter().find(|w| (w.from == end && w.to == board_end) || (w.to == end && w.from == board_end));
            let constant = unique(constant(&part.name, &part.annotations.pins[pin].name), &mut constants);
            wired.push(Wired {
                end,
                constant,
                gpio,
                board_pin: circuit.end_name(board_end),
                color: wire.and_then(|w| w.color.clone()),
            });
        }
    }

    let mut notes = Vec::new();
    let mut headers: Vec<String> = vec!["Arduino.h".into()];
    let mut libraries: BTreeSet<String> = BTreeSet::new();
    let (mut globals, mut setups, mut tests) = (String::new(), String::new(), String::new());
    let mut identifiers: Vec<String> = Vec::new();
    for (index, part) in circuit.parts.iter().enumerate() {
        let driver = part.annotations.drivers.iter().find(|d| d.framework == FRAMEWORK);
        let uses_board = wired.iter().any(|w| w.end.part == index);
        if index != board_index && driver.is_none() {
            if uses_board {
                notes.push(format!("`{}` is wired to the board but says nothing of its driver (`// @driver arduino …`): set it up by hand", part.name));
            }
            continue;
        }
        let identifier = unique(identifier(&part.name), &mut identifiers);
        let filled: Vec<(InitSection, Result<String, String>, &str, bool)> = InitSection::ALL
            .into_iter()
            .flat_map(|section| part.annotations.init_lines(section).map(move |line| (section, line)))
            .map(|(section, line)| (section, fill(&line.code, &identifier, index, &wired, circuit), line.code.as_str(), line.optional))
            .collect();
        // A line the rest needs, left out: the whole part is, or its code
        // would not build.
        if let Some((_, Err(why), _, _)) = filled.iter().find(|(_, result, _, optional)| result.is_err() && !optional) {
            let _ = writeln!(setups, "  // {} is left out, {why}: set it up by hand", part.name);
            notes.push(format!("`{}` is left out, {why}: set it up by hand", part.name));
            continue;
        }
        if let Some(driver) = driver {
            if let Some(header) = driver.header.as_ref().filter(|h| !headers.contains(h)) {
                headers.push(header.clone());
            }
            libraries.extend(driver.library.clone());
        }
        for (section, out) in [(InitSection::Global, &mut globals), (InitSection::Setup, &mut setups), (InitSection::Test, &mut tests)] {
            let lines: Vec<_> = filled.iter().filter(|(s, ..)| *s == section).collect();
            if lines.is_empty() {
                continue;
            }
            let indent = if section == InitSection::Global { "" } else { "  " };
            let _ = writeln!(out, "{indent}// {}", part.name);
            if section == InitSection::Test {
                let _ = writeln!(out, "  Serial.println(\"{}\");", part.name);
            }
            for (_, result, line, _) in lines {
                match result {
                    Ok(code) => {
                        let _ = writeln!(out, "{indent}{code}");
                    }
                    Err(why) => {
                        let _ = writeln!(out, "{indent}// left out, {why}: {line}");
                        notes.push(format!("{}: left out, {why}: {line}", part.name));
                    }
                }
            }
            if section == InitSection::Test {
                out.push_str("  delay(500);\n");
            }
        }
    }

    let mut pins_h = format!(
        "// Written by Code2Form from {file_name}, and again each time the firmware is\n\
         // generated: the GPIO of the board ({}) each part's pin is wired to.\n\n#pragma once\n",
        board.name
    );
    for (index, part) in circuit.parts.iter().enumerate() {
        let pins: Vec<&Wired> = wired.iter().filter(|w| w.end.part == index).collect();
        if pins.is_empty() {
            continue;
        }
        let _ = writeln!(pins_h, "\n// {}: {}", part.name, part.part);
        let width = pins.iter().map(|w| w.constant.len()).max().unwrap_or(0);
        for w in pins {
            let color = w.color.as_ref().map_or(String::new(), |c| format!(", {c} wire"));
            let _ = writeln!(pins_h, "#define {:width$} {:<3} // {}{color}", w.constant, w.gpio, w.board_pin);
        }
    }

    let mut platformio_ini = format!(
        "; Written by Code2Form from {file_name}: the board and the libraries its parts need.\n\n\
         [env:{board}]\nplatform = {platform}\nboard = {board}\nframework = {FRAMEWORK}\nmonitor_speed = 115200\n",
        board = target.board,
        platform = target.platform,
    );
    if !libraries.is_empty() {
        platformio_ini.push_str("lib_deps =\n");
        for library in &libraries {
            let _ = writeln!(platformio_ini, "    {library}");
        }
    }

    let mut main_cpp = format!(
        "// Written by Code2Form from {file_name}: a first firmware that sets each part\n\
         // up, then tries them in turn, to check the wiring. It is yours to change:\n\
         // generating the firmware again rewrites include/pins.h only.\n\n"
    );
    for header in &headers {
        let _ = writeln!(main_cpp, "#include <{header}>");
    }
    main_cpp.push_str("#include \"pins.h\"\n\n");
    if !globals.is_empty() {
        main_cpp.push_str(&globals);
        main_cpp.push('\n');
    }
    let stem = Path::new(file_name).file_stem().map_or_else(|| "circuit".into(), |s| s.to_string_lossy().into_owned());
    let _ = write!(
        main_cpp,
        "void setup() {{\n  Serial.begin(115200);\n  delay(500);\n  Serial.println(\"{stem}: setting up\");\n{setups}}}\n\n\
         void loop() {{\n{tests}  delay(1000);\n}}\n"
    );
    Ok(Firmware { board: board.name.clone(), pins_h, platformio_ini, main_cpp, notes })
}

/// A file of the project, where it goes in the firmware's folder.
pub const PINS_H: &str = "include/pins.h";
pub const PLATFORMIO_INI: &str = "platformio.ini";
pub const MAIN_CPP: &str = "src/main.cpp";

/// What writing a firmware did to each of its files.
#[derive(Clone, Debug, PartialEq)]
pub struct Written {
    pub path: PathBuf,
    /// False for a file that was there, left to the user.
    pub written: bool,
}

/// Writes `firmware` into `dir`: `include/pins.h` always, the others only
/// when they are not there yet.
pub fn write(firmware: &Firmware, dir: &Path) -> Result<Vec<Written>, String> {
    let files = [(PINS_H, &firmware.pins_h, true), (PLATFORMIO_INI, &firmware.platformio_ini, false), (MAIN_CPP, &firmware.main_cpp, false)];
    let mut out = Vec::new();
    for (name, text, always) in files {
        let path = dir.join(name);
        let write = always || !path.exists();
        if write {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|err| format!("cannot create {}: {err}", parent.display()))?;
            }
            std::fs::write(&path, text).map_err(|err| format!("cannot write {}: {err}", path.display()))?;
        }
        out.push(Written { path, written: write });
    }
    Ok(out)
}

/// The GPIO a board pin is, from its label (`"GPIO4, A3, touch 4"`); `None`
/// for a supply, a ground, or a pin that is no GPIO.
fn gpio(circuit: &Circuit, end: End) -> Option<u32> {
    let pin = circuit.pin(end);
    if !matches!(pin.kind, PinKind::Io | PinKind::Input | PinKind::Output) {
        return None;
    }
    let digits: String = pin.label.as_deref()?.strip_prefix("GPIO")?.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// `amp` and `BCLK` as a constant: `AMP_BCLK`; `_5V_RAIL_EN` for a part
/// whose name starts with a digit.
fn constant(part: &str, pin: &str) -> String {
    let word = |s: &str| s.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect::<String>();
    let pin = match pin {
        "+" => "PLUS".to_string(),
        "-" => "MINUS".to_string(),
        pin => word(pin),
    };
    let mut out = format!("{}_{pin}", word(part));
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// `name`, or `name_2`, `name_3`… when it is taken: two names that differ
/// only in what C cannot spell (`ring-2`, `ring_2`) stay apart.
fn unique(name: String, taken: &mut Vec<String>) -> String {
    let mut candidate = name.clone();
    let mut n = 2;
    while taken.contains(&candidate) {
        candidate = format!("{name}_{n}");
        n += 1;
    }
    taken.push(candidate.clone());
    candidate
}

/// A part's name as a C++ name: `amp`, `ring_2`, `_5v_rail`.
fn identifier(name: &str) -> String {
    let mut out: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// A line of set-up code with `{part}` and `{pin:NAME}` filled in; why it
/// is left out, when a pin it names is not wired to a GPIO.
fn fill(line: &str, part_name: &str, part: usize, wired: &[Wired], circuit: &Circuit) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = line;
    while let Some(open) = rest.find("{pin:") {
        out.push_str(&rest[..open]);
        let close = open + rest[open..].find('}').ok_or("an unclosed `{pin:`")?;
        let name = rest[open + 5..close].trim();
        let instance = &circuit.parts[part];
        let pin = instance.pin(name).ok_or_else(|| format!("`{}` has no pin {name}", instance.name))?;
        let w = wired
            .iter()
            .find(|w| w.end == End { part, pin })
            .ok_or_else(|| format!("`{}.{name}` is not wired to a GPIO of the board", instance.name))?;
        out.push_str(&w.constant);
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    Ok(out.replace("{part}", part_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOARD: &str = "\
// @board seeed_xiao_esp32s3 espressif32
// @driver arduino builtin \"Arduino.h\"
// @pin D0 [0, 0, 0] io 3.3V \"GPIO1, A0\"
// @pin D2 [0, 2, 0] io 3.3V reserved \"GPIO3, strapping\"
// @pin D6 [0, 6, 0] io 3.3V \"GPIO43, TX\"
// @pin 3V3 [0, 8, 0] power_out 3.3V
// @pin GND [0, 9, 0] ground
cube(1);
";
    const RING: &str = "\
// @driver arduino \"adafruit/Adafruit NeoPixel@^1.12.3\" \"Adafruit_NeoPixel.h\"
// @init global: Adafruit_NeoPixel {part}(12, {pin:DIN}, NEO_GRB + NEO_KHZ800);
// @init setup: {part}.begin();
// @init test: {part}.fill(0x202020); {part}.show();
// @init test optional: pinMode({pin:EN}, OUTPUT);
// @pin DIN [0, 0, 0] input 3.3V
// @pin EN [0, 1, 0] input 3.3V
// @pin VCC [0, 2, 0] power_in 3.3V
// @pin GND [0, 3, 0] ground
cube(1);
";

    fn circuit(wires: &str, board: &str) -> Circuit {
        let dir = std::env::temp_dir().join(format!("c2f-firmware-{}-{}", std::process::id(), wires.len() + board.len()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("board.c2f"), board).unwrap();
        std::fs::write(dir.join("ring.c2f"), RING).unwrap();
        let src = format!(
            "[parts.xiao]\npart = \"board.c2f\"\n[parts.ring-2]\npart = \"ring.c2f\"\nat = [20, 0, 0]\n\
             [[wires]]\nfrom = \"xiao.3V3\"\nto = \"ring-2.VCC\"\n[[wires]]\nfrom = \"xiao.GND\"\nto = \"ring-2.GND\"\n{wires}"
        );
        crate::parse(&src, &dir.join("lamp.c2fc"), &[]).expect("parses")
    }

    #[test]
    fn a_pin_wired_to_a_gpio_becomes_a_constant_and_fills_the_code() {
        let circuit = circuit("[[wires]]\nfrom = \"xiao.D6\"\nto = \"ring-2.DIN\"\ncolor = \"green\"\n", BOARD);
        let firmware = firmware(&circuit, "lamp.c2fc").expect("a firmware");
        assert_eq!(firmware.board, "xiao");
        assert!(firmware.pins_h.contains("#define RING_2_DIN 43  // xiao.D6, green wire\n"), "{}", firmware.pins_h);
        assert!(!firmware.pins_h.contains("VCC"), "supplies are no GPIO: {}", firmware.pins_h);
        let main = &firmware.main_cpp;
        assert!(main.contains("#include <Arduino.h>\n#include <Adafruit_NeoPixel.h>\n#include \"pins.h\"\n"), "{main}");
        assert!(main.contains("Adafruit_NeoPixel ring_2(12, RING_2_DIN, NEO_GRB + NEO_KHZ800);"), "{main}");
        assert!(main.contains("  ring_2.begin();\n"), "{main}");
        assert!(main.contains("  Serial.println(\"ring-2\");\n  ring_2.fill(0x202020); ring_2.show();\n"), "{main}");
        // EN is not wired: its line is left out, and said so.
        assert!(main.contains("  // left out, `ring-2.EN` is not wired to a GPIO of the board: pinMode({pin:EN}, OUTPUT);"), "{main}");
        assert_eq!(firmware.notes.len(), 1);
        assert!(firmware.platformio_ini.contains("[env:seeed_xiao_esp32s3]\nplatform = espressif32\nboard = seeed_xiao_esp32s3\nframework = arduino\n"));
        assert!(firmware.platformio_ini.ends_with("lib_deps =\n    adafruit/Adafruit NeoPixel@^1.12.3\n"), "{}", firmware.platformio_ini);
    }

    #[test]
    fn a_reserved_pin_an_error_or_no_board_stops_it() {
        let reserved = circuit("[[wires]]\nfrom = \"xiao.D2\"\nto = \"ring-2.DIN\"\n", BOARD);
        let errors = firmware(&reserved, "lamp.c2fc").unwrap_err();
        assert!(errors[0].contains("`xiao.D2` is reserved by its part"), "{errors:?}");
        // Unless the wire allows it.
        let allowed = circuit("[[wires]]\nfrom = \"xiao.D2\"\nto = \"ring-2.DIN\"\nallow = [\"reserved\"]\n", BOARD);
        assert!(firmware(&allowed, "lamp.c2fc").unwrap().pins_h.contains("#define RING_2_DIN 3"));
        let no_board = circuit("", &BOARD.replace("// @board seeed_xiao_esp32s3 espressif32\n", ""));
        assert!(firmware(&no_board, "lamp.c2fc").unwrap_err()[0].starts_with("no part of the circuit is a board"));
    }

    #[test]
    fn writing_again_rewrites_the_pins_only() {
        let first = firmware(&circuit("[[wires]]\nfrom = \"xiao.D0\"\nto = \"ring-2.DIN\"\n", BOARD), "lamp.c2fc").unwrap();
        let dir = std::env::temp_dir().join(format!("c2f-firmware-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let written = write(&first, &dir).unwrap();
        assert!(written.iter().all(|w| w.written));
        std::fs::write(dir.join(MAIN_CPP), "// mine").unwrap();
        let second = firmware(&circuit("[[wires]]\nfrom = \"xiao.D6\"\nto = \"ring-2.DIN\"\n", BOARD), "lamp.c2fc").unwrap();
        let written = write(&second, &dir).unwrap();
        assert_eq!(written.iter().map(|w| w.written).collect::<Vec<_>>(), [true, false, false]);
        assert!(std::fs::read_to_string(dir.join(PINS_H)).unwrap().contains("RING_2_DIN 43"));
        assert_eq!(std::fs::read_to_string(dir.join(MAIN_CPP)).unwrap(), "// mine");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_part_whose_needed_pin_is_not_wired_is_left_out_whole() {
        // DIN, which the ring's declaration needs, is not wired: neither its
        // declaration nor its begin() may be written.
        let firmware = firmware(&circuit("", BOARD), "lamp.c2fc").unwrap();
        let main = &firmware.main_cpp;
        assert!(!main.contains("ring_2.begin()") && !main.contains("Adafruit_NeoPixel ring_2"), "{main}");
        assert!(!main.contains("#include <Adafruit_NeoPixel.h>"), "{main}");
        assert!(main.contains("  // ring-2 is left out, `ring-2.DIN` is not wired to a GPIO of the board: set it up by hand"), "{main}");
        assert!(!firmware.platformio_ini.contains("lib_deps"), "{}", firmware.platformio_ini);
    }

    #[test]
    fn names_become_c_names_and_stay_apart() {
        assert_eq!(constant("amp", "BCLK"), "AMP_BCLK");
        assert_eq!(constant("mic", "L/R"), "MIC_L_R");
        assert_eq!(constant("amp", "+"), "AMP_PLUS");
        assert_eq!(constant("5v-rail", "EN"), "_5V_RAIL_EN");
        assert_eq!(identifier("5v-rail"), "_5v_rail");
        let mut taken = Vec::new();
        let names: Vec<String> = ["MIC_L_R", "MIC_L_R", "MIC_L_R"].map(|n| unique(n.into(), &mut taken)).into();
        assert_eq!(names, ["MIC_L_R", "MIC_L_R_2", "MIC_L_R_3"]);
    }
}
