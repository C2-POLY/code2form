// Written by Code2Form from winder.c2fc: a first firmware that sets each part
// up, then tries them in turn, to check the wiring. It is yours to change:
// generating the firmware again rewrites include/pins.h only.

#include <Arduino.h>
#include "pins.h"

// driver
const uint8_t driver_half_steps[8] = { 0b0001, 0b0011, 0b0010, 0b0110, 0b0100, 0b1100, 0b1000, 0b1001 };

void setup() {
  Serial.begin(115200);
  delay(500);
  Serial.println("winder: setting up");
  // driver
  pinMode(DRIVER_IN1, OUTPUT); pinMode(DRIVER_IN2, OUTPUT); pinMode(DRIVER_IN3, OUTPUT); pinMode(DRIVER_IN4, OUTPUT);
}

void loop() {
  // driver
  Serial.println("driver");
  for (int s = 0; s < 512; s++) { uint8_t coils = driver_half_steps[s % 8]; digitalWrite(DRIVER_IN1, (coils & 1) ? HIGH : LOW); digitalWrite(DRIVER_IN2, (coils & 2) ? HIGH : LOW); digitalWrite(DRIVER_IN3, (coils & 4) ? HIGH : LOW); digitalWrite(DRIVER_IN4, (coils & 8) ? HIGH : LOW); delayMicroseconds(1200); }
  digitalWrite(DRIVER_IN1, LOW); digitalWrite(DRIVER_IN2, LOW); digitalWrite(DRIVER_IN3, LOW); digitalWrite(DRIVER_IN4, LOW);
  delay(500);
  delay(1000);
}
