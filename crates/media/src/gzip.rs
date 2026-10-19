//! gzip (RFC 1952) decompression over the DEFLATE decoder written for OpenEXR, for documents
//! stored gzip-compressed (Premiere Pro projects). Every input is hostile: a bad header, truncated
//! data or output beyond the caller's limit is an error, never a panic.

/// Why a stream did not decompress.
pub type Error = &'static str;

const FHCRC: u8 = 0x02;
const FEXTRA: u8 = 0x04;
const FNAME: u8 = 0x08;
const FCOMMENT: u8 = 0x10;

/// Whether `data` starts like a gzip stream.
pub fn is_gzip(data: &[u8]) -> bool {
    data.starts_with(&[0x1f, 0x8b])
}

/// Decompress the first member of a gzip stream, producing at most `max` bytes.
pub fn decompress(data: &[u8], max: usize) -> Result<Vec<u8>, Error> {
    let header = data.get(..10).ok_or("truncated gzip header")?;
    if !is_gzip(header) {
        return Err("not a gzip stream");
    }
    if header.get(2) != Some(&8) {
        return Err("gzip stream is not DEFLATE");
    }
    let flags = header.get(3).copied().unwrap_or(0);
    let mut pos = 10usize;
    if flags & FEXTRA != 0 {
        let len = match data.get(pos..pos + 2) {
            Some([a, b]) => usize::from(u16::from_le_bytes([*a, *b])),
            _ => return Err("truncated gzip extra field"),
        };
        pos = pos.checked_add(2 + len).ok_or("gzip extra field too long")?;
    }
    for flag in [FNAME, FCOMMENT] {
        if flags & flag != 0 {
            let rest = data.get(pos..).ok_or("truncated gzip header")?;
            let end = rest.iter().position(|b| *b == 0).ok_or("unterminated gzip header string")?;
            pos = pos.checked_add(end + 1).ok_or("truncated gzip header")?;
        }
    }
    if flags & FHCRC != 0 {
        pos = pos.checked_add(2).ok_or("truncated gzip header")?;
    }
    let body = data.get(pos..).ok_or("truncated gzip stream")?;
    // the trailer's ISIZE is the uncompressed size modulo 2^32: a good first guess for the limit
    let stored = match data.len().checked_sub(4).and_then(|i| data.get(i..)) {
        Some([a, b, c, d]) => u32::from_le_bytes([*a, *b, *c, *d]) as usize,
        _ => return Err("truncated gzip trailer"),
    };
    let limit = if stored > 0 && stored <= max { stored } else { max };
    crate::still::exr::inflate::inflate(body, limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "hello hello hello\n" compressed by Python's gzip module with the file name "a.txt".
    const HELLO: [u8; 35] = [
        0x1f, 0x8b, 0x08, 0x08, 0x00, 0x00, 0x00, 0x00, 0x02, 0xff, 0x61, 0x2e, 0x74, 0x78, 0x74, 0x00, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0x40, 0x90,
        0x5c, 0x00, 0x3b, 0x7c, 0x8a, 0xdf, 0x12, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn decompresses_a_gzip_file() {
        assert!(is_gzip(&HELLO));
        assert_eq!(decompress(&HELLO, 1 << 20).unwrap(), b"hello hello hello\n");
        assert!(decompress(&HELLO, 4).is_err(), "more than the limit");
    }

    #[test]
    fn rejects_garbage_without_panicking() {
        assert!(decompress(&[], 100).is_err());
        assert!(decompress(b"<?xml version", 100).is_err());
        assert!(decompress(&[0x1f, 0x8b, 8, 0x08, 0, 0, 0, 0, 0, 3, b'a'], 100).is_err(), "unterminated name");
        assert!(decompress(&[0x1f, 0x8b, 8, 0x04, 0, 0, 0, 0, 0, 3, 0xff, 0xff], 100).is_err(), "extra field past the end");
        for n in 0..HELLO.len() {
            let _ = decompress(&HELLO[..n], 100);
        }
    }
}
