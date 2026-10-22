# Electronics: describe a part, wire a circuit

Code2Form models the electronics of a build next to its printed parts.
A part says where its pins, mount holes and ports are. A circuit file
places parts and wires their pins. Code2Form then checks the wiring,
draws it in 3D, walks you through soldering it, and gives an enclosure
the standoffs and openings its parts need.

This guide starts with a small part, then wires it, then follows the
repository's full example, Grogu: `examples/grogu/grogu.c2fc` and its
plate, `examples/grogu/grogu_plate.c2f`.

The commands below are written `c2f`. From a checkout, run them as
`cargo run --release -p c2f-cli -- ...` from the repository's root.

## 1. Annotate a part

A part is an ordinary `.c2f` file that draws the part. Annotations are
`//` comments that start with `@pin`, `@mount_hole` or `@port`. They use
the part's own coordinates, in millimeters. The reference implementation
reads them as plain comments, so the file renders as before.

Take a light sensor breakout: a 20 x 15 mm board, 1.6 mm thick, four
pins along its front edge, two mount holes and the sensor on top.

### Draw the board

```scad
// A light sensor breakout: a 20 x 15 mm board, 1.6 mm thick, four pins
// along its front edge, two mount holes and the sensor on top.

module light_sensor() {
    t = 1.6;
    difference() {
        color("DarkBlue") cube([20, 15, t]);
        for (x = [2.5, 17.5])
            translate([x, 12.5, -1]) cylinder(d = 2.2, h = t + 2, $fn = 24);
    }
    color("Black") translate([8.5, 7.5, t]) cube([3, 3, 0.8]);
}

light_sensor();
```

### Add the pins

One `@pin` per pin: a name, then its position `[x, y, z]`, where the
wire is soldered (here, on top of the board, `z = 1.6`).

```scad
// @pin VIN [6.19, 2, 1.6] power_in 3.6V "1.8 to 3.6 V"
// @pin GND [8.73, 2, 1.6] ground
// @pin SCL [11.27, 2, 1.6] input 3.6V "I2C clock"
// @pin SDA [13.81, 2, 1.6] io 3.6V "I2C data"
```

After the name and the position, fields come in any order:

| Field | Form | Default |
|---|---|---|
| Direction, the way the wire leaves | `up`, `down`, `+x`, `-x`, `+y`, `-y`, `+z`, `-z`, or a vector `[x, y, z]` | `up` |
| Type | `io`, `input`, `output`, `power_in`, `power_out`, `ground`, `passive`, `nc` | `passive` |
| Voltage | a number and `V`, as `3.3V`: the most the pin takes, or what it supplies for `power_out` | none |
| Label | a `"string"`, shown in the app and the wiring list | none |
| `reserved` | a pin the board needs for itself, such as a strapping pin | not reserved |

A pin name cannot contain `.`, `,` or `=`: a circuit calls the pin
`part.pin`. Each name is used once per part.

The checks only know what you declare. Give power pins their type and
voltage, and ground pins `ground`: that is what catches a 5 V wire on a
3.3 V pin.

### Add the mount holes

A `@mount_hole` has no name. Its position may be `[x, y]` (at `z = 0`)
or `[x, y, z]`, and it needs its diameter `d=`. Its direction is the
side the part rests on: `down` unless you say otherwise.

```scad
// @mount_hole [2.5, 12.5] d=2.2
// @mount_hole [17.5, 12.5] d=2.2
```

### Add the ports

A `@port` is an opening the enclosure must leave: a connector, a
sensor's window, a shaft. It has a name, the center of the opening, the
way it faces (`up` by default), and its size: `[width, height]` or a
round `d=`. A label is optional.

```scad
// @port window [10, 9, 2.4] up d=3 "sensor window"
```

A port `[width, height]` facing sideways is `width` across and `height`
up; facing up or down, `width` runs along the part's x and `height`
along its y. The XIAO's USB-C is a sideways one:
`// @port usb_c [8.9, 21, 2.83] +y [8.94, 3.26] "USB-C"`.

### Check what you wrote

