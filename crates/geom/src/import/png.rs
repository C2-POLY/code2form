//! A PNG decoder for `surface()`: every color type and bit depth, Adam7
//! interlacing, to 8-bit RGB as lodepng gives it to the reference
//! implementation (16-bit samples keep their high byte, low depths are
//! scaled to 0..255).

pub(super) const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

pub(super) struct Image {
    pub width: usize,
    pub height: usize,
    /// Row by row from the top, three bytes per pixel.
    pub rgb: Vec<u8>,
}

pub(super) fn decode(data: &[u8]) -> Result<Image, String> {
    let mut rest = data.strip_prefix(SIGNATURE).ok_or("not a PNG file")?;
    let mut header = None;
    let mut palette: &[u8] = &[];
    let mut compressed = Vec::new();
    while rest.len() >= 12 {
        let length = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
        let kind = &rest[4..8];
        let body = rest.get(8..8 + length).ok_or("truncated chunk")?;
        match kind {
            b"IHDR" if body.len() >= 13 => header = Some(body),
            b"PLTE" => palette = body,
            b"IDAT" => compressed.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        rest = rest.get(12 + length..).ok_or("truncated chunk")?;
    }
    let header = header.ok_or("no IHDR chunk")?;
    let width = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
    let height = u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
    let (depth, color, interlaced) = (header[8] as usize, header[9], header[12] == 1);
    let channels = match (color, depth) {
        (0, 1 | 2 | 4 | 8 | 16) => 1,
        (2, 8 | 16) => 3,
        (3, 1 | 2 | 4 | 8) => 1,
        (4, 8 | 16) => 2,
        (6, 8 | 16) => 4,
        _ => return Err(format!("unsupported color type {color} with bit depth {depth}")),
    };
    if width == 0 || height == 0 || width.saturating_mul(height) > 1 << 28 {
        return Err("bad image size".into());
    }
    let bits = channels * depth;
    // At most a filter byte and a row of samples per row, plus Adam7's
    // partial rows: a bound against decompression bombs.
    let limit = height * (2 + (width * bits).div_ceil(8)) * 2 + 1024;
    let raw = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&compressed, limit)
        .map_err(|e| format!("bad image data: {e:?}"))?;
    let mut rgb = vec![0u8; width * height * 3];
    let mut consumed = 0;
    // Adam7 passes: start and step in x and y; one pass when not
    // interlaced.
    let passes: &[(usize, usize, usize, usize)] = if interlaced {
        &[(0, 8, 0, 8), (4, 8, 0, 8), (0, 4, 4, 8), (2, 4, 0, 4), (0, 2, 2, 4), (1, 2, 0, 2), (0, 1, 1, 2)]
    } else {
        &[(0, 1, 0, 1)]
    };
    for &(x0, dx, y0, dy) in passes {
        let (w, h) = ((width.saturating_sub(x0)).div_ceil(dx), (height.saturating_sub(y0)).div_ceil(dy));
        if w == 0 || h == 0 {
            continue;
        }
        let stride = (w * bits).div_ceil(8);
        let bpp = bits.div_ceil(8);
        let mut previous = vec![0u8; stride];
        for row in 0..h {
            let filter = *raw.get(consumed).ok_or("truncated image data")?;
            let mut line = raw.get(consumed + 1..consumed + 1 + stride).ok_or("truncated image data")?.to_vec();
            consumed += 1 + stride;
            unfilter(filter, &mut line, &previous, bpp)?;
            for col in 0..w {
                let sample = |c: usize| sample(&line, (col * channels + c) * depth, depth);
                let px = match color {
                    3 => {
                        let i = sample(0) as usize * 3;
                        let entry = palette.get(i..i + 3).ok_or("palette index out of range")?;
                        [entry[0], entry[1], entry[2]]
                    }
                    0 | 4 => [scale(sample(0), depth); 3],
                    _ => [scale(sample(0), depth), scale(sample(1), depth), scale(sample(2), depth)],
                };
                let at = ((y0 + row * dy) * width + x0 + col * dx) * 3;
                rgb[at..at + 3].copy_from_slice(&px);
            }
            previous = line;
        }
    }
    Ok(Image { width, height, rgb })
}

