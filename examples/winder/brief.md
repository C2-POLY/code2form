# Watch winder

A winder for an automatic watch (a Tissot PRX 40 mm on its steel
bracelet), on a bedside table: it turns the watch slowly, on and off
through the day, so that it keeps running between wears.

## What was asked, and answered

- **Use**: one watch, on a bedside table, turning a few hours a day;
  quiet enough for a bedroom. Default accepted: about 650 turns a day,
  both ways.
- **Size**: the watch on its bracelet, closed on a cushion; the whole
  winder within 20 cm a side.
- **Pieces**: a drum that holds the cushion and turns, a frame it turns
  in; the watch slides on and off the cushion from the front.
- **Making**: PLA on a 220 x 220 x 250 mm printer, no supports; the
  cushion in TPU.
- **Electronics**: a XIAO ESP32-S3 on USB-C power, a 28BYJ-48 5 V
  stepper on its ULN2003 board (bought as a kit).
- **Look**: a drum tilted towards the viewer, the watch's face showing.

A motor opened its questions: what it drives (the watch and its drum,
about 300 g with the cushion), where that weight sits (out front, 45 mm
along the axis, the case off the axis), how fast (a few turns a minute),
how quiet (a stepper, slow), continuous or not (programs of turns).

## Mechanical review

1. **Masses.** Watch 150 g (to weigh: the PRX on steel is given as
   135 to 150 g). Drum and cushion about 160 g printed. The frame about
   150 g. `mass_properties` on the design, with its boards and the watch:
   477 g.
2. **Balance.** The drum leans 30° towards the front: the center of
   gravity moves forward, over the base. `mass_properties` gives a
   tipping angle of 37°, enough for a table; the base's front is kept
   long for it.
3. **Forces and supports.** On the motor's shaft alone, the drum and the
   watch (about 310 g) hang 45 mm in front of it: `loads` gives 119 N·mm
   of bending moment on the motor and 17.5 N on its front bushing, for a
   gearbox made to turn a shaft, not to carry it. **Decision**: the drum
   turns on its own bearings, a 608 pivot in the back plate and two 608
   rollers under the front rim, and the motor only drives it, through a
   gear: then each 608 takes 1 to 1.6 N, under a thousandth of its static
   rating, and the motor carries nothing.
4. **Torque and speed.** The watch's case, off the axis, needs 12.8
   mN·m in the worst position of a turn. On its own shaft, with the
   friction of its bushing, the motor would need 38.2 mN·m for its 29.4:
   a margin of 0.77. **Decision**: a 14 to 64 tooth gear pair (module
   1.5), 4.57:1: 121 mN·m at the drum, a margin of 9.4, and 3.3 turns a
   minute from the motor's 15.
5. **Material and printing.** The drum, the plate and the pinion print
   flat without supports; the pivot's 608 is pressed into the plate from
   the front, on a shoulder that bears on its outer race; the drum's
   collar bears on the inner race only. The cushion in TPU, for the
   bracelet to slide on snugly.
6. **Fastening.** The plate to the base's cheeks with four M3 screws; the
   pivot an M8 bolt whose nut sits in the drum's hub; the rollers on M8
   bolts into the posts.
7. **Assembly.** The boards on the floor behind and under the plate, the
   motor in its pocket at the back of the plate, its wires led around the
   plate's underside.
8. **Energy.** The 28BYJ-48 draws about 240 mA at 5 V when it turns, the
   XIAO about 100 mA: under 0.5 A, within any USB port.

Left to check: the watch's mass on a scale, and a test print of the
pivot's seat (a press fit, 0.1 mm over the bearing).

## Requirements

- mass < 600 g
- tipping > 25°
- torque margin >= 2
- size <= 220 x 220 x 250 mm
- part "drum"
- part "pinion"
- component "electronics/xiao_esp32s3.c2f"
- component "parts/uln2003.c2f"
- component "parts/stepper_28byj48.c2f"
- prints without supports
- quiet enough for a bedroom

## What the tools compute from

```json mass
{
  "material": "PLA",
  "infill": 20,
  "extra": [{ "name": "watch", "mass_g": 150, "at": [0, -39, 100.5] }]
}
```

The drum's shaft, tilted 30° up towards the front, from where it meets
the back plate; the watch's case 15 mm off it in the worst case; the drum
on its pivot and its rollers; the stepper through the 14 to 64 gears.

```json loads
{
  "axis": { "at": [0, 0, 78], "direction": [0, -0.866, 0.5] },
  "loads": [
    { "name": "drum and cushion", "mass_g": 160, "at": [0, -39, 100.5] },
    { "name": "watch case, off the axis", "mass_g": 100, "at": [0, -39, 118] },
    { "name": "bracelet", "mass_g": 50, "at": [0, -39, 100.5] }
  ],
  "supports": [
    { "name": "pivot 608", "at_mm": -3.5, "kind": "bearing", "static_load_N": 1370 },
    { "name": "rollers (2 x 608)", "at_mm": 76, "kind": "bearing", "static_load_N": 2740 }
  ],
  "motor": { "torque_mNm": 29.4, "holding_torque_mNm": 34.3, "speed_rpm": 15 },
  "shaft": { "d_mm": 8, "material": "steel" },
  "ratio": 4.571
}
```
