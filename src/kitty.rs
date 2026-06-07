//! Pure builders for the Kitty graphics protocol escape sequences used by the
//! client to display images. No I/O — every function returns the exact bytes to
//! write to the terminal.

/// Standard base64 (RFC 4648) encoding of `data`.
pub fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18 & 63) as usize] as char);
        out.push(T[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6 & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// Transmit a PNG whose bytes are **already** base64-encoded, chunked into
/// ≤4096-char pieces per the protocol. The daemon base64-encodes blobs, so the
/// client uses this form (and never double-encodes).
pub fn transmit_b64(id: u64, data_b64: &str) -> String {
    let chunks: Vec<&str> = if data_b64.is_empty() {
        vec![""]
    } else {
        // base64 is ASCII, so byte-chunking respects char boundaries.
        data_b64.as_bytes().chunks(4096).map(|c| std::str::from_utf8(c).unwrap()).collect()
    };
    // Kitty image ids are 32-bit; our store ids are 64-bit content hashes, so
    // narrow to u32 (transmit and place stay consistent, collisions negligible).
    let id = id as u32;
    let mut out = String::new();
    let n = chunks.len();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = if i + 1 < n { 1 } else { 0 };
        if i == 0 {
            out.push_str(&format!("\x1b_Gf=100,a=t,t=d,i={id},q=2,m={more};{chunk}\x1b\\"));
        } else {
            out.push_str(&format!("\x1b_Gm={more};{chunk}\x1b\\"));
        }
    }
    out
}

/// Transmit a raw PNG image with id `id`.
pub fn transmit(id: u64, png: &[u8]) -> String {
    transmit_b64(id, &b64(png))
}

/// Place image `id` at the current cursor cell, scaled to `cols × rows` cells,
/// under placement id `place`. A distinct `place` lets one image (e.g. a shared
/// file-type icon) appear at many spots at once.
pub fn place(id: u64, place: u32, cols: u16, rows: u16) -> String {
    let id = id as u32; // Kitty image ids are 32-bit
    format!("\x1b_Ga=p,i={id},p={place},c={cols},r={rows},q=2\x1b\\")
}

/// Delete the placement `place` of image `id` (keeps the transmitted data).
pub fn delete(id: u64, place: u32) -> String {
    let id = id as u32; // Kitty image ids are 32-bit
    format!("\x1b_Ga=d,d=i,i={id},p={place},q=2\x1b\\")
}
