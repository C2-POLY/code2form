# Code2Form

[![CI](https://github.com/C2-POLY/code2form/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/C2-POLY/code2form/actions/workflows/ci.yml)

Parametric 3D design as code, with a live preview. Designs are short
programs saved as `.c2f` files, and Code2Form also opens `.scad` files as
they are. Written in Rust, it renders as you type.

- **Editor and live 3D view**: syntax highlighting, errors marked on their
  line, re-render 400 ms after the last keystroke, orbit, pan and zoom,
  and a turntable that turns the view around the model;
  Open Recent lists the last ten files opened or saved; each file opens in
  a tab of its own, with its view, console and undo history, and the tabs
  come back at the next launch.
- **Settings** (⌘,): the app icon in ten tints (graphite by default,
  shown in the Dock and the Finder), the code's size, whether to render
  as you type and after what pause, and folders searched for libraries
  and for `text()`'s fonts before the default ones. The grid, the axes
  and the layer height are kept as left.
- **Customizer**: the parameters a design declares in comments
  (`width = 20; // [10:100]`) get sliders, dropdowns, checkboxes and text
  fields, grouped by `/* [Group] */`; presets are saved next to the design
  in a `.json` file.
- **Geometry**: CSG with [Manifold](https://github.com/elalish/manifold)
  (through its Rust port), `text()` with real font shaping, `import()` of
  STL, OFF, AMF, 3MF, DXF and SVG, `surface()` from `.dat` and PNG.
- **Export** of solids to STL and 3MF (in millimeters), checked with
  PrusaSlicer's own loader, and of 2D models to SVG and DXF, laid out as
  the reference implementation (release 2021.01) writes them.
- **Print pauses**: `pause(z, message);` asks the 3D print to stop once it
  is done up to height `z`, before the layer that covers it, to drop
  magnets, nuts or weights into closed cavities. The 3MF carries the
  pauses (`M601`, the message shown on the printer) and an empty settings
  part, so that PrusaSlicer opens it as a project, which keeps the pauses
  and leaves the printer, print and filament settings as they are: with
  File > Open Project, or "Open as project" when the file is dropped or
  opened from the Finder. It makes no geometry, and the reference
  implementation ignores it with a warning. The 3D view shows each pause
  as a plane at its height, labelled with its message (the Pauses toggle).
- **Layer view** (the Layers toggle): the model cut at the top of a layer,
  its cut face in red so cavities show, picked with a slider on the right
  of the view (drag, scroll, or the arrow keys), which marks the pauses;
  the layer height is set under the slider. Right-click the slider to add
  a pause at a layer's top or remove one, and drag a mark to move it: the
  app writes the change into the code, as one step to undo. A height
  computed in the code moves there, and pauses from a library or an
  included file stay as they are.
- **Selection** in the 3D view: a click on an object lights it up and
  puts the editor on the line of the statement that made it, and the
  editor's cursor lights up the geometry of the statement it is in. A
  module written in the file is one object, which a double-click goes
  inside; Shift-click adds to the selection, Alt-click reaches the shape
  a `difference()` subtracts (shown see-through), and Escape clears it.
  Shapes from a library or an included file are read-only.
- **Tree** of the program, between the editor and the view: its
  operations, transforms, shapes and module calls, nested as in the code
  and named by the `// name` comment ending their line. A click selects
  the object in the view (Shift-click adds), and what the view selects
  lights up in the tree; a double-click or F2 renames it, the eye hides
  it (`*`), `!` isolates it and `%` makes it see-through, and a drag
  moves it before, after or into another, as into a `difference()`.
  Each is one edit of the code, one step to undo. The **Code** toggle
  (⌘E) hides the editor, to model with the view, the tree and the panels
  alone.
- **Properties panel**: the selected call's arguments in a panel on the
  right, as in Fusion: sizes, radii, `$fn`, vectors component by
  component, booleans, texts and a color picker, for builtins and for the
  parameters a module declares (in the file or in a `use`d library).
  Changing a value rewrites its literal in place, as one undo step; a
  number dragged (on its field or its label) is written once, when the
  drag ends. A parameter left out shows its default, greyed, and editing
  it adds the argument; a computed one (`width / 2`) is never overwritten:
  it shows its value, with buttons to replace it by that number, to go to
  the variable's definition or to open it in the Customizer. The
  *Placement* section edits the `translate`, `rotate` and `scale` around
  the object, and *Edges* rounds or chamfers a `cube` or a `cylinder`
  (all, vertical, top or bottom edges; top, bottom or both rims) into a
  call to the library's `shapes/rounded.c2f`, keeping its size.
- **Move, turn and scale handles** on the selected object, as in Fusion:
  arrows and squares move it along an axis or in a plane, rings turn it
  about its centre, and the end squares and the centre dot scale it.
  The object follows the drag with the amount by the pointer, snapped to
  1 mm and 15° (steps set in the bar over the view; Shift snaps off), or
  type an exact value and press Enter. On release the code gets the
  change as one undo step: the object's `translate`, `rotate` or `scale`
  is rewritten, or added around it. World or Local picks the world's
  axes or the object's own once turned.
- **Sketches**: Sketch ▸ On the XY plane (or XZ, YZ, or a flat face of
  the model) turns the view face on, with a grid, to draw lines,
  rectangles, circles and arcs that snap to points, midpoints and
  centers. Constraints (horizontal, vertical, parallel, perpendicular,
  coincident, tangent, equal, fixed) and dimensions (length, radius,
  angle, optionally named) hold the profile, solved live as points are
  dragged: blue can still move, white is fully constrained, red
  conflicts. Finishing writes a `sketch_<name>()` module with its named
  dimensions as Customizer parameters, in one undo step; Extrude
  (`linear_extrude`, adding or cutting into the object it was drawn on)
  and Revolve (`rotate_extrude` around a line of the sketch) write the
  solid too. Edit sketch reopens it with its constraints from the code.
- **Electronics**: a part declares its pins, mount holes and ports in
  comments (`// @pin D3 [2.54, 15.2, 1.6] up io 3.3V "GPIO4"`), which
  the reference implementation ignores. A `.c2fc` circuit file (TOML)
  places parts and wires their pins, as in `examples/grogu/grogu.c2fc`.
  See the [electronics guide](docs/electronics.md).
- **Circuit checks** before soldering: a supply above what a pin takes,
  supplies tied together or to ground, a supply or ground left open, a
  wired `nc` pin; warnings for no common ground, a wired `reserved` pin
  and two outputs on one net.
- **Wiring in 3D**: the app shows a circuit's parts and its wires in
  their colors, with a legend; a click on a pin highlights its net, and
  the Assembly mode walks through the wires one at a time, saving its
  progress next to the circuit. Wires are moved, added and removed with
  the mouse, and a Catalogue panel of the library's parts and your own
  adds a part by dragging its picture into the view, or makes a new one
  from a template.
- **Electronics library**: annotated parts in `library/electronics`
  (XIAO ESP32-S3, ESP32-S3-DevKitC-1, DRV8833, MAX98357A, INMP441,
  capacitors, a speaker, a DC motor, a touch plate); see
  `library/README.md`.
- **Enclosures**: a `.c2f` file that names its circuit
  (`// @circuit grogu.c2fc`) gets `standoffs_for()`, `holes_for()`,
  `cutout_for()` and `circuit_part()`, which follow the circuit's parts,
  as `examples/grogu/grogu_plate.c2f` does.
- **Command line**: `c2f` prints the AST, the CSG tree, an STL, a 3MF, an
  SVG or a DXF, and takes `-D name=value`, `-p presets.json` and
  `-P preset`; `c2f pins` prints a part's annotations, and `c2f circuit`
  checks a circuit, prints its wiring list and bill of materials, and
  exports its assembled parts (see the
  [guide](docs/electronics.md#6-from-the-command-line)).
- **AI agents**: `c2f mcp` is a Model Context Protocol server, so that an
  agent such as Claude Code writes a model, renders it, looks at a
  picture of it and exports it, without a window; and describes a
  circuit from the library's parts, has it checked and looks at it with
  its wires.

## AI agents (MCP)

`c2f mcp` serves the Model Context Protocol on stdin and stdout. To add it
to Claude Code, with `c2f` on your `PATH` (or its full path instead):

```sh
claude mcp add code2form -- c2f mcp
```

The agent works on files and code it renders, not in the open app. Each
tool takes the program's text as `code` (its includes resolving from
`working_dir`, by default the server's current directory) or a file as
`path` (resolving from its folder), and, but for `parameters`, customizer
values as `parameters`, such as `{"width": 30, "label": "A"}`.

| Tool | What it returns |
|---|---|
| `render` | The error that stops the model, with its file, line and column; the echo output and warnings; whether it is 3D or 2D, its triangles, volume (area in 2D) and bounding box |
| `parameters` | The customizer parameters: name, group, description, default and widget |
| `image` | A PNG picture of the model from `view` (`diagonal` or `isometric`, `front`, `back`, `left`, `right`, `top`, `bottom`), `width` by `height` pixels (800 by 600, at most 1600), with the `render` report |
| `export` | Writes a solid as STL or 3MF, or a 2D model as SVG or DXF, to `output`, in the `format` its extension gives |

For electronics, the agent finds parts and describes a circuit (`.c2fc`).
The circuit tools take the circuit's text as `code` (its part files
resolving from `working_dir`, then the library folders) or a file as
`path`.

| Tool | What it returns |
|---|---|
| `library_parts` | The parts in the `electronics/` folders of the library paths: the name a circuit gives as `part`, the file, and the file's first comment line |
| `part_pins` | A part's pins (name, label, type, voltage, reserved, position, direction), mount holes and ports, and its badly written annotations, from `code` or `path` (as is, from `working_dir`, or from the library) |
| `check_circuit` | Whether the circuit is fit to solder (`ok`): its errors and warnings with their file, line and column, from reading it, evaluating its parts and checking its wiring |
| `circuit_image` | A PNG picture of the circuit, its parts placed and its wires as tubes in their colors, from `view`, `width` by `height` pixels; `net` (a pin, as `xiao.GND`) highlights the wires of its net and greys the others |
| `circuit_wires` | The wiring list in the order to lay the wires: step, each pin and its label, color, label, length in millimeters |
| `circuit_bom` | The bill of materials: each part file and set of params, its quantity and its instances |

For example, ask Claude Code: "With the Code2Form tools, build a
`robot.c2fc` circuit for a XIAO ESP32-S3 that drives a DC motor through
a DRV8833 and reads a button: look up the parts' pins, check the circuit
until it has no errors, and show me the wiring." The agent lists the
parts, reads their pins, writes the circuit, validates it with
`check_circuit`, looks at it with `circuit_image` and gives the wiring
list. Then open `robot.c2fc` in the app to see it in 3D, and
`c2f circuit wires robot.c2fc` prints the list to solder from.

The picture comes from a small software renderer in `c2f-geom`, needing
no screen or GPU: the app's colors, lighting and framing, with outlines
and creases drawn darker. A 2D model shows as the 1 mm slab the app
draws, best seen from `top`.

## Compatibility

Code2Form opens `.scad` files. Its parser, evaluator and geometry are
checked against a reference implementation (release 2021.01) on every
example and library that release ships: the same syntax tree, CSG tree
and echo output on all 86 files, and the same volumes on every model it
can export but one (see below). Committed goldens in `tests/oracle` keep
it so without the reference installed.

Known differences: on `GEB.scad`'s exact setup, the reference's
`projection()` without `cut` returns B's holes with the wrong orientation,
so the operations after it fill them, while Code2Form keeps them open as
the reference does for the same solid placed or built otherwise (0.3% of
the volume of `GEB.scad`); fonts that keep their hinting at
large sizes (Helvetica, Menlo) differ by a fraction of a percent of the
text size; a few edge cases with NaN and infinite values.

## Building

Rust 1.85 or later.

```sh
cargo run --release -p code2form -- design.c2f   # the desktop app
cargo run --release -p c2f-cli -- stl design.c2f > design.stl
cargo run --release -p c2f-cli -- 3mf design.c2f > design.3mf
cargo run --release -p c2f-cli -- svg plate.c2f > plate.svg     # 2D models
cargo run --release -p c2f-cli -- dxf plate.c2f > plate.dxf
cargo run --release -p c2f-cli -- circuit check examples/grogu/grogu.c2fc
cargo run --release -p c2f-cli -- circuit wires --csv examples/grogu/grogu.c2fc > wires.csv
cargo run --release -p c2f-cli -- circuit 3mf examples/grogu/grogu.c2fc > grogu.3mf
```

On macOS, `scripts/bundle-macos.sh` builds `Code2Form.app`, a universal
app that runs on Apple silicon and Intel Macs (macOS 11 or later), and
the zips a release ships: `Code2Form-<version>-macos-universal.zip` and
`c2f-<version>-macos-universal.zip`, in `target/release`. It needs a
Rust toolchain with both macOS targets
(`rustup target add aarch64-apple-darwin x86_64-apple-darwin`). `--install`
puts the app in `~/Applications`, and `--install --default` also makes it the
app that opens `.c2f` and `.scad` files. The app then carries the
repository's `library` folder, and the Liberation fonts and the MCAD
library, so it works on its own; the script copies the last two from
the reference app installed in `/Applications`,
or from the `Contents/Resources` folder that `C2F_REFERENCE_RESOURCES`
names.

## Layout

| Crate | What it does |
|---|---|
| `crates/lang` (`c2f-lang`) | Lexer, parser, AST, customizer comments, electronics annotations, AST dump |
| `crates/eval` (`c2f-eval`) | Evaluation to a CSG tree, built-in functions and modules, parameters |
| `crates/geom` (`c2f-geom`) | Meshes, booleans, extrusions, text, imports, STL, 3MF, SVG and DXF exports, PNG pictures |
| `crates/circuit` (`c2f-circuit`) | Circuit files: parts placed in space and the wires between their pins |
| `crates/app` (`c2f-cli`) | The `c2f` command and its MCP server |
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
release bundles: `fonts/` keeps that exact version, which CI points the
font path variable `crates/geom/src/text/fonts.rs` reads at, on macOS,
Windows and Linux.

## Licence

Code2Form is licensed under either of the [Apache License 2.0](LICENSE-APACHE)
or the [MIT licence](LICENSE-MIT), at your option. Unless you state
otherwise, any contribution you submit for inclusion is licensed the same
way, without any additional terms. The parts and helpers in `library/`
are under the same licences.

The Liberation fonts 2.00.1 in `fonts/` are under the SIL Open Font
License 1.1 (`fonts/Liberation-2.00.1/LICENSE`). The macOS app bundles
them and the MCAD library (LGPL 2.1), each with its licence; the app
build copies them from the reference app, and MCAD is not part of this
repository. Some test designs and data in `tests/oracle` come from the
reference release's examples, dedicated to the public domain (CC0).
