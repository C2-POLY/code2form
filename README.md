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
- **The window**: one bar across the top holds the File menu, the tabs,
  a switch between the code alone, the code beside the view and the view
  alone, then Render, Export and the settings. The console folds under
  the code (under the view when it is alone), and the view's tools float
  over its top left: fit, the views, the grid, the axes, the layers and
  the pauses.
- **Exploded view**: a design whose pieces are written in `part("name")`
  gets an Exploded button among the view's tools, to see how it goes
  together. It slides the parts apart, a slider sets how far, and two
  complementary ways choose how:
  - **Radial**: each part carries on along the line from the center of
    gravity of the model (or of its parent part) to its own, so a part
    far from the center goes further; one at the center still moves a
    quarter of the model's size.
  - **Axis**: each part slides along its own Z axis, the axis it is built
    and turns about as its `rotate()`s place it, by half the size of its
    parent: a drum comes out of its frame, a roller slides along its
    shaft.

  Either way, a part in a part moves with its parent, then 1.5 times
  further from it at each level down, and a part that is the whole
  model stays. Only the view changes: the model, its exports and its
  sizes stay assembled, and an animated model plays apart.
- **Examples**: the Examples menu lists the designs and circuits that
  come with the app, by subject: a chest that opens, three gears that
  drive one another and a walking mannequin (Game), and the Grogu
  circuit with its plate. One opens as a copy in
  `~/Documents/Code2Form/Examples`, its folder with it, to change and
  save; opened again, the copy keeps its changes.
- **Settings** (⌘,), page by page. General: the interface in English
  or French, and whether the last tabs come back at launch. Appearance:
  a light or a dark theme (or the system's, as it changes), the
  interface's size, the app icon in ten tints (graphite by default, shown
  in the Dock and the Finder), and buttons in the icon's color rather
  than blue. Editor: the code's size, Tab's indent, and saving as soon as
  typing pauses. Model: whether to render as you type and after what
  pause, and the model's color. Export: the folder exports go to, the
  format of the Export shortcut, showing each file exported in the
  Finder, and the STL and glTF options. Libraries: folders searched for
  libraries and for `text()`'s fonts before the default ones. Shortcuts:
  the keys of each action. The grid, the axes and the layer height are
  kept as left.
- **Animation**: a design that reads `$t`, the animation time from 0 to
  1 as in the reference implementation, gets a bar along the bottom of the
  view to play it: play and pause, the time's slider, frames a second and steps
  from 0 to 1. Each frame renders once and stays in memory, so the second
  time round plays at full speed; an edit renders the frames again.
  `c2f -D '$t=0.25'` and the MCP tools' `parameters` (`{"$t": 0.25}`)
  set the time.
- **Customizer**: the parameters a design declares in comments
  (`width = 20; // [10:100]`) get sliders, dropdowns, checkboxes and text
  fields, grouped by `/* [Group] */`, on a card over the view's top right
  that folds to its title; presets are saved next to the design in a
  `.json` file.
