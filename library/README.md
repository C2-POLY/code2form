# Code2Form library

Parts and helpers that Code2Form ships with: the app bundles this folder
in its `Resources/libraries`, and `cargo run` finds it in the repository,
so a design or a circuit names a part from here as it names MCAD's:

```
use <electronics/xiao_esp32s3.c2f>
xiao_esp32s3(headers = true);
```

or, in a `.c2fc` circuit, `part = "electronics/xiao_esp32s3.c2f"`.

## Electronics

`electronics/` holds annotated parts: each file draws the part, declares
its pins, mount holes and ports in `// @pin`, `// @mount_hole` and
`// @port` comments, in its own coordinates, and defines a module of the
file's name taking the same parameters as its customizer.

| File | Part |
|---|---|
| `xiao_esp32s3.c2f` | Seeed Studio XIAO ESP32-S3 |
| `esp32s3_devkitc1.c2f` | Espressif ESP32-S3-DevKitC-1 |
| `drv8833.c2f` | DRV8833 dual H-bridge motor driver module |
| `max98357a.c2f` | Adafruit MAX98357A I2S amplifier breakout |
| `inmp441.c2f` | INMP441 I2S MEMS microphone module |
| `capacitor_radial.c2f` | Radial electrolytic capacitor (diameter and height as parameters) |
| `capacitor_ceramic.c2f` | 100 nF through-hole ceramic capacitor |
| `speaker.c2f` | 28 mm 8 ohm speaker |
| `dc_motor.c2f` | Size 130 DC motor (Mabuchi FA-130RA) |
| `n20_gear_motor.c2f` | N20 micro metal gear motor, 6 V, 100:1 (ratio as a parameter) |
| `stepper_28byj48.c2f` | 28BYJ-48 5 V geared stepper, with its 5-pin plug for a ULN2003 board |
| `touch_plate.c2f` | Copper plate for a capacitive touch pad |
| `neopixel_ring_12.c2f` | Adafruit NeoPixel Ring 12 (12 RGB LEDs) |

Each file starts with a link to the manufacturer's documentation and says
which dimensions come from it and which are estimated: check those
against your part before printing a tight fit. Pins carry their
silkscreen name, a label with their function, their type and the most
they take in volts; the ESP32-S3's strapping pins (GPIO0, GPIO3, GPIO45,
GPIO46) are marked `reserved`. Annotations are plain numbers: they stay
where the file's default parameters put them. Every part's sheet gives
its mass (`// @spec mass: 17 g`), and the motors their torque and speed,
which the mechanical checks read as quantities; the opening comment says
which come from the maker and which are estimated.

## Mechanical

`mechanical/` holds the parts that carry and pass on a motion, drawn to
scale with their sheet: what they are, their key figures (size, mass,
load ratings) and the standard or the maker's page they come from. They
have no pins; a design `use`s them as the electronics:

```
use <mechanical/bearing_seat.c2f>
use <mechanical/gear_pair.c2f>
bearing_seat("608", fit = "press") cube([40, 40, 10]);
gear_pair(ratio = 3, mod = 1, thickness = 6);
```

| File | Part | Key figures |
|---|---|---|
| `bearing_608.c2f` | Ball bearing 608 | 8 x 22 x 7 mm, C 3.45 kN, C0 1.37 kN, 12.1 g (SKF) |
| `bearing_625.c2f` | Ball bearing 625 | 5 x 16 x 5 mm, C 1.14 kN, C0 0.38 kN, 4.7 g (SKF) |
| `bearing_6800.c2f` | Ball bearing 6800 | 10 x 19 x 5 mm, C 1.89 kN, C0 0.84 kN, 5 g (NSK) |
| `bearing_6001.c2f` | Ball bearing 6001 | 12 x 28 x 8 mm, C 5.4 kN, C0 2.36 kN, 21 g (SKF) |
| `shaft.c2f` | Plain round shaft | diameter, length and material (steel, stainless, aluminium, brass) as parameters |
| `socket_screw.c2f` | Socket head cap screw ISO 4762 | M2 to M5, the length as a parameter |
| `hex_nut.c2f` | Hexagon nut ISO 4032 | M2 to M5 |
| `heat_set_insert.c2f` | Brass heat-set insert | M2 to M5, with the hole to print (M3: 4 mm) |
| `rigid_coupling.c2f` | Rigid shaft coupling | bores 3 to 12 mm (5 to 8 mm by default), 20 x 25 mm |
| `spur_gear.c2f` | Involute spur gear | module, teeth, thickness, bore, pressure angle 20° |
| `bearing_seat.c2f` | Helper: a bearing's seat | `bearing_seat(bearing, fit, depth)`: press (+0.1 mm) or slide (+0.3 mm) fit for FDM printing |
| `gear_pair.c2f` | Helper: a gear reduction | `gear_pair(ratio, mod, teeth, thickness, center)`: two gears at m·(z1 + z2)/2, the actual ratio echoed |

`mod` is the gears' module: the language keeps the word `module` for
itself. The screws, nuts and inserts echo their mass for the parameters
set, as the shaft and the coupling do; `lib/` holds the
bearings' sizes and drawing, which the bearings and `bearing_seat` share.

## Shapes

`shapes/rounded.c2f` rounds or chamfers the edges of a cube or a
cylinder, keeping its bounding box and placement, as the properties panel
writes it:

```
use <shapes/rounded.c2f>
rounded_cube([30, 20, 10], fillet = 2, edges = "vertical");
chamfered_cylinder(h = 20, r = 10, chamfer = 1, rims = "top");
```

`edges` is `"all"` (the default), `"vertical"`, `"top"` or `"bottom"`;
`rims` is `"both"` (the default), `"top"` or `"bottom"`; `center` is
`cylinder()`'s and `cube()`'s.

## Licence

The files in this folder are licensed as the rest of the repository,
under either of the [Apache License 2.0](../LICENSE-APACHE) or the
[MIT licence](../LICENSE-MIT), at your option.
