//! Checks a circuit before soldering it, from what its parts declare of
//! their pins: voltages, supplies, grounds, and pins better left alone.
//! Nothing is evaluated: the checks read the circuit and the annotations.
//!
//! A net is a set of pins connected by wires. The ground pins of one part
//! are taken to be one net, as boards tie them together.

use c2f_lang::Span;
use c2f_lang::annotations::PinKind;

use crate::{Circuit, End};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    pub fn name(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

/// A problem found, at the wire it is about in the circuit file (its
/// `from`), or at the part's name.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub severity: Severity,
    pub message: String,
    pub span: Span,
}

impl Finding {
    /// As an error of the circuit file `src` at `path` is rendered,
    /// `warning:` for a warning.
    pub fn render(&self, src: &str, path: &std::path::Path) -> String {
        let rendered = c2f_lang::Diagnostic::new(self.message.clone(), self.span).render(src, &path.display().to_string());
        match self.severity {
            Severity::Error => rendered,
            Severity::Warning => rendered.replacen("error:", "warning:", 1),
        }
    }
}

/// How close two voltages are to be the same.
const TOLERANCE: f64 = 1e-6;

/// The problems of `circuit`, in the order of the circuit file.
///
/// Errors: a supply above what a pin on its net takes; two supplies of
/// different voltages, or a supply and a ground, on one net; a part's
/// supply or ground left unwired while its other pins are wired; a wired
/// `nc` pin. Warnings: grounds that are not all one net; a wired
/// `reserved` pin; two outputs on one net.
pub fn check(circuit: &Circuit) -> Vec<Finding> {
    let nets = Nets::new(circuit);
    let mut findings = Vec::new();
    let name = |end: End| format!("`{}`", circuit.end_name(end));
    let volts = |v: f64| format!("{v} V");
    let mut add = |severity, message: String, span| findings.push(Finding { severity, message, span });

    for net in nets.all() {
        let kind = |end: End| circuit.pin(end).kind;
        let sources: Vec<(End, f64)> = net
            .iter()
            .filter(|&&end| kind(end) == PinKind::PowerOut)
            .filter_map(|&end| circuit.pin(end).voltage.map(|v| (end, v)))
            .collect();
        // The highest supply against what each pin takes.
        if let Some(&(source, supply)) = sources.iter().max_by(|a, b| a.1.total_cmp(&b.1)) {
            for &end in &net {
                let pin = circuit.pin(end);
                if matches!(pin.kind, PinKind::PowerOut | PinKind::Ground) {
                    continue;
                }
                if let Some(most) = pin.voltage.filter(|&most| most < supply - TOLERANCE) {
                    let message =
                        format!("{} supplies {} to {}, which takes at most {}", name(source), volts(supply), name(end), volts(most));
                    add(Severity::Error, message, nets.span(end));
                }
            }
        }
        if let Some(&(first, voltage)) = sources.first() {
            for &(other, v) in &sources[1..] {
                if (v - voltage).abs() > TOLERANCE {
                    let message = format!(
                        "{} ({}) and {} ({}) are tied together: two supplies of different voltages",
                        name(first),
                        volts(voltage),
                        name(other),
                        volts(v)
                    );
                    add(Severity::Error, message, nets.span(other));
                }
            }
        }
        if let Some(&ground) = net.iter().find(|&&end| kind(end) == PinKind::Ground) {
            for &end in net.iter().filter(|&&end| kind(end) == PinKind::PowerOut) {
                let message = format!("{} is a supply wired to the ground {}: a short circuit", name(end), name(ground));
                add(Severity::Error, message, nets.span(end));
            }
        }
        let outputs: Vec<End> = net.iter().copied().filter(|&end| kind(end) == PinKind::Output).collect();
        for &other in outputs.iter().skip(1) {
            let message =
                format!("{} and {} are two outputs on one net: they drive it against each other", name(outputs[0]), name(other));
            add(Severity::Warning, message, nets.span(other));
        }
    }

    for (index, part) in circuit.parts.iter().enumerate() {
        let ends: Vec<End> = (0..part.annotations.pins.len()).map(|pin| End { part: index, pin }).collect();
        if !ends.iter().any(|&end| nets.wired(end)) {
            continue;
        }
        let of_kind = |kind| ends.iter().copied().filter(move |&end| circuit.pin(end).kind == kind);
        let mut open = of_kind(PinKind::PowerIn).filter(|&end| !nets.wired(end)).collect::<Vec<_>>();
        // One ground pin wired grounds the part.
        let grounds: Vec<End> = of_kind(PinKind::Ground).collect();
        if !grounds.is_empty() && !grounds.iter().any(|&end| nets.wired(end)) {
            open.push(grounds[0]);
        }
        for end in open {
            let what = if circuit.pin(end).kind == PinKind::Ground { "its ground" } else { "a supply it needs" };
            let message = format!("{} is not wired, though other pins of `{}` are: {what} is left open", name(end), part.name);
            add(Severity::Error, message, part.span);
        }
    }

    for end in nets.wired_ends() {
        let pin = circuit.pin(end);
        if pin.kind == PinKind::Nc {
            add(Severity::Error, format!("{} is not connected (nc): nothing may be wired to it", name(end)), nets.span(end));
        }
        if pin.reserved {
            let why = pin.label.as_ref().map_or(String::new(), |label| format!(" ({label})"));
            let message = format!("{} is reserved by its part{why}: better left unwired", name(end));
            add(Severity::Warning, message, nets.span(end));
        }
    }

    // The grounds of the wired parts, by net, the first one found first.
    let mut grounds: Vec<(usize, End)> = Vec::new();
    for end in nets.wired_ends() {
        if circuit.pin(end).kind == PinKind::Ground {
            let net = nets.root(end);
            if !grounds.iter().any(|&(n, _)| n == net) {
                grounds.push((net, end));
            }
        }
    }
    if let Some(&(_, common)) = grounds.first() {
        for &(_, end) in &grounds[1..] {
            let message = format!("no common ground: {} is not connected to {}", name(end), name(common));
            add(Severity::Warning, message, nets.span(end));
        }
    }

    findings.sort_by_key(|f| (f.span.start, f.severity));
    findings
}

