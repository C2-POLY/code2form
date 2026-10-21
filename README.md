# Code2Form

[![CI](https://github.com/C2-POLY/code2form/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/C2-POLY/code2form/actions/workflows/ci.yml)

Parametric 3D design as code, with a live preview. Designs are short
programs saved as `.c2f` files, and Code2Form also opens `.scad` files as
they are. Written in Rust, it renders as you type.

- **Editor and live 3D view**: syntax highlighting, errors marked on their
  line, re-render 400 ms after the last keystroke, orbit, pan and zoom.
- **Customizer**: the parameters a design declares in comments
  (`width = 20; // [10:100]`) get sliders, dropdowns, checkboxes and text
  fields, grouped by `/* [Group] */`; presets are saved next to the design
  in a `.json` file.
- **Geometry**: CSG with [Manifold](https://github.com/elalish/manifold)
  (through its Rust port), `text()` with real font shaping, `import()` of
  STL, OFF, AMF, 3MF, DXF and SVG, `surface()` from `.dat` and PNG.
- **Export** to STL and 3MF (in millimeters), checked with PrusaSlicer's
  own loader.
- **Command line**: `c2f` prints the AST, the CSG tree, an STL or a 3MF, and
  takes `-D name=value`, `-p presets.json` and `-P preset`.

## Compatibility

Code2Form opens `.scad` files. Its parser, evaluator and geometry are
checked against a reference implementation (release 2021.01) on every
example and library that release ships: the same syntax tree, CSG tree
and echo output on all 86 files, and the same volumes on every model it
can export but one (see below). Committed goldens in `tests/oracle` keep
it so without the reference installed.

Known differences: `offset(r = ...)` places its arc points slightly
differently (Clipper 2 instead of 1), which moves volumes by up to 0.3%;
fonts that keep their hinting at large sizes (Helvetica, Menlo) differ by
a fraction of a percent of the text size; a few edge cases with NaN and
infinite values.

## Building

Rust 1.85 or later.

```sh
cargo run --release -p code2form -- design.c2f   # the desktop app
cargo run --release -p c2f-cli -- stl design.c2f > design.stl
cargo run --release -p c2f-cli -- 3mf design.c2f > design.3mf
```

On macOS, `scripts/bundle-macos.sh` builds `Code2Form.app`; `--install`
puts it in `~/Applications`, and `--install --default` also makes it the
app that opens `.c2f` and `.scad` files. The app then carries the
Liberation fonts and the MCAD library, so it works on its own; the
script copies them from the reference app installed in `/Applications`,
or from the `Contents/Resources` folder that `C2F_REFERENCE_RESOURCES`
names.

## Layout

| Crate | What it does |
|---|---|
| `crates/lang` (`c2f-lang`) | Lexer, parser, AST, customizer comments, AST dump |
| `crates/eval` (`c2f-eval`) | Evaluation to a CSG tree, built-in functions and modules, parameters |
| `crates/geom` (`c2f-geom`) | Meshes, booleans, extrusions, text, imports, STL and 3MF exports |
| `crates/app` (`c2f-cli`) | The `c2f` command |
| `crates/studio` (`code2form`) | The desktop app (egui) |

## Tests

```sh
cargo test --workspace                       # unit tests and committed goldens
cargo test --workspace -- --include-ignored  # also compare live with the reference
scripts/oracle.sh                            # regenerate the goldens from the reference
```

The live comparisons and the goldens need the reference release 2021.01,
installed in `/Applications` or wherever `C2F_REFERENCE` points (its
executable). The `text()` tests need the Liberation fonts 2.00.1 that
release bundles, read from its app or from the font path variable
`crates/geom/src/text/fonts.rs` reads; CI fetches them and sets that
variable on macOS, Windows and Linux.

## Licence

Code2Form is licensed under either of the [Apache License 2.0](LICENSE-APACHE)
or the [MIT licence](LICENSE-MIT), at your option. Unless you state
otherwise, any contribution you submit for inclusion is licensed the same
way, without any additional terms.

The macOS app bundles the Liberation fonts (SIL Open Font License 1.1)
and the MCAD library (LGPL 2.1), each with its licence; they are copied
from the reference app when the app is built and are not part of this
repository. Some test designs and data in `tests/oracle` come from the
reference release's examples, dedicated to the public domain (CC0).