- **Geometry**: CSG with [Manifold](https://github.com/elalish/manifold)
  (through its Rust port), `text()` with real font shaping, `import()` of
  STL, OFF, OBJ, AMF, 3MF, DXF and SVG (and STEP, see
  [Other programs](#other-programs-obj-step-urdf-json)), `surface()`
  from `.dat` and PNG.
- **Meshes**: Open… takes an STL, 3MF, OBJ, OFF, AMF or STEP file (or one
  dropped on the window or opened from the Finder) as a design to change:
  its size, turn and place are parameters in the customizer, each object
  of a 3MF or OBJ file a `part()` (`import("box.3mf", object = "Lid")`
  reads one), and comments show where to add or cut material. A mesh
  holds triangles, not the shapes it was drawn with, so it is changed
  around, not inside. A mesh that is not a closed solid (small holes,
  faces turned the wrong way or listed twice, as many downloads have) is
  repaired when it is imported, with a warning saying what was done;
  what cannot be repaired is shown, but left out of the model. File > Insert >
  Mesh… imports one on the cursor's line, and Insert > Design's Modules
  or Whole Design adds a `use <…>` or `include <…>` line at the top, its
  path relative to the design.
- **Export** of solids to STL and 3MF (in millimeters), checked with
  PrusaSlicer's own loader, and of 2D models to SVG and DXF, laid out as
  the reference implementation (release 2021.01) writes them. Export >
  DXF of the View… draws a 3D model's outline as the view shows it, at its
  size, for a laser cutter or a drawing (`c2f dxf --view front`).
- **Other programs**: solids export to OBJ, to faceted STEP for CAD
  programs, to URDF for robots, and their parts to JSON (see
  [Other programs](#other-programs-obj-step-urdf-json)).
- **Game engines**: solids export to glTF 2.0 (`.glb`), which Godot 4
  imports as it is and Unity through glTFast or UnityGLTF, with their
  colors as materials (see [Game engines](#game-engines-gltf)).
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
- **Command line**: `c2f` prints the AST, the CSG tree, an STL, a 3MF, a
  glTF, an OBJ, a STEP, the parts' JSON, an SVG or a DXF (or writes them,
  and a URDF, with `-o file`), and takes `-D name=value`, `-p presets.json`
  and `-P preset`; `c2f pins` prints a part's annotations, and `c2f circuit`
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
| `electrify` | Makes an approved design (`path`) ready for its electronics: its circuit file beside it, `// @circuit`, its drawing moved unchanged into `design()`, and empty `electronics_cuts()` and `electronics_mounts()`; reports the volume before and after |
| `section` | Where a plane across `axis` (x, y or z) at `at` cuts the solid: its outlines at their real size, with area and bounds. To measure a mesh before writing it again as a design: a cut through the axis of a turned part is its profile |
| `export` | Writes a solid as STL, 3MF, glTF (`.glb`, with `scale` in meters per unit), OBJ (and its `.mtl`), STEP, URDF (and its meshes, with `density`) or its parts as JSON, a 2D model as SVG or DXF, or a 3D model's outline seen from `view` as DXF, to `output`, in the `format` its extension gives |

The server also offers two prompts that lead an agent from an idea to a
printed object with its electronics: `design_object` (the model first,
until you approve it) then `add_electronics` (see the
[guide](docs/electronics.md#from-an-idea-to-a-build)).

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

## Game engines (glTF)

File > Export > glTF… (or `c2f glb design.c2f > design.glb`, or the MCP
`export` tool) writes the model as a binary glTF 2.0 file, the open
format Godot 4 imports as it is and Unity through the glTFast or
UnityGLTF packages. The file passes the Khronos glTF-Validator.

- **Size**: glTF counts in meters. By default a millimeter stays a
  millimeter, so a 40 mm cube is 4 cm in the engine. Settings > Export >
  glTF models (or `--scale 1`, the `scale` argument of the MCP tool)
  makes each unit a meter instead, for levels and scenery modeled in
  meters.
- **Axes**: glTF has Y up where a design has Z up. The model is turned a
  quarter turn about X: the design's Z is the engine's Y, and its Y the
  engine's -Z, so the model stands as in the app.
- **Colors**: each `color()` becomes a material (PBR, not metallic), and
  a color with an alpha below 1 blends. The parts without `color()` take
  the model color picked in the settings, as do the faces a
  `difference()` cuts, which the preview shows in green.
- **Shading**: normals are smoothed across the facets of a curved surface
  and kept apart where faces meet at more than 35°, so a cylinder reads
  round and a cube keeps its edges.

### Parts

STL and 3MF exports unite the whole model into one solid. `part("name")`
keeps what it holds apart from the rest: in a glTF it is a node of its
own, named, that a game moves or animates.

```
cube([40, 40, 5]);
translate([20, 20, 5]) rotate([0, 0, 45]) part("turret") {
    cylinder(h = 10, r = 12);
    translate([0, 8, 5]) part("barrel") rotate([-90, 0, 0]) cylinder(h = 25, r = 2);
}
```

- A part's geometry is in its own frame; the `translate()`, `rotate()`,
  `scale()` and `mirror()` written around it become the node's
  translation, rotation and scale. A shear has no glTF equivalent: the
  node takes the nearest rotation, with a warning.
- A part in a part is a child node, and follows its parent. What is
  outside any part is the root node, named after the file. Names stay
  unique: a second part named `lid` is `lid 2`.
- A part inside `difference()`, `intersection()`, `hull()`,
  `minkowski()` and the other operations that reshape it is merged into
  the geometry around it, with a warning. `union()` and `color()` keep
  it apart; it takes the color it is drawn in.
- A part without geometry is an empty node, a pivot to hang other parts
  on.
- STL, 3MF and the app's view draw the parts as the rest of the model,
  and `c2f csg` prints them as `group()`. The reference implementation
  ignores `part()` and what it holds, with a warning; a file that defines
  `module part(name) children();` opens there whole. A module of your own
  named `part` wins over the built-in one.

### Animation

A design that reads `$t` exports animated: its parts move as `$t` goes
from 0 to 1, as the app's playback bar shows them.

```
cube([40, 40, 5]);
translate([20, 20, 5]) rotate([0, 0, 360 * $t]) part("turret") {
    cylinder(h = 10, r = 12);
    translate([0, 8, 5]) part("barrel") rotate([-90, 0, 0]) cylinder(h = 25, r = 2);
}
```

- Each part's geometry is built once, at `$t = 0`. The program is then
  evaluated at each frame, without building any geometry, for where each
  part is: its translation, rotation and scale become a track of the
  glTF animation, interpolated linearly, for each part that moves.
- The animation lasts 2 seconds, sampled 30 times a second, both ends
  included (Settings > Export > glTF models, `--duration` and `--fps`,
  or the MCP tool's `duration` and `fps`). It is named after the file
  (`--animation`, `animation`), and loops: its name then ends in `-loop`,
  which Godot reads to play it over and over (`--no-loop`, `"loop":
  false` play it once). `--still` (`"animate": false`) writes the model
  still, at `$t = 0` or at the `$t` set with `-D` or `parameters`. An
  animation has at most 10,000 frames, each an evaluation of the
  program.
- For the loop to be seamless, a part should be where it started at
  `$t = 1`, as `360 * $t` or `sin(360 * $t)` are.
- Only movement is animated. A part whose shape changes with `$t`, or
  geometry outside any part that does, keeps its shape at `$t = 0`, with
  a warning; so do parts that come and go. A model that reads `$t`
  without any `part()` exports still, with a warning.

### In Godot 4 and Unity

`examples/game/chest.c2f` is a treasure chest whose lid opens and closes
while a gem on it turns: three nodes (`chest`, `lid`, `gem`), two
materials and a transparent one, and an animation that loops. Next to it,
`gears.c2f` has three gears of 12, 24 and 18 teeth, each turning at the
speed its teeth set, and `walker.c2f` a mannequin whose limbs are parts
in the parts they hang from (hips, chest, upper arm, forearm...), walking
on the spot.

**Godot 4** (checked with 4.7): copy the `.glb` into the project. Godot
imports it as a scene: open it, or drag it into another scene. The nodes
keep their names and hierarchy, the materials their colors, and an
`AnimationPlayer` holds the animation, set to loop, under the file's name
without `-loop` (`$AnimationPlayer.play("chest")`). Godot simplifies
animation tracks on import with tolerances counted in meters: a movement
of a few millimeters comes out rounded (a 2 mm rise of a real-size part
came out 1.2 mm high), while turns and movements of a centimeter or more
keep their shape. For a small object with small movements, export with a
unit as a meter and scale the node in Godot.

**Unity**: install the glTFast package (Package Manager > Add package by
name > `com.unity.cloud.gltfast`) or UnityGLTF, then drop the `.glb` into
`Assets`. The model comes in as a prefab with its nodes and materials,
and its animation as a clip; in glTFast's import settings, *Animation
Method: Mecanim* makes it a clip an `Animator` plays. This path is not
yet checked in Unity itself.

**Triangles**: a game draws every triangle every frame. Curves are split
as `$fn`, `$fa` and `$fs` say; the export prints the count (and the app
shows it under the view). Keep `$fn` low on small curves: `$fn = 8` for a
gem, 24 to 48 for a wheel or a barrel.

## Other programs (OBJ, STEP, URDF, JSON)

Each is in Export, `c2f <format>` and the MCP `export` tool.

- **OBJ** (Blender, Maya, game engines): in millimeters, Z up as
  designed, an object for the root and for each `part()`, placed where
  the model puts it, and a material for each color in a `.mtl` beside it
  (`c2f obj -o model.obj design.c2f`). `import("model.obj")` reads it back.
- **STEP** (Fusion 360, SolidWorks, FreeCAD, Onshape): AP214, in
  millimeters, in the model's colors. Code2Form builds triangles, not
  exact surfaces, so the solids are faceted: the triangles in one plane
  and one color become one face, holes its inner loops, so that a cube
  has six faces to pick, measure and extrude, and a cylinder stays a
  prism of facets. Each closed shell is a solid, a shell inside another a
  cavity. FreeCAD reads the examples back as valid solids of the same
  volume.
- **STEP import**: `import("motor.step")` (or `.stp`, or Open…) asks
  FreeCAD's command-line program to mesh it, when FreeCAD is installed
  (found on the `PATH`, where its installers put it, or through
  `C2F_FREECAD`); facets turn by at most `$fa`, or a `$fn`th of a turn.
  The mesh is cached, so the design renders again at once.
- **URDF** (ROS, RViz, Gazebo, MuJoCo, PyBullet, Isaac Sim): the root and
  each `part()` are links, their geometry a binary STL in meters in a
  folder beside the `.urdf`, named relative to it (PyBullet, MuJoCo and
  URDF viewers read that as it is; in a ROS package, make them
  `package://` paths), their mass, center of mass and inertia
  from their volume and a density (PLA's, 1240 kg/m³, by default:
  `--density`). A part is joined to the part it is written in at the
  origin of its own frame, fixed unless `part()` says how it moves:

  ```
  part("base") cylinder(r = 20, h = 10);
  translate([0, 0, 10]) part("shoulder", joint = "continuous") {
      cylinder(r = 8, h = 10);
      translate([0, 0, 10]) rotate([90, 0, 0])
          part("elbow", joint = "revolute", axis = [0, 0, 1], limits = [-90, 45])
              cube([5, 30, 5]);
  }
  ```

  `joint` is `revolute` (between `limits`, in degrees), `continuous` (a
  wheel), `prismatic` (sliding between `limits`, in millimeters) or
  `fixed`; `axis` is in the part's own frame, Z by default. Other
  exports ignore the joint (`c2f urdf -o arm.urdf arm.c2f`).
- **JSON** (a bill of materials, a configurator, an agent): the
  `code2form-parts` format, version 1, in millimeters: the design's
  parameters, its bounds and volume, and for the root and each `part()`
  its name, its parent, its place (`matrix` in its parent, `world` in the
  model, row by row), its joint, bounds, volume, triangles and colors.

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
cargo run --release -p c2f-cli -- glb design.c2f > design.glb  # game engines
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
| `crates/geom` (`c2f-geom`) | Meshes, booleans, extrusions, text, imports, STL, 3MF, glTF, OBJ, STEP, URDF, JSON, SVG and DXF exports, PNG pictures |
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
