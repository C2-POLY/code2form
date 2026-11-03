// Written by Code2Form from grogu.c2fc, and again each time the firmware is
// generated: the GPIO of the board (xiao) each part's pin is wired to.

#pragma once

// amp: electronics/max98357a.c2f
#define AMP_LRC  5   // xiao.D4, green wire
#define AMP_BCLK 4   // xiao.D3, blue wire
#define AMP_DIN  6   // xiao.D5, yellow wire

// mic: electronics/inmp441.c2f
#define MIC_SD  9   // xiao.D10, yellow wire
#define MIC_WS  8   // xiao.D9, green wire
#define MIC_SCK 7   // xiao.D8, blue wire

// driver: electronics/drv8833.c2f
#define DRIVER_IN2 3   // xiao.D2, purple wire
#define DRIVER_IN1 2   // xiao.D1, purple wire

// touch: electronics/touch_plate.c2f
#define TOUCH_PAD 1   // xiao.D0, white wire