/// The nets of a circuit with at least one wire, each a list of its pins:
/// the order of the wires, each net's pins in the order wires reach them.
pub fn nets(circuit: &Circuit) -> Vec<Vec<End>> {
    Nets::new(circuit).all()
}

/// The net `end` is on, with `end` alone when it is not wired.
pub fn net_of(circuit: &Circuit, end: End) -> Vec<End> {
    nets(circuit).into_iter().find(|net| net.contains(&end)).unwrap_or_else(|| vec![end])
}

/// Union-find over every pin of the circuit, numbered part after part.
struct Nets<'a> {
    circuit: &'a Circuit,
    /// The number of the first pin of each part.
    first: Vec<usize>,
    parent: Vec<usize>,
}

impl<'a> Nets<'a> {
    fn new(circuit: &'a Circuit) -> Nets<'a> {
        let mut first = Vec::with_capacity(circuit.parts.len());
        let mut count = 0;
        for part in &circuit.parts {
            first.push(count);
            count += part.annotations.pins.len();
        }
        let mut nets = Nets { circuit, first, parent: (0..count).collect() };
        for wire in &circuit.wires {
            nets.join(nets.number(wire.from), nets.number(wire.to));
        }
        for (index, part) in circuit.parts.iter().enumerate() {
            let mut grounds = part.annotations.pins.iter().enumerate().filter(|(_, pin)| pin.kind == PinKind::Ground);
            if let Some((first, _)) = grounds.next() {
                for (other, _) in grounds {
                    nets.join(nets.number(End { part: index, pin: first }), nets.number(End { part: index, pin: other }));
                }
            }
        }
        nets
    }

    fn number(&self, end: End) -> usize {
        self.first[end.part] + end.pin
    }

    fn find(&self, mut n: usize) -> usize {
        while self.parent[n] != n {
            n = self.parent[n];
        }
        n
    }

    fn join(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        self.parent[a.max(b)] = a.min(b);
    }

    fn root(&self, end: End) -> usize {
        self.find(self.number(end))
    }

    fn wired(&self, end: End) -> bool {
        self.circuit.wires.iter().any(|w| w.from == end || w.to == end)
    }

    /// The wired pins, in the order the wires reach them.
    fn wired_ends(&self) -> Vec<End> {
        let mut ends: Vec<End> = Vec::new();
        for wire in &self.circuit.wires {
            for end in [wire.from, wire.to] {
                if !ends.contains(&end) {
                    ends.push(end);
                }
            }
        }
        ends
    }

    fn all(&self) -> Vec<Vec<End>> {
        let mut nets: Vec<(usize, Vec<End>)> = Vec::new();
        for end in self.wired_ends() {
            let root = self.root(end);
            match nets.iter_mut().find(|(r, _)| *r == root) {
                Some((_, net)) => net.push(end),
                None => nets.push((root, vec![end])),
            }
        }
        nets.into_iter().map(|(_, net)| net).collect()
    }

    /// The first wire at `end`, or else its part's name.
    fn span(&self, end: End) -> Span {
        self.circuit
            .wires
            .iter()
            .find(|w| w.from == end || w.to == end)
            .map_or(self.circuit.parts[end.part].span, |w| w.span)
    }
}