Put the annotations at the top of the file and save it as
`light_sensor.c2f`. `c2f pins` prints what Code2Form read, one per line
with its line number, every field spelled out:

```console
$ c2f pins light_sensor.c2f
4: @pin VIN [6.19, 2, 1.6] up power_in 3.6V "1.8 to 3.6 V"
5: @pin GND [8.73, 2, 1.6] up ground
6: @pin SCL [11.27, 2, 1.6] up input 3.6V "I2C clock"
7: @pin SDA [13.81, 2, 1.6] up io 3.6V "I2C data"
8: @mount_hole [2.5, 12.5, 0] d=2.2 down
9: @mount_hole [17.5, 12.5, 0] d=2.2 down
10: @port window [10, 9, 2.4] up d=3 "sensor window"
```

A badly written annotation is reported on its line, and `c2f pins`
fails. In the app it is marked in the editor, and the model still
renders. With a space in `3.6 V`, the pin is left out and the error
follows the annotations read:

```console
$ c2f pins light_sensor.c2f
5: @pin GND [8.73, 2, 1.6] up ground
6: @pin SCL [11.27, 2, 1.6] up input 3.6V "I2C clock"
7: @pin SDA [13.81, 2, 1.6] up io 3.6V "I2C data"
8: @mount_hole [2.5, 12.5, 0] d=2.2 down
9: @mount_hole [17.5, 12.5, 0] d=2.2 down
10: @port window [10, 9, 2.4] up d=3 "sensor window"
error: unknown pin field `3.6`: expected a direction (up, down, +x, -y, [x, y, z]), a type (io, input, output, power_in, power_out, ground, passive, nc), a voltage like 3.3V, reserved or a "label"
 --> light_sensor.c2f:4:37
  |
4 | // @pin VIN [6.19, 2, 1.6] power_in 3.6 V "1.8 to 3.6 V"
  |                                     ^^^
```

`c2f check` reports the same errors with the syntax errors of the file.

Positions and sizes are plain numbers. They do not follow the file's
variables: if a parameter moves a pin, the annotation stays where the
default parameters put it.

### The library's parts

`library/electronics` holds annotated parts ready to use: the XIAO
ESP32-S3 and the ESP32-S3-DevKitC-1, DRV8833, MAX98357A and INMP441
modules, capacitors, a speaker, a DC motor and a touch plate (see
`library/README.md`). Read their pins the same way:

```console
$ c2f pins library/electronics/dc_motor.c2f
19: @pin + [0, -5, 7.55] -x passive 6V "terminal marked + or red"
20: @pin - [0, 5, 7.55] -x passive 6V
21: @mount_hole [25.1, -6, 7.55] d=2 +x
22: @mount_hole [25.1, 6, 7.55] d=2 +x
23: @port shaft [25.1, 0, 7.55] +x d=6 "shaft and bearing boss"
```

## 2. Write a circuit

A circuit is a `.c2fc` file, in TOML. It places parts and lists the
wires between their pins. Save this next to `light_sensor.c2f` as
`lamp.c2fc`:

```toml
version = 1

[parts.xiao]
part = "electronics/xiao_esp32s3.c2f"

[parts.sensor]
part = "light_sensor.c2f"
at = [30, 0, 0]

[[wires]]
from = "xiao.3V3"
to = "sensor.VIN"
color = "red"
label = "3.3 V"

[[wires]]
from = "xiao.GND"
to = "sensor.GND"
color = "black"

[[wires]]
from = "xiao.D5"
to = "sensor.SCL"
color = "yellow"
label = "I2C"

[[wires]]
from = "xiao.D4"
to = "sensor.SDA"
color = "blue"
label = "I2C"
```

### The keys

