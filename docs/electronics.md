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

### Add its sheet

A part also carries its technical sheet, which the catalogue shows and
searches (section 4) and an agent reads (section 7): what it is, its
key figures and where they come from.

```scad
// @description Ambient light sensor on a 20 x 15 mm breakout board, read
// @description over I2C, with two mount holes.
// @spec supply: 1.8 to 3.6 V
// @spec interface: I2C
// @spec size: 20 x 15 x 1.6 mm
// @source https://example.com/light-sensor product page
// @source https://example.com/light-sensor.pdf datasheet
```

| Annotation | Form |
|---|---|
| `@description` | what the part is and does, a sentence or two; several lines make one description |
| `@spec` | `key: value`, one figure per line: supply, current, interface, size... Each key once |
| `@source` | a URL starting with `https://` or `http://`, then an optional label (plain or `"quoted"`) |

Their text runs to the end of the line. Give only figures your sources
state, and a `@source` for each page they come from. The parts of the
library each have one, from their maker's product page and datasheet.

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

`c2f pins` prints the sheet too, `@description` joined into one line.
`c2f check` reports the same errors with the syntax errors of the file:
a `@spec` without its `:`, a `@source` that is not a web address, a key
or a URL given twice.

Positions and sizes are plain numbers. They do not follow the file's
variables: if a parameter moves a pin, the annotation stays where the
default parameters put it.

### The library's parts

`library/electronics` holds annotated parts ready to use: the XIAO
ESP32-S3 and the ESP32-S3-DevKitC-1, DRV8833, MAX98357A and INMP441
modules, capacitors, a speaker, a DC motor, an N20 gear motor, a
28BYJ-48 stepper and a touch plate (see `library/README.md`). Each sheet
gives the part's mass, and a motor's torque and speed, from its maker
where one gives them. Read their pins the same way:

```console
$ c2f pins library/electronics/dc_motor.c2f
24: @description Small brushed DC motor, size 130 (Mabuchi FA-130 and its many copies): the motor of toys and small robots, run from a few volts.
27: @spec motor: brushed DC, size 130
28: @spec voltage: up to 6 V as annotated; Mabuchi's FA-130RA is rated 1.5 to 3 V, check your copy
29: @spec size: 20.1 x 15.1 x 25.1 mm without its boss
30: @spec shaft: 2 mm across
31: @spec mount: two 2 mm holes 12 mm apart on the front face, on some copies
32: @spec torque: 0.59 mN·m at 1.5 V, at its best efficiency (6990 rpm)
33: @spec stall torque: 2.55 mN·m at 1.5 V
34: @spec speed: 9100 rpm with no load at 1.5 V
35: @spec mass: 17 g
36: @source https://product.mabuchi-motor.com/detail.html?id=9 Mabuchi FA-130RA product page (ratings, mass, drawing)
42: @pin + [0, -5, 7.55] -x passive 6V "terminal marked + or red"
43: @pin - [0, 5, 7.55] -x passive 6V
44: @mount_hole [25.1, -6, 7.55] d=2 +x
45: @mount_hole [25.1, 6, 7.55] d=2 +x
46: @port shaft [25.1, 0, 7.55] +x d=6 "shaft and bearing boss"
```

