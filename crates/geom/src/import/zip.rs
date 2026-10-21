//! Reading members of a ZIP archive (3MF packages, compressed AMF): the
//! central directory, stored and deflated entries.

/// The archive's entries: name and uncompressed content.
pub(super) fn entries(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let u16_at = |i: usize| data.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    let u32_at = |i: usize| data.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    // The end of central directory record, searched from the end (it may
    // be followed by a comment).
    let eocd = (0..data.len().saturating_sub(21))
        .rev()
        .find(|&i| data[i..].starts_with(b"PK\x05\x06"))
        .ok_or("not a ZIP archive")?;
    let count = u16_at(eocd + 10).ok_or("truncated ZIP")?;
    let mut at = u32_at(eocd + 16).ok_or("truncated ZIP")?;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if !data.get(at..).is_some_and(|d| d.starts_with(b"PK\x01\x02")) {
            return Err("bad ZIP central directory".into());
        }
        let field = |offset: usize, width: usize| if width == 2 { u16_at(at + offset) } else { u32_at(at + offset) };
        let (Some(method), Some(compressed), Some(size), Some(name_len), Some(extra_len), Some(comment_len), Some(local)) =
            (field(10, 2), field(20, 4), field(24, 4), field(28, 2), field(30, 2), field(32, 2), field(42, 4))
        else {
            return Err("truncated ZIP".into());
        };
        let name = data.get(at + 46..at + 46 + name_len).ok_or("truncated ZIP")?;
        let name = String::from_utf8_lossy(name).into_owned();
        at += 46 + name_len + extra_len + comment_len;
        // The data follows the local header, whose name and extra field
        // lengths may differ from the central ones.
        if !data.get(local..).is_some_and(|d| d.starts_with(b"PK\x03\x04")) {
            return Err(format!("bad ZIP entry '{name}'"));
        }
        let (Some(local_name), Some(local_extra)) = (u16_at(local + 26), u16_at(local + 28)) else {
            return Err("truncated ZIP".into());
        };
        let start = local + 30 + local_name + local_extra;
        let raw = data.get(start..start + compressed).ok_or_else(|| format!("truncated ZIP entry '{name}'"))?;
        let content = match method {
            0 => raw.to_vec(),
            // No more than the size the directory announces.
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(raw, size)
                .map_err(|e| format!("can't inflate '{name}': {e:?}"))?,
            _ => return Err(format!("unsupported compression method {method} for '{name}'")),
        };
        out.push((name, content));
    }
    Ok(out)
}

#[cfg(test)]
pub(super) mod tests {
    /// A ZIP archive of stored entries.
    pub fn stored(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, content) in files {
            let offset = out.len() as u32;
            let header = |sig: &[u8], central: bool| {
                let mut h = sig.to_vec();
                if central {
                    h.extend(20u16.to_le_bytes());
                }
                h.extend(20u16.to_le_bytes()); // version needed
                h.extend(0u16.to_le_bytes()); // flags
                h.extend(0u16.to_le_bytes()); // method: stored
                h.extend(0u32.to_le_bytes()); // time, date
                h.extend(0u32.to_le_bytes()); // crc (unchecked)
                h.extend((content.len() as u32).to_le_bytes());
                h.extend((content.len() as u32).to_le_bytes());
                h.extend((name.len() as u16).to_le_bytes());
                h.extend(0u16.to_le_bytes()); // extra
                if central {
                    h.extend([0u8; 6]); // comment length, disk, internal attributes
                    h.extend(0u32.to_le_bytes()); // external attributes
                    h.extend(offset.to_le_bytes());
                }
                h.extend(name.as_bytes());
                h
            };
            out.extend(header(b"PK\x03\x04", false));
            out.extend(*content);
            central.extend(header(b"PK\x01\x02", true));
        }
        let start = out.len() as u32;
        let size = central.len() as u32;
        out.extend(central);
        out.extend(b"PK\x05\x06");
        out.extend([0u8; 4]);
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((files.len() as u16).to_le_bytes());
        out.extend(size.to_le_bytes());
        out.extend(start.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out
    }

    #[test]
    fn stored_entries() {
        let zip = stored(&[("a.txt", b"hello"), ("dir/b", b"")]);
        let entries = super::entries(&zip).unwrap();
        assert_eq!(entries, vec![("a.txt".to_string(), b"hello".to_vec()), ("dir/b".to_string(), Vec::new())]);
    }
}