| Key | What it is |
|---|---|
| `version` | `1`, optional |
| `enclosure` | the `.c2f` file of the enclosure made for the circuit, optional |
| `[parts.NAME]` | a part, by the name wires call it by (no `.`, `,`, `=` or spaces) |
| `part` | its `.c2f` file, required |
| `at` | where its origin goes, `[x, y, z]` in millimeters, as `translate()`; `[0, 0, 0]` by default |
| `rotate` | degrees about x, then y, then z, as `rotate()`, applied before `at`; `[0, 0, 0]` by default |
| `params` | values for the part's variables, as `c2f -D name=value` sets them: `{ headers = true }` |
| `color` | the color the app shows the part in; otherwise each part takes the next color of a palette |
| `[[wires]]` | a wire, in the order you will lay them |
| `from`, `to` | two pins, as `part.pin`, required |
| `color` | the wire's color: a CSS name (`red`, `orange`, `purple`...) or `#rrggbb` |
| `label` | what the wire carries, shown in the legend and the wiring list |
| `allow` | warnings accepted for the pins at its two ends: `["reserved"]`, `["two_outputs"]`, `["common_ground"]` (see [Check the wiring](#3-check-the-wiring)) |

A part's file is found next to the circuit first, then in the library
folders (those in `OPENSCADPATH`, then the library the app ships). That
is why `electronics/xiao_esp32s3.c2f` needs no path. The file's own
customizer parameters can be set with `params`, as Grogu does for the
amplifier's headers and the capacitor's size.

The JSON Schema of the format is `crates/circuit/circuit.schema.json`.
Point your editor's TOML support at it to get completion.

A wrong pin, an unknown part, a file not found or a bad value is an
error on its line, as for a `.c2f` file:

```console
$ c2f circuit check lamp.c2fc
error: `sensor` (light_sensor.c2f) has no pin named `SDO`
  --> lamp.c2fc:29:6
   |
29 | to = "sensor.SDO"
   |      ^^^^^^^^^^^^
```

### Grogu

`examples/grogu/grogu.c2fc` is a complete circuit: a XIAO ESP32-S3 that
listens (INMP441 microphone), speaks (MAX98357A amplifier and a
speaker), moves (DRV8833 driver and a DC motor) and feels a touch (a
copper plate). Read it top to bottom: it is commented. A few patterns
worth copying:

```toml
# The PETG plate the parts sit on, in this file's coordinates.
enclosure = "grogu_plate.c2f"

# The XIAO's USB-C faces the front wall (-y).
[parts.xiao]
part = "electronics/xiao_esp32s3.c2f"
at = [17.8, 21, 0]
rotate = [0, 0, 180]

# Stands on its header's pins.
[parts.amp]
part = "electronics/max98357a.c2f"
at = [-30, 0, 8.5]
params = { headers = true }

[parts.cap]
part = "electronics/capacitor_radial.c2f"
at = [-20, 26, 0]
params = { d = 8, h = 12 }

# Power first, then signals.
[[wires]]
from = "xiao.5V"
to = "amp.VIN"
color = "red"
label = "5 V"
```

Its wires go in the order you solder them: the 5 V rail and ground,
the 3.3 V supply of the microphone, then the I2S buses, the motor and
the touch plate. Several wires may reach one pin (`amp.VIN` takes three):
the app fans them apart.

## 3. Check the wiring

`c2f circuit check` reads the circuit, evaluates its parts and checks
the wiring against what the parts declare. Run it before soldering.

```console
$ c2f circuit check lamp.c2fc
lamp.c2fc: 2 parts, 4 wires: no errors
```

Wire the sensor to `xiao.5V` instead of `xiao.3V3`:

```console
$ c2f circuit check lamp.c2fc
error: `xiao.5V` supplies 5 V to `sensor.VIN`, which takes at most 3.6 V
  --> lamp.c2fc:11:8
   |
11 | from = "xiao.5V"
   |        ^^^^^^^^^
lamp.c2fc: 2 parts, 4 wires: 1 error
```

A net is the set of pins wires connect. The ground pins of one part
count as one net, as boards tie them together. Each finding points at
the wire's `from` in the circuit file, or at the part's name.

Errors fail the check (exit status 1):

| Rule | Reports |
|---|---|
| Supply above what a pin takes | "`xiao.5V` supplies 5 V to `sensor.VIN`, which takes at most 3.6 V": a `power_out` pin's voltage is above the voltage of another pin on its net (ground and other supplies aside) |
| Two supplies tied together | "`a` (5 V) and `b` (3.3 V) are tied together: two supplies of different voltages" |
| Supply on ground | "`a` is a supply wired to the ground `b`: a short circuit" |
| Supply or ground left open | "`sensor.VIN` is not wired, though other pins of `sensor` are: a supply it needs is left open" (or "its ground"), for a `power_in` pin, or all the ground pins, of a part that has other pins wired |
| Wired `nc` pin | "`a` is not connected (nc): nothing may be wired to it" |

Warnings are reported but pass:

| Rule | Reports |
|---|---|
| `allow` name | Rule | Reports |
|---|---|---|
| `common_ground` | No common ground | "no common ground: `b` is not connected to `a`": the wired parts' grounds are not all one net |
| `reserved` | Wired `reserved` pin | "`xiao.D2` is reserved by its part (its label): better left unwired" |
| `two_outputs` | Two outputs on one net | "`a` and `b` are two outputs on one net: they drive it against each other" |

When you know a warning is fine, accept it on a wire: `allow =
["reserved"]` silences that warning for the pins at the wire's two
ends, and only there. Say why in a comment above it. Errors cannot be
allowed: an unknown name, or the name of an error, is an error.

A part with no pin wired is left out of the checks: a part you have not
wired yet is not an error.

Grogu's motor uses D2, a strapping pin of the ESP32-S3, because D6 and
D7 are the serial port. GPIO3 only matters at boot once its JTAG eFuse
is burned, so the wire accepts the warning:

```toml
# D2 is GPIO3, a strapping pin: it only matters at boot once the JTAG
# eFuse is burned, and the driver's input pulls it low, which is safe.
[[wires]]
from = "xiao.D2"
to = "driver.IN2"
color = "purple"
allow = ["reserved"]
```

```console
$ c2f circuit check examples/grogu/grogu.c2fc
examples/grogu/grogu.c2fc: 8 parts, 23 wires: no errors
```

The app runs the same checks each time the circuit changes, and prints
them in the console. The line of the first error is marked in the
editor.

## 4. See it in the app

Open a `.c2fc` file like any design, with File > Open or from the
command line:

```sh
cargo run --release -p code2form -- examples/grogu/grogu.c2fc
```

The editor shows the circuit's text. The 3D view shows each part where
the circuit places it, and the wires on top. Edit the circuit as you
would a design: only the parts an edit changes render again.

### Wires and legend

Each wire is a tube in its color, from pin to pin. It leaves each pin
the way the pin's direction points, then curves to the other end. A
wire without a color, or with one not understood, is light grey. Pins
show as dots: wired pins larger and lighter. Wires are drawn, not
modeled: they never reach an export.

The legend, at the top right of the view, lists each color with the
labels of its wires: `red` 5 V, decoupling, keep the driver awake.
Under it, **Wire margin** sets what is added to each wire's length for
stripping and slack (10 mm by default, kept between sessions): a wire's
bubble and card give its length, as "About 32 mm of wire: 22 mm routed
+ 10 mm margin".

### Enclosure, collisions and cut

When the circuit names an `enclosure`, the app draws it see-through
around the parts; the **Enclosure** button in the toolbar hides and
shows it. It follows the circuit as you type, before you save.

A part that runs into the enclosure or into another part turns red, and
the console says by how much, as "Collision: `touch` runs into the
enclosure by 120.00 mm³". Parts that only touch are fine.

To look inside, turn on **Layers**: the slider cuts the parts, the
enclosure and the wires at a height.

### Hover, click, double-click

- **Hover** a pin, a wire or a part: a bubble names it, as "xiao · 5V ·
  VUSB: USB 5 V; or 5 V in" or "Red wire · 5 V: xiao.5V ➡ amp.VIN".
- **Click a pin**: its whole net stands out and the other wires grey
  out. A card gives the pin's type, voltage and `reserved`, and the
  other pins of its net (for `xiao.5V`, "Net: amp.VIN, cap.+, driver.EEP, driver.VCC
  (4 wires)"), or "Not wired".
- **Click a wire**: it stands out with its two pins, and the editor
  jumps to its line in the circuit file.
- **Click a part**: it stands out with its wires; the card gives its
  file and params.
- **Double-click a part**: opens its `.c2f` file, to fix an annotation.
- **Escape** clears the selection.

### Rewire with the mouse

The view edits the wiring too, by writing into the circuit file: only
the values or the table concerned change, and the file keeps its
comments, order and layout. Each gesture is one step to undo (Cmd+Z in
the editor).

- **Move a wire's end**: press on the wire near one of its ends (its
  first or last third, not on the pin's dot) and drop it on another pin.
  That end's `from` or `to` changes; its color, label and `allow` stay.
- **Add a wire**: press on a pin and drop on another. A `[[wires]]`
  table with `from` and `to` goes at the end of the file, last in the
  mount order; give it a color and a label in the editor.
- **Remove a wire**: drop its end away from any pin, or click the wire
  and press **Delete** (or Backspace). Its `[[wires]]` table goes, from
  its header to its last key; comments before the next table stay.

While you drag, the pin under the pointer stands out. When dropping
there would break a check (5 V onto a 3.3 V pin, two supplies tied), it
turns red and the bubble says why; you can still drop. A drag that
starts elsewhere turns the camera as usual. Rewiring is off in Assembly
mode.

### Assembly mode

The **Assembly** button in the toolbar (for a circuit with wires) walks
you through the wiring, one wire at a time, in the file's order.

A bar over the view gives the wire to lay, as "Red wire: xiao 5V (USB 5
V; or 5 V in) ➡ amp VIN (2.5 to 5.5 V) · 5 V". The camera turns to its
two pins, which pulse. The wires done show as they are, and the wires
still to lay grey out. Then:

- **Next ▶** or the **Right arrow** checks the wire off and goes to the
  next.
- **◀ Previous** or the **Left arrow** goes back.
- The checkbox checks a wire off, or unchecks it.
- **Start over** unchecks every wire and goes back to the first.
- A click on a wire in the view goes to its step.

The bar counts the wires done. Progress is saved next to the circuit:
`grogu.c2fc` keeps it in `grogu.mount.json`. Wires are named there by
their ends, as `xiao.5V -> amp.VIN`, so reordering the file keeps what
is done. Delete the file to start afresh, or press Start over.

## 5. Build an enclosure around it

A `.c2f` file that names its circuit in a comment is drawn in the
circuit's coordinates and gets four built-in modules that follow its
parts:

```scad
// @circuit lamp.c2fc
```

The path is relative to the file. Move a part in the circuit, and its
standoffs and cutouts follow at the next render.

| Module | Makes |
|---|---|
| `standoffs_for(part, height, d, hole, screw)` | a standoff under each mount hole of the part, `height` long, along the hole's direction |
| `holes_for(part, depth, hole, screw)` | a clearance hole at each mount hole, from 1 mm behind it, `depth` long (10 by default), to subtract |
| `cutout_for(part, port, clearance = 0.5, depth = 10)` | the port's opening grown by `clearance`, from 1 mm behind it along the way it faces, to subtract from a wall |
| `circuit_part(part)` | the part itself, with its params, where the circuit places it |

`screw = "M2"`, `"M2.5"` or `"M3"` sizes a standoff and its pilot hole
for a screw that cuts its thread in the plastic, and a clearance hole
for that screw. Without it, they follow the part's hole; `d` and `hole`
set them directly.

A lid for the lamp's sensor, with a hole over its window and two
standoffs under its holes:

```scad
// @circuit lamp.c2fc

// A lid 3 mm above the sensor's top, with a hole over its window.
difference() {
    translate([25, -5, 5]) cube([30, 25, 2]);
    cutout_for("sensor", "window", clearance = 0.5);
}
// Standoffs under the sensor's mount holes, for M2 screws.
standoffs_for("sensor", height = 5, screw = "M2");
%circuit_part("sensor");
```

`circuit_part()` also derives shapes from a part, as Grogu's plate
draws a fence around each board:

```scad
module fence_for(part, height) {
    linear_extrude(height = height)
        difference() {
            offset(delta = clearance + fence) projection() circuit_part(part);
            offset(delta = clearance) projection() circuit_part(part);
        }
}
```

`examples/grogu/grogu_plate.c2f` uses all four: fences, an opening for
the XIAO's USB-C, standoffs and screw holes for the motor on the right
wall, and a grille cut where the speaker's cone faces.

Without `@circuit`, or with a part or port the circuit does not have,
the modules warn and draw nothing. The reference implementation ignores
them as unknown modules.

## 6. From the command line

```sh
c2f pins part.c2f                       # the annotations a part declares
c2f check part.c2f                      # syntax errors and badly written annotations
c2f circuit check grogu.c2fc            # the wiring checks; fails on an error
c2f circuit wires grogu.c2fc            # the wiring list, in the order to lay the wires
c2f circuit wires --csv grogu.c2fc      # the same, as CSV
c2f circuit bom grogu.c2fc              # the bill of materials
c2f circuit bom --csv grogu.c2fc
c2f circuit stl grogu.c2fc > grogu.stl  # the parts assembled (not the wires)
c2f circuit 3mf grogu.c2fc > grogu.3mf
```

The wiring list, to print and solder from (the first rows of Grogu's):

```console
$ c2f circuit wires examples/grogu/grogu.c2fc
step  from                                          to                                        color   length  label
   1  xiao.5V (VUSB: USB 5 V; or 5 V in)            amp.VIN (2.5 to 5.5 V)                    red      32 mm  5 V
   2  xiao.GND                                      amp.GND                                   black    35 mm
   3  cap.+ (longer lead)                           amp.VIN (2.5 to 5.5 V)                    red      44 mm  decoupling
   4  cap.- (striped side)                          amp.GND                                   black    43 mm
```

The length is that of the curve drawn in the app, from pin to pin, plus
a 10 mm margin for stripping and slack: `--margin 0` gives the bare
curve, `--margin 20` more slack. `c2f circuit check` also reports, as
warnings, the parts that run into each other or into the enclosure.

The bill of materials groups the parts by file and params:

```console
$ c2f circuit bom examples/grogu/grogu.c2fc
name              quantity  file                              params        instances
xiao_esp32s3             1  electronics/xiao_esp32s3.c2f                    xiao
max98357a                1  electronics/max98357a.c2f         headers=true  amp
capacitor_radial         1  electronics/capacitor_radial.c2f  d=8; h=12     cap
speaker                  1  electronics/speaker.c2f                         speaker
inmp441                  1  electronics/inmp441.c2f                         mic
drv8833                  1  electronics/drv8833.c2f                         driver
dc_motor                 1  electronics/dc_motor.c2f                        motor
touch_plate              1  electronics/touch_plate.c2f                     touch
```

With `--csv`, the wiring list's columns are
`step,from,from_label,to,to_label,color,label,length_mm`, and the bill
of materials' `name,quantity,file,params,instances`.

## 7. With an AI agent

`c2f mcp` is a Model Context Protocol server. Add it to Claude Code
with `claude mcp add code2form -- c2f mcp` (see the README's
[AI agents](../README.md#ai-agents-mcp) section). Its electronics tools:

| Tool | Does |
|---|---|
| `library_parts` | lists the library's electronics parts, by the name a circuit gives as `part` |
| `part_pins` | a part's pins, mount holes, ports and badly written annotations, from `code` or `path` |
| `check_circuit` | the wiring checks, with `ok` false on an error |
| `circuit_image` | a PNG of the circuit with its wires; `net` (as `xiao.GND`) highlights one net |
| `circuit_wires` | the wiring list |
| `circuit_bom` | the bill of materials |

The circuit tools take the circuit as `code` (its parts found from
`working_dir`, then the library) or as a file `path`. A typical request:
"With the Code2Form tools, build a `robot.c2fc` circuit for a XIAO
ESP32-S3 that drives a DC motor through a DRV8833: look up the parts'
pins, check the circuit until it has no errors, and show me the
wiring." Then open `robot.c2fc` in the app and wire it in Assembly
mode.