/// The `depth`-bit sample starting at bit `bit` of `line`.
fn sample(line: &[u8], bit: usize, depth: usize) -> u16 {
    match depth {
        16 => u16::from_be_bytes([line[bit / 8], line[bit / 8 + 1]]),
        8 => line[bit / 8] as u16,
        _ => ((line[bit / 8] >> (8 - depth - bit % 8)) & ((1 << depth) - 1) as u8) as u16,
    }
}

/// To 8 bits: the high byte of 16-bit samples, low depths stretched.
fn scale(value: u16, depth: usize) -> u8 {
    match depth {
        16 => (value >> 8) as u8,
        8 => value as u8,
        _ => (value as u32 * 255 / ((1 << depth) - 1)) as u8,
    }
}

fn unfilter(filter: u8, line: &mut [u8], previous: &[u8], bpp: usize) -> Result<(), String> {
    for i in 0..line.len() {
        let a = if i >= bpp { line[i - bpp] } else { 0 };
        let b = previous[i];
        let c = if i >= bpp { previous[i - bpp] } else { 0 };
        let predictor = match filter {
            0 => 0,
            1 => a,
            2 => b,
            3 => ((a as u16 + b as u16) / 2) as u8,
            4 => {
                let p = a as i16 + b as i16 - c as i16;
                let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
                if pa <= pb && pa <= pc {
                    a
                } else if pb <= pc {
                    b
                } else {
                    c
                }
            }
            _ => return Err(format!("bad filter type {filter}")),
        };
        line[i] = line[i].wrapping_add(predictor);
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// A PNG of `rows` (already serialized samples), unfiltered.
    pub fn encode(width: u32, height: u32, depth: u8, color: u8, rows: &[Vec<u8>], palette: Option<&[u8]>) -> Vec<u8> {
        fn chunk(out: &mut Vec<u8>, kind: &[u8], body: &[u8]) {
            out.extend((body.len() as u32).to_be_bytes());
            out.extend(kind);
            out.extend(body);
            out.extend([0u8; 4]); // CRC, not checked
        }
        let mut raw = Vec::new();
        for row in rows {
            raw.push(0);
            raw.extend(row);
        }
        let mut out = SIGNATURE.to_vec();
        let mut ihdr = width.to_be_bytes().to_vec();
        ihdr.extend(height.to_be_bytes());
        ihdr.extend([depth, color, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &ihdr);
        if let Some(palette) = palette {
            chunk(&mut out, b"PLTE", palette);
        }
        chunk(&mut out, b"IDAT", &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6));
        chunk(&mut out, b"IEND", &[]);
        out
    }

    #[test]
    fn gray_rgb_palette_and_low_depths() {
        let gray = decode(&encode(3, 1, 8, 0, &[vec![0, 128, 255]], None)).unwrap();
        assert_eq!(gray.rgb, [0, 0, 0, 128, 128, 128, 255, 255, 255]);
        let rgb16 = decode(&encode(1, 1, 16, 2, &[vec![0x12, 0x34, 0xab, 0xcd, 0xff, 0x00]], None)).unwrap();
        assert_eq!(rgb16.rgb, [0x12, 0xab, 0xff]);
        let palette = decode(&encode(2, 1, 8, 3, &[vec![1, 0]], Some(&[200, 100, 50, 7, 8, 9]))).unwrap();
        assert_eq!(palette.rgb, [7, 8, 9, 200, 100, 50]);
        // 4-bit samples 1, 15, 8 scale by 17, as lodepng does.
        let low = decode(&encode(3, 1, 4, 0, &[vec![0x1f, 0x80]], None)).unwrap();
        assert_eq!(low.rgb, [17, 17, 17, 255, 255, 255, 136, 136, 136]);
    }

    #[test]
    fn filters() {
        let mut line = vec![10, 20, 30, 40];
        unfilter(1, &mut line, &[0; 4], 1).unwrap();
        assert_eq!(line, [10, 30, 60, 100]);
        let mut line = vec![1, 1, 1, 1];
        unfilter(4, &mut line, &[5, 6, 7, 8], 1).unwrap();
        assert_eq!(line, [6, 7, 8, 9]);
    }
}
