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
| `dc_motor.c2f` | Size 130 DC motor |
| `touch_plate.c2f` | Copper plate for a capacitive touch pad |

Each file starts with a link to the manufacturer's documentation and says
which dimensions come from it and which are estimated: check those
against your part before printing a tight fit. Pins carry their
silkscreen name, a label with their function, their type and the most
they take in volts; the ESP32-S3's strapping pins (GPIO0, GPIO3, GPIO45,
GPIO46) are marked `reserved`. Annotations are plain numbers: they stay
where the file's default parameters put them.

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
