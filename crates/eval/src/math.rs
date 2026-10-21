//! Degree-based trigonometry that is exact at multiples of 30° and 45°,
//! so `rotate(90)` yields 0 and 1 rather than 6.1e-17, and the random
//! generator behind `rands()`.

use std::f64::consts::FRAC_1_SQRT_2;

const SQRT3_2: f64 = 0.866_025_403_784_438_6;

fn rad(deg: f64) -> f64 {
    deg.to_radians()
}

/// Reduces to [0, 360).
fn wrap(x: f64) -> f64 {
    let x = x % 360.0;
    if x < 0.0 { x + 360.0 } else { x }
}

pub fn sin_degrees(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let mut x = wrap(x);
    let oppose = x >= 180.0;
    if oppose {
        x -= 180.0;
    }
    if x > 90.0 {
        x = 180.0 - x;
    }
    let r = if x < 45.0 {
        if x == 30.0 { 0.5 } else { rad(x).sin() }
    } else if x == 45.0 {
        FRAC_1_SQRT_2
    } else if x == 60.0 {
        SQRT3_2
    } else {
        rad(90.0 - x).cos()
    };
    if oppose { -r } else { r }
}

pub fn cos_degrees(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let mut x = wrap(x);
    let mut oppose = x >= 180.0;
    if oppose {
        x -= 180.0;
    }
    if x > 90.0 {
        x = 180.0 - x;
        oppose = !oppose;
    }
    let r = if x > 45.0 {
        if x == 60.0 { 0.5 } else { rad(90.0 - x).sin() }
    } else if x == 45.0 {
        FRAC_1_SQRT_2
    } else if x == 30.0 {
        SQRT3_2
    } else {
        rad(x).cos()
    };
    if oppose { -r } else { r }
}

pub fn tan_degrees(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    // Odd multiples of 90 keep their sign: tan(90) is inf, tan(270) and
    // tan(-90) are -inf.
    let negative = x < 0.0;
    let mut x = x.abs() % 360.0;
    if x == 90.0 || x == 270.0 {
        let up = (x == 90.0) != negative;
        return if up { f64::INFINITY } else { f64::NEG_INFINITY };
    }
    let mut oppose = negative;
    if x >= 180.0 {
        x -= 180.0;
    }
    if x > 90.0 {
        x = 180.0 - x;
        oppose = !oppose;
    }
    let r = if x == 0.0 {
        0.0
    } else if x == 45.0 {
        1.0
    } else if x == 90.0 {
        f64::INFINITY
    } else {
        rad(x).tan()
    };
    if oppose { -r } else { r }
}

/// The 32-bit Mersenne Twister (`std::mt19937`).
pub struct Mt19937 {
    state: [u32; 624],
    index: usize,
}

impl Mt19937 {
    pub fn new(seed: u32) -> Self {
        let mut state = [0u32; 624];
        state[0] = seed;
        for i in 1..624 {
            state[i] = 1_812_433_253u32.wrapping_mul(state[i - 1] ^ (state[i - 1] >> 30)).wrapping_add(i as u32);
        }
        Mt19937 { state, index: 624 }
    }

    pub fn next_u32(&mut self) -> u32 {
        if self.index >= 624 {
            for i in 0..624 {
                let y = (self.state[i] & 0x8000_0000) | (self.state[(i + 1) % 624] & 0x7fff_ffff);
                let mut next = self.state[(i + 397) % 624] ^ (y >> 1);
                if y & 1 != 0 {
                    next ^= 0x9908_b0df;
                }
                self.state[i] = next;
            }
            self.index = 0;
        }
        let mut y = self.state[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^ (y >> 18)
    }

    /// `std::uniform_real_distribution<double>(min, max)` as implemented by
    /// libc++ (macOS builds): `generate_canonical` draws two 32-bit words.
    pub fn uniform(&mut self, min: f64, max: f64) -> f64 {
        let r = 4_294_967_296.0;
        let low = self.next_u32() as f64;
        let high = self.next_u32() as f64;
        let canonical = (low + high * r) / (r * r);
        (max - min) * canonical + min
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_angles() {
        assert_eq!(sin_degrees(30.0), 0.5);
        assert_eq!(cos_degrees(90.0), 0.0);
        assert_eq!(cos_degrees(60.0), 0.5);
        assert_eq!(sin_degrees(450.0), 1.0);
        assert_eq!(sin_degrees(-90.0), -1.0);
        assert_eq!(tan_degrees(45.0), 1.0);
        assert_eq!(tan_degrees(90.0), f64::INFINITY);
        assert_eq!(tan_degrees(270.0), f64::NEG_INFINITY);
        assert_eq!(tan_degrees(-90.0), f64::NEG_INFINITY);
        assert_eq!(tan_degrees(-270.0), f64::INFINITY);
        assert_eq!(tan_degrees(135.0), -1.0);
    }

    #[test]
    fn matches_std_mt19937() {
        // The 10000th output of a default-seeded std::mt19937 is 4123659995.
        let mut mt = Mt19937::new(5489);
        let mut last = 0;
        for _ in 0..10000 {
            last = mt.next_u32();
        }
        assert_eq!(last, 4_123_659_995);
    }

    #[test]
    fn matches_reference_rands() {
        // The reference implementation (release 2021.01) gives
        // rands(0, 1, 2, 42) = [0.796543, 0.183435].
        let mut mt = Mt19937::new(42);
        assert!((mt.uniform(0.0, 1.0) - 0.796543).abs() < 1e-6);
        assert!((mt.uniform(0.0, 1.0) - 0.183435).abs() < 1e-6);
    }
}
