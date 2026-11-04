// Written by Code2Form from grogu.c2fc: a first firmware that sets each part
// up, then tries them in turn, to check the wiring. It is yours to change:
// generating the firmware again rewrites include/pins.h only.

#include <Arduino.h>
#include <driver/i2s.h>
#include "pins.h"

void setup() {
  Serial.begin(115200);
  delay(500);
  Serial.println("grogu: setting up");
  // amp
  i2s_config_t amp_i2s = { .mode = (i2s_mode_t)(I2S_MODE_MASTER | I2S_MODE_TX), .sample_rate = 16000, .bits_per_sample = I2S_BITS_PER_SAMPLE_16BIT, .channel_format = I2S_CHANNEL_FMT_ONLY_LEFT, .communication_format = I2S_COMM_FORMAT_STAND_I2S, .intr_alloc_flags = 0, .dma_buf_count = 4, .dma_buf_len = 256 };
  i2s_pin_config_t amp_pins = { .mck_io_num = I2S_PIN_NO_CHANGE, .bck_io_num = AMP_BCLK, .ws_io_num = AMP_LRC, .data_out_num = AMP_DIN, .data_in_num = I2S_PIN_NO_CHANGE };
  i2s_driver_install(I2S_NUM_0, &amp_i2s, 0, NULL); i2s_set_pin(I2S_NUM_0, &amp_pins);
  // left out, `amp.SD` is not wired to a GPIO of the board: pinMode({pin:SD}, OUTPUT); digitalWrite({pin:SD}, HIGH);
  // mic
  i2s_config_t mic_i2s = { .mode = (i2s_mode_t)(I2S_MODE_MASTER | I2S_MODE_RX), .sample_rate = 16000, .bits_per_sample = I2S_BITS_PER_SAMPLE_32BIT, .channel_format = I2S_CHANNEL_FMT_ONLY_LEFT, .communication_format = I2S_COMM_FORMAT_STAND_I2S, .intr_alloc_flags = 0, .dma_buf_count = 4, .dma_buf_len = 256 };
  i2s_pin_config_t mic_pins = { .mck_io_num = I2S_PIN_NO_CHANGE, .bck_io_num = MIC_SCK, .ws_io_num = MIC_WS, .data_out_num = I2S_PIN_NO_CHANGE, .data_in_num = MIC_SD };
  i2s_driver_install(I2S_NUM_1, &mic_i2s, 0, NULL); i2s_set_pin(I2S_NUM_1, &mic_pins);
  // driver
  // left out, `driver.EEP` is not wired to a GPIO of the board: pinMode({pin:EEP}, OUTPUT); digitalWrite({pin:EEP}, HIGH);
  pinMode(DRIVER_IN1, OUTPUT); digitalWrite(DRIVER_IN1, LOW);
  pinMode(DRIVER_IN2, OUTPUT); digitalWrite(DRIVER_IN2, LOW);
  // left out, `driver.IN3` is not wired to a GPIO of the board: pinMode({pin:IN3}, OUTPUT); digitalWrite({pin:IN3}, LOW);
  // left out, `driver.IN4` is not wired to a GPIO of the board: pinMode({pin:IN4}, OUTPUT); digitalWrite({pin:IN4}, LOW);
}

void loop() {
  // amp
  Serial.println("amp");
  { static int16_t tone[256]; for (int i = 0; i < 256; i++) tone[i] = (i / 16) % 2 ? 3000 : -3000; size_t written; for (int k = 0; k < 30; k++) i2s_write(I2S_NUM_0, tone, sizeof(tone), &written, portMAX_DELAY); }
  delay(500);
  // mic
  Serial.println("mic");
  { static int32_t samples[256]; size_t got = 0; i2s_read(I2S_NUM_1, samples, sizeof(samples), &got, 100); long peak = 0; for (size_t i = 0; i < got / 4; i++) peak = max(peak, labs((long)(samples[i] >> 14))); Serial.printf("mic: level %ld\n", peak); }
  delay(500);
  // driver
  Serial.println("driver");
  digitalWrite(DRIVER_IN1, HIGH); delay(500); digitalWrite(DRIVER_IN1, LOW); digitalWrite(DRIVER_IN2, LOW);
  // left out, `driver.IN3` is not wired to a GPIO of the board: digitalWrite({pin:IN3}, HIGH); delay(500); digitalWrite({pin:IN3}, LOW); digitalWrite({pin:IN4}, LOW);
  delay(500);
  // touch
  Serial.println("touch");
  Serial.printf("touch: touch %lu\n", (unsigned long)touchRead(TOUCH_PAD));
  delay(500);
  delay(1000);
}
