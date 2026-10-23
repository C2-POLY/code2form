//! Reading members of a ZIP archive (3MF packages, compressed AMF): the
//! central directory, stored and deflated entries, and the ZIP64 records
//! that some CAD programs write even for small archives.

/// The archive's entries: name and uncompressed content.
pub(crate) fn entries(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let u16_at = |i: usize| data.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    let u32_at = |i: usize| data.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    // The end of central directory record, searched from the end (it may
    // be followed by a comment).
    let eocd = (0..data.len().saturating_sub(21))
        .rev()
        .find(|&i| data[i..].starts_with(b"PK\x05\x06"))
        .ok_or("not a ZIP archive")?;
    let u64_at = |i: usize| data.get(i..i + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap_or([0; 8])) as usize);
    let mut count = u16_at(eocd + 10).ok_or("truncated ZIP")?;
    let mut at = u32_at(eocd + 16).ok_or("truncated ZIP")?;
    // ZIP64: a locator just before the record points to the 64-bit one,
    // which holds the real count and offset.
    let locator = eocd.checked_sub(20).filter(|&i| data[i..].starts_with(b"PK\x06\x07"));
    if let Some(locator) = locator {
        let record = u64_at(locator + 8).ok_or("truncated ZIP")?;
        if !data.get(record..).is_some_and(|d| d.starts_with(b"PK\x06\x06")) {
            return Err("bad ZIP64 record".into());
        }
        count = u64_at(record + 32).ok_or("truncated ZIP")?;
        at = u64_at(record + 48).ok_or("truncated ZIP")?;
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if !data.get(at..).is_some_and(|d| d.starts_with(b"PK\x01\x02")) {
            return Err("bad ZIP central directory".into());
        }
        let field = |offset: usize, width: usize| if width == 2 { u16_at(at + offset) } else { u32_at(at + offset) };
        let (Some(method), Some(mut compressed), Some(mut size), Some(name_len), Some(extra_len), Some(comment_len), Some(mut local)) =
            (field(10, 2), field(20, 4), field(24, 4), field(28, 2), field(30, 2), field(32, 2), field(42, 4))
        else {
            return Err("truncated ZIP".into());
        };
        let name = data.get(at + 46..at + 46 + name_len).ok_or("truncated ZIP")?;
        let name = String::from_utf8_lossy(name).into_owned();
        // The values too large for 32 bits are in the ZIP64 extra field, in
        // this order, those that are.
        let mut extra = at + 46 + name_len;
        let extra_end = extra + extra_len;
        while extra + 4 <= extra_end {
            let (Some(id), Some(len)) = (u16_at(extra), u16_at(extra + 2)) else { break };
            if id == 1 {
                let mut value = extra + 4;
                for slot in [&mut size, &mut compressed, &mut local] {
                    if *slot == 0xFFFF_FFFF {
                        *slot = u64_at(value).ok_or("truncated ZIP")?;
                        value += 8;
                    }
                }
            }
            extra += 4 + len;
        }
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
    #[test]
    fn a_zip64_archive_reads() {
        // `zip -fz`'s layout: the 64-bit record and its locator before the
        // end record, sizes in the extra field.
        let content = b"hello";
        let mut out = Vec::new();
        let mut local = b"PK\x03\x04".to_vec();
        local.extend([45, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        local.extend(0xFFFF_FFFFu32.to_le_bytes());
        local.extend(0xFFFF_FFFFu32.to_le_bytes());
        local.extend(1u16.to_le_bytes());
        local.extend(20u16.to_le_bytes());
        local.push(b'a');
        local.extend(1u16.to_le_bytes());
        local.extend(16u16.to_le_bytes());
        local.extend((content.len() as u64).to_le_bytes());
        local.extend((content.len() as u64).to_le_bytes());
        out.extend(&local);
        out.extend(content);
        let directory = out.len();
        out.extend(b"PK\x01\x02");
        out.extend([45, 0, 45, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend(0xFFFF_FFFFu32.to_le_bytes());
        out.extend(0xFFFF_FFFFu32.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(28u16.to_le_bytes());
        // Comment length, disk, internal and external attributes.
        out.extend([0u8; 10]);
        out.extend(0xFFFF_FFFFu32.to_le_bytes());
        out.push(b'a');
        out.extend(1u16.to_le_bytes());
        out.extend(24u16.to_le_bytes());
        out.extend((content.len() as u64).to_le_bytes());
        out.extend((content.len() as u64).to_le_bytes());
        out.extend(0u64.to_le_bytes());
        let directory_size = out.len() - directory;
        let record = out.len();
        out.extend(b"PK\x06\x06");
        out.extend(44u64.to_le_bytes());
        out.extend([45, 0, 45, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend(1u64.to_le_bytes());
        out.extend(1u64.to_le_bytes());
        out.extend((directory_size as u64).to_le_bytes());
        out.extend((directory as u64).to_le_bytes());
        out.extend(b"PK\x06\x07");
        out.extend(0u32.to_le_bytes());
        out.extend((record as u64).to_le_bytes());
        out.extend(1u32.to_le_bytes());
        out.extend(b"PK\x05\x06");
        out.extend([0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]);
        out.extend(0xFFFF_FFFFu32.to_le_bytes());
        out.extend(0xFFFF_FFFFu32.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        assert_eq!(super::entries(&out).unwrap(), vec![("a".to_string(), content.to_vec())]);
    }

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