What turns and holds the motor's work is in `library/mechanical`: ball
bearings (608, 625, 6800, 6001, with their load ratings), a shaft, ISO
4762 screws and ISO 4032 nuts from M2 to M5, brass heat-set inserts with
the hole to print for each, a rigid shaft coupling and a parametric
involute spur gear, each with its mass and its sources. Two helpers model
what a mechanical review asks for: `bearing_seat("608", fit = "press")`
cuts a bearing's pocket with clearances chosen for FDM printing, and
`gear_pair(ratio = 3, mod = 1)` places two meshing gears at their center
distance and echoes the ratio they give. `library_parts` and the
catalogue list them after the electronics, and find them in French or
English (`roulement 8 mm`, `pignon`, `vis M3`).

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
| `via` | points the wire passes through on its way, `[[x, y, z], ...]` in the circuit's coordinates (see [Wires through the enclosure](#wires-through-the-enclosure)) |
| `allow` | warnings accepted for the pins at its two ends: `["reserved"]`, `["two_outputs"]`, `["common_ground"]` (see [Check the wiring](#3-check-the-wiring)) |

A part's file is found next to the circuit first, then in the library
folders (those in `C2F_LIBRARY_PATH`, then the library the app ships). That
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
WARNING: wire 3 (cap.+ -> amp.VIN) runs into the enclosure at [-21.6, 25.5, -1.8], through a face facing -z: route it around with `via`, or carve it a channel_for(3)
WARNING: wire 4 (cap.- -> amp.GND) runs into the enclosure at [-18.2, 25.5, -1.7], through a face facing -z: route it around with `via`, or carve it a channel_for(4)
examples/grogu/grogu.c2fc: 8 parts, 23 wires: no errors, 2 warnings
```

The two warnings come from the parts and the enclosure, not from the
wiring: the capacitor's leads leave it downwards, through the plate.
With an `enclosure`, the check also warns of a part that runs into
another or into the enclosure (by its volume, in mm³), and of a wire
whose curve runs into the enclosure: where its curve first is in the
material, and the way the face it meets faces (`-z`, or a vector for a
slanted wall). The wire's center line is checked, not its thickness.

### Does it fit?

`c2f circuit fit` measures the room each part has in the enclosure, in
millimeters, and warns of what will not go together. On
`crates/app/tests/fit/winder.c2fc`, a watch winder's XIAO, ULN2003
board and stepper motor in a box whose back wall has no opening for the
USB-C:

```console
$ c2f circuit fit winder.c2fc
part    clearance  enclosure  rests on  nearest part
xiao    3.50 mm    3.50 mm    0.00 mm   motor 9.39 mm
driver  6.00 mm    6.00 mm    0.00 mm   over 10 mm
motor   1.00 mm    1.00 mm    0.00 mm   xiao 9.39 mm
WARNING: no cutout_for in front of port `xiao.usb_c`: the enclosure covers it at [26.5, 36.0, 5.6]
winder.c2fc: 3 parts, 1 warning
```

| Column | Is |
|---|---|
| `clearance` | the least room around the part: the smaller of `enclosure` and `nearest part` |
| `enclosure` | the shortest distance from the part to the enclosure around it, `overlaps by` a volume when it runs into it |
| `rests on` | the distance to what the part rests on: the enclosure beyond the face of its mount holes (its bottom when it has none) |
| `nearest part` | the closest other part, and how far |

What the part rests on does not count against its clearance: a board
touches its standoffs, a motor its flange's wall. What it puts through
that support, as a motor's shaft through its hole, does: its room is
measured from 1 mm past the face it rests on. Further than 10 mm,
there is room enough and the distance is not measured (`over 10 mm`).

Warnings, which do not fail the command:

| Warning | When |
|---|---|
| "`motor` is 0.38 mm from the enclosure at [...], under the 0.5 mm clearance" | a part comes closer than the clearance to the enclosure or to another part; "touches" at 0, "runs into" or "overlap" when they share some volume |
| "no cutout_for in front of port `xiao.usb_c`: the enclosure covers it at [...]" | a wall is in front of most of a port's opening, within 5 mm of it |
| "no standoff under mount hole 2 of `driver` (at [...])" | the enclosure is nowhere within 1 mm along the hole's axis, around the hole |
| "wire 7 (driver.A -> motor.blue) runs into the enclosure at [...]" | as for `c2f circuit check` |

`--clearance 1` asks for 1 mm of room instead of 0.5 mm. "At [...]" is
where, in the circuit's coordinates, to the nearest tenth of a
millimeter.

The distances are exact for the meshes as rendered: the shortest
distance between the triangles of the two surfaces. A round face counts
as its facets, so set `$fn` on both sides for a tight fit: a 9 mm boss in
a 10 mm hole, both at `$fn = 48`, has 0.998 mm of room, not 1 mm. Where
two surfaces come closest is located to within the size of the smaller
of the two triangles there: on a large flat face, about where. Ports and
mount holes are probed with rays, a grid of 25 across a port's opening
(out to 80% of the way to its edge: a grille counts as open when most of
its rays pass) and a ring of 8 around a hole at 0.75 times its diameter,
between a standoff's bore and its outside.

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
around the parts; **Enclosure** in the **View** menu hides and
shows it. It follows the circuit as you type, before you save.

A part that runs into the enclosure or into another part turns red, and
the console says by how much, as "Collision: `touch` runs into the
enclosure by 120.00 mm³". Parts that only touch are fine. A part closer
than 0.5 mm to the enclosure or to another part turns amber, and the
console gives the warnings of [`c2f circuit fit`](#does-it-fit), with
the wires that run into the enclosure.

To look inside, turn on **Layers**: the slider cuts the parts, the
enclosure and the wires at a height.

Or turn on **Section** (the box cut open, among the view's tools): it
cuts the enclosure alone across **X**, **Y** or **Z**, where its slider
says, and the parts and wires stay whole inside it. The same card's
**Opacity** slider (also under **Enclosure** in the **Views** menu)
sets how see-through the enclosure is, from 0 to 100 % (28 % by
default); opaque, the cut runs through its walls in red. Neither
renders anything again. A design without a circuit gets the same
**Section** tool, which cuts the model.

The **Parts** card, at the bottom left of the view, lists each part in
the color it shows in: a click picks it out with its wires, as a click
on it in the view does, and a second click clears it.

For a screenshot, `CODE2FORM_SCREENSHOT_SELECT=cut:z:20` cuts the view
across z at 20 mm; add `,opacity:100` for an opaque enclosure, and a
part's name to pick it out.

### Catalogue

The **Catalogue** button in the top bar (for a circuit) opens a panel
of every part under `electronics/` in the library folders: the
library's, the app's and your own (`~/Documents/Code2Form/libraries/electronics`),
a part found in several listed once, from the first folder searched.
Each row shows a picture of the part, its name, the first sentence of
its `@description` (or of its opening comment) and its number of pins.

The search field ranks the parts that answer every word typed, best
first: a word in the name weighs more than in the description, then the
specs, then the pins' names and labels. Case and accents do not matter,
French and English both work, and it knows the domain's synonyms:
`micro I2S` finds the INMP441, `driver moteur` the DRV8833,
`haut-parleur` the speaker, `ampli` the MAX98357A.

Hover a part's name to see its sheet; click it to keep the sheet open
under its row: its description, its specs, its pins by type (power,
ground, inputs and outputs...) and its sources as links.

- **Drag a picture into the view**: the part goes where you drop it, on
  the ground (z = 0), to the nearest 0.1 mm. Dropping outside the view
  does nothing.
- **Add** places it 5 mm right of the circuit's parts, level with their
  front.

Either way, the app writes a `[parts.NAME]` table, with `part` and `at`,
after the circuit's last part, as one step to undo. NAME is the file's
name, `_2`, `_3`... when it is taken; rename it in the editor.

**New part...** asks for a name and makes `NAME.c2f` in your parts
folder from a template: a 20 x 15 x 1.6 mm board in a `/* [Size] */`
section of the customizer, two pins (`VCC`, `GND`) and a mount hole,
drawn by a module `NAME()`. It then opens it (asking first to save the
circuit if it changed), and the catalogue lists it.

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

### Rewire and move parts with the mouse

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
turns red and the bubble says why; you can still drop.

Parts move the same way:

- **Move a part**: press on it (not on a pin's dot or near a wire's
  end) and drag. It slides flat, at its own height, where you hold it,
  on a 0.5 mm grid (0.1 mm with **Shift** held); its wires follow. On
  release its `at` is rewritten, or added after its `part` line, with
  its height kept.
- **Turn a part**: **R** turns the part selected, or the one being
  dragged, a quarter turn about z: `rotate` gets 90° more on z, or
  `rotate = [0, 0, 90]` is added.
- **Remove a part**: click it and press **Delete** (or Backspace), or
  press **Remove with its wires** under its card. Its `[parts.NAME]`
  table goes, with every `[[wires]]` table to it; the console says how
  many, as "Removed `mic` and its 6 wires".

A press on a pin wins over a wire's end, which wins over a part. A drag
that starts on nothing turns the camera as usual. All of this is off in
Assembly mode.

### Assembly mode

The **Assembly** button in the top bar (for a circuit with wires) walks
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
| `channel_for(wire, d = 3)` | a tube `d` across along the wire's curve, its `via` points included, to subtract so that the wire passes |
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

### Wires through the enclosure

A wire is drawn as a curve from pin to pin, and `c2f circuit check`
warns when that curve runs into the enclosure. Two ways out: route it
around, or carve it a passage.

`via` lists points the wire passes through, in order, in the circuit's
coordinates. The curve still leaves each pin the way the pin points,
then runs through each point, turning smoothly there. The views draw it
so, and the wiring list's lengths count the detour. Over the left wall
of a box:

```toml
[[wires]]
from = "xiao.5V"
to = "driver.VCC"
color = "red"
label = "5 V"
via = [[-53.5, 0, 55]]
```

`channel_for(wire, d = 3)` makes a tube `d` across along the same curve,
to subtract from the enclosure: a passage the wire, 1 mm thick as
drawn, runs through without touching. `wire` is the wire's number, from
1 in the file's order (as the check's warnings and the app's `wire:7`
name it), or a label: every wire with that label, carved as a bundle.
The watch winder's motor cable runs through its back plate; its frame
file carves the five wires a passage:

```scad
// @circuit winder.c2fc
include <watch_winder.c2f>
difference() {
    frame();
    channel_for("motor cable", d = 4, $fn = 12);
}
```

A channel is a chain of spheres, hulled two by two, 24 to a stretch of
wire: under a file's `$fn = 64` it renders in seconds, with `$fn = 12`
in a fraction of one. A channel follows its wire: move a part or a
`via` point, and the channel moves at the next render.

## 6. From the command line

```sh
c2f pins part.c2f                       # the annotations a part declares
c2f check part.c2f                      # syntax errors and badly written annotations
c2f circuit check grogu.c2fc            # the wiring checks; fails on an error
c2f circuit wires grogu.c2fc            # the wiring list, in the order to lay the wires
c2f circuit wires --csv grogu.c2fc      # the same, as CSV
c2f circuit bom grogu.c2fc              # the bill of materials
c2f circuit bom --csv grogu.c2fc
c2f circuit fit grogu.c2fc              # each part's room in the enclosure, and what does not fit
c2f circuit fit --clearance 1 grogu.c2fc
c2f circuit stl grogu.c2fc > grogu.stl  # the parts assembled (not the wires)
c2f circuit 3mf grogu.c2fc > grogu.3mf
c2f mass grogu.c2fc                     # its masses, center of gravity and tipping angle
c2f mass --material petg --infill 30 --wall 1.2 grogu_plate.c2f
c2f loads winder.json                   # the loads on a shaft and its motor (section 7)
```

The wiring list, to print and solder from (the first rows of Grogu's):

```console
$ c2f circuit wires examples/grogu/grogu.c2fc
step  from                                          to                                        color   length  label
   1  xiao.5V (VUSB: USB 5 V; or 5 V in)            amp.VIN (Vin: 2.5 to 5.5 V)               red      32 mm  5 V
   2  xiao.GND                                      amp.GND                                   black    35 mm
   3  cap.+ (longer lead)                           amp.VIN (Vin: 2.5 to 5.5 V)               red      43 mm  decoupling
   4  cap.- (striped side)                          amp.GND                                   black    42 mm
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
| `library_parts` | lists the library's electronics parts, by the name a circuit gives as `part`, with their description, specs and sources; `query` ranks them as the catalogue's search does |
| `part_pins` | a part's pins, mount holes, ports and badly written annotations, from `code` or `path` |
| `check_circuit` | the wiring checks, with `ok` false on an error |
| `circuit_image` | a PNG of the circuit with its wires, in its enclosure drawn see-through (`enclosure` false leaves it out), or its sheet of views (`view` `sheet`, `section` cuts it open); `net` (as `xiao.GND`) highlights one net |
| `circuit_wires` | the wiring list |
| `circuit_bom` | the bill of materials |
| `fit` | how each part fits in the enclosure, as [`c2f circuit fit`](#does-it-fit) says it; `clearance` sets the room wanted |
| `check_part` | a part's annotations checked against its rendered shape (see below) |
| `save_part` | writes a part into your parts folder, then checks it (see below) |
| `mass_properties` | what a design or a circuit weighs printed, its center of gravity, and how far it leans before it tips (see below) |
| `loads` | the torque a motor must give a shaft, what each support takes, the shaft's deflection and the margins against the motor's and bearings' sheets (see below) |
| `save_brief` | writes the brief the plan ends on; refused without a requirement a tool measures |
| `read_brief` | a brief's text, requirements and `mass` and `loads` blocks |
| `check_brief` | measures a design and its circuit against their brief: each requirement holds, fails or is unmeasured |

The circuit tools take the circuit as `code` (its parts found from
`working_dir`, then the library) or as a file `path`. `check_circuit`
also gives, as warnings, the parts that run into each other or into the
enclosure and the wires that run into it.

`fit` answers in JSON, a part at a time: its `name`, its least
`clearance_mm` (null over 10 mm), `tight` when it is under `clearance`
or collides, `enclosure`, `rests_on` and `nearest` (each with
`distance_mm`, `overlap_mm3` and `at`, the nearest with its `part`; null
over 10 mm), its `covered_ports` and `loose_holes` with where, and its
`warnings`. Then `wires`, those that run into the enclosure (`wire`
from 1, `from`, `to`, `at`, `facing`), and all the `warnings`, word for
word those of `c2f circuit fit`. A typical request:
"With the Code2Form tools, build a `robot.c2fc` circuit for a XIAO
ESP32-S3 that drives a DC motor through a DRV8833: look up the parts'
pins, check the circuit until it has no errors, and show me the
wiring." Then open `robot.c2fc` in the app and wire it in Assembly
mode.

`mass_properties` measures, rather than guesses, what a mechanical
review needs. It takes a design (`code` or `path`, with `parameters`)
or a circuit, and prints each `part()` (`root` for what is outside any)
in `material` (PLA by default, PETG, ABS, ASA, TPU, nylon, or a density
in g/cm³) with `infill` percent (20) and walls `wall` millimeters thick
(0.8); `parts` sets them per part, as `{"lid": {"material": "PETG"}}`.
A part's mass estimates a slicer's: its surface times the wall, plus the
infill's share of the rest; its center is the solid's centroid. A
circuit adds the enclosure's parts and each part it places, at its
bounding box's center, by the mass its sheet gives
(`// @spec mass: 3.3 g`; without one, `mass_g` is null and a warning
says so). `extra` adds what the model carries without drawing it, as
`[{"name": "watch", "mass_g": 150, "at": [0, 0, 60]}]`. It answers with
`parts` (`name`, `volume_mm3`, `mass_g`, `material`, `infill`, `wall`,
`center`), `components` for a circuit, `extra`, the `total` mass and
`center` of gravity, and `stability`: the `base` it stands on (the
convex hull of the points within 0.2 mm of its lowest), whether the
center of gravity is `inside` it, and `min_tipping_deg`, how far it
leans before it tips over, toward `direction`. `c2f mass` prints the
same as a table.

### Create a part from its product page

When the library lacks a part, an agent that can read the web writes it
from the supplier's product page and datasheet: the board drawn as a
module of the file's name, its pins, mount holes and ports, and its
sheet (section 1). Code2Form does not fetch the pages itself; its tools
check what the agent wrote and file it.

- `check_part` (`code`, or a `path`) reads the annotations, renders the
  part with its default parameters and compares the two. Errors, with
  their line, make `ok` false: a badly written annotation, a part that
  does not render, a pin or a mount hole outside the part's bounding box
  (by more than 0.5 mm), two pins at the same place, a port whose
  direction points into the part rather than out of it, a power pin
  without its voltage. A missing `@description` or `@source` is a
  warning. The report also gives the bounding box and how many pins,
  holes, ports, specs and sources the part has.
- `save_part` (`name`, `code`, `overwrite`) writes `NAME.c2f` into your
  parts folder, `~/Documents/Code2Form/libraries/electronics`, the one
  the catalogue's **New part...** uses: the catalogue lists it, and a
  circuit names it `electronics/NAME.c2f`. It refuses a file that does
  not parse, in which `check_part` finds errors, or without a `@description`
  and at least one `@source`, and it does not replace a part unless
  `overwrite` is true. It returns the path and the `check_part` report.

A typical request: "Find the Adafruit BME280 breakout's product page and
datasheet, write it as a Code2Form part with its pins, mount holes and
sheet, check it until it has no errors, and save it as
`bme280_breakout`." Then **⟳** in the catalogue lists it.

### Loads on a shaft

`loads` (and `c2f loads scenario.json` on the command line, the same
JSON) weighs a design's moving parts with numbers: static engineering
estimates, with margins for what they leave out, not a simulation. The
scenario gives the shaft's `axis` (`at` and `direction`; gravity is
along -z, the axis may be tilted), the `loads` (`name`, `mass_g`, `at`
their center of mass), the `supports` along the axis (`at_mm`, `kind`
`motor`, `bearing` or `bushing` for a printed hole, a `part` whose sheet
gives its figures, `coupling` true for a motor on a flexible coupling,
which holds nothing up), the `motor` (its `part`, or `torque_mNm`,
`stall_torque_mNm`, `speed_rpm`, `max_radial_load_N`,
`max_axial_load_N`), the `shaft` (`d_mm` and `material`, or `E_GPa`),
the `ratio` (1) and the `target_margin` (2). A motor's or a bearing's
figures come from its sheet when the scenario leaves them out:
`// @spec torque: 34 mN·m`, `stall torque`, `holding torque`, `speed`,
`max radial load`, `max axial load`, `static load`.

It answers the torque the shaft needs (the loads' unbalance in the
worst position of a turn, plus friction: 0.0015 in a ball bearing, 0.3
in a printed hole or a gear motor's sleeve), each support's radial and
axial reaction and moment, the shaft's deflection at each load, the
margins, a `suggested_ratio` when the torque falls short, `warnings`
and a `summary` to quote. One support holds the loads and their
overhang's moment, which loads a motor's front bearing with the moment
over its bearings' span (8 mm); two are solved exactly; more share the
load as on springs of one stiffness.

A watch winder: a 150 g watch whose center sits 20 mm off a horizontal
5 mm steel shaft, 35 mm in front of a gear motor's face (34 mN·m, 10 N
of radial load). On the motor alone, the shaft needs 40.2 mN·m (29.4
for the watch, 10.8 of friction in the motor's sleeve): a torque margin
of 0.85, and a 3:1 reduction suggested; the watch's 1.47 N hangs 35 mm
past the motor, 51.5 N·mm that put 7.91 N on its front bearing, a radial
margin of 1.26 and an overhung-load warning. A ball bearing 5 mm behind
the watch takes 1.72 N and leaves the motor 0.25 N: friction falls to
0.19 mN·m, the torque needed to 29.6 mN·m, and a 2:1 reduction reaches
the margin of 2. With `coupling` true, the motor takes no radial load
at all, and the bearing alone holds the overhang's 7.4 N·mm.

### From an idea to a build

The server offers three prompts, which Claude Code lists as
`/mcp__code2form__plan_object`, `/mcp__code2form__design_object` and
`/mcp__code2form__add_electronics` (Claude Desktop under the + button).
They keep the work in order, each step ending on what you approve:

1. **`plan_object`** (an `idea`, maybe a `folder`): the agent draws
   nothing. It asks what the object must do, in short rounds that follow
   your answers, reviews its mechanics (masses, balance, forces and
   supports, torque, material, fastening, assembly, energy) with
   `mass_properties` and `loads`, and writes a brief with `save_brief`:
   its decisions, and requirements a tool measures. See
   [the watch winder](plan-mode.md).
2. **`design_object`** (a `brief` or a `description`, maybe
   `constraints` and a `folder`): the agent writes a parametric design,
   renders it, looks at it from every side, checks it against the brief
   with `check_brief`, and shows it to you until you approve it. No
   electronics yet.
3. **`add_electronics`** (the approved `design` and its `components`,
   as product page links or library names): the agent finds or creates
   each part, tells you its wiring plan, then calls `electrify` on the
   design, places the parts and wires them, checks the circuit, gives
   the design what the parts need, and shows you the build.

`electrify` makes an approved design ready for its electronics without
rewriting it. It creates the circuit file beside the design, with the
design as its `enclosure`, names the circuit in the design
(`// @circuit`), and moves what the design draws, unchanged, into a
module `design()`. Its assignments, modules and functions stay where
they are, so it draws the same: the tool reports the volume before and
after. Two empty modules take what the electronics need, and the design
draws all three once:

```scad
difference() {
    union() {
        design();
        electronics_mounts();   // standoffs_for(...)
    }
    electronics_cuts();         // cutout_for(...), holes_for(...)
}
```

A design made of pieces (`part("Base")`, `part("Dome")`) keeps them
apart: a piece inside that `difference()` would be merged into the
geometry around it. So `electrify` leaves the drawing where it is and
gives the cuts and mounts to the piece that holds the electronics, named
with `piece` when there are several, wherever the design draws it:

```scad
part("Base") difference() { union() { base(); electronics_mounts(); } electronics_cuts(); }
```

They are written in the design's coordinates, which are the piece's
where the design draws it assembled.

In the app, a design that names its circuit has an Electronics button
on the top bar, and a circuit with an enclosure a Part button, to go
from one to the other.

[From an idea to a night light](idea-to-build.md) follows the two
prompts on a real example, step by step, with the pictures of each.
