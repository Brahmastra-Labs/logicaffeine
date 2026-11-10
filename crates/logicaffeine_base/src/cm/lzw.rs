//! GIF's LZW, both ways: [`decode`] reads an image's code stream into its pixel indices, and [`encode`] writes
//! them back the way the common encoders do — a clear code first, codes widening as the table grows, and a
//! clear code once the table holds `clear_at` entries — so a GIF whose encoder followed that policy can be held
//! as its pixels and rebuilt byte for byte. An encoder whose dictionary loses strings ends some matches early and
//! adds the string again: [`cuts`] finds the pixels where it did, and [`encode_cut`] writes its stream back from them.

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_base/cm/lzw/tests.rs"]
mod tests;

/// The table never holds more than 4096 codes (12 bits).
const TABLE: usize = 4096;

/// Decode a GIF LZW code stream with `min_code` bits per pixel (2..=8). `None` when a code refers past the table,
/// the stream ends before its end code, or more than `max_out` pixels would come out.
pub fn decode(data: &[u8], min_code: u8, max_out: usize) -> Option<Vec<u8>> {
    decode_with(data, min_code, max_out, |_| {})
}

/// [`decode`], telling `ended` where each code's string ends in the pixels.
fn decode_with(data: &[u8], min_code: u8, max_out: usize, mut ended: impl FnMut(usize)) -> Option<Vec<u8>> {
    if !(2..=8).contains(&min_code) {
        return None;
    }
    let clear = 1usize << min_code;
    let end = clear + 1;
    let mut prefix = vec![0u16; TABLE];
    let mut suffix = vec![0u8; TABLE];
    let mut first = vec![0u8; TABLE];
    let mut length = vec![0u16; TABLE];
    for c in 0..clear {
        suffix[c] = c as u8;
        first[c] = c as u8;
        length[c] = 1;
    }
    let mut next = clear + 2;
    let mut bits = u32::from(min_code) + 1;
    let (mut buf, mut have, mut pos) = (0u32, 0u32, 0usize);
    let mut prev: Option<usize> = None;
    let mut out = Vec::new();
    loop {
        while have < bits {
            buf |= u32::from(*data.get(pos)?) << have;
            have += 8;
            pos += 1;
        }
        let code = (buf & ((1 << bits) - 1)) as usize;
        buf >>= bits;
        have -= bits;
        if code == clear {
            next = clear + 2;
            bits = u32::from(min_code) + 1;
            prev = None;
            continue;
        }
        if code == end {
            return Some(out);
        }
        let entry = if code < next && (code < clear || code > end) {
            code
        } else if code == next && prev.is_some() {
            next
        } else {
            return None;
        };
        if let Some(p) = prev {
            if next < TABLE {
                prefix[next] = p as u16;
                suffix[next] = if entry == next { first[p] } else { first[entry] };
                first[next] = first[p];
                length[next] = length[p] + 1;
                next += 1;
                if next == 1 << bits && bits < 12 {
                    bits += 1;
                }
            }
        }
        let n = usize::from(length[entry]);
        if out.len() + n > max_out {
            return None;
        }
        let start = out.len();
        out.resize(start + n, 0);
        let mut c = entry;
        for i in (0..n).rev() {
            out[start + i] = suffix[c];
            c = usize::from(prefix[c]);
        }
        ended(out.len());
        prev = Some(entry);
    }
}

/// Encode pixel indices of `min_code` bits as a GIF LZW code stream, emitting a clear code first and again
/// whenever the table reaches `clear_at` entries (`clear_at` in `clear + 2 ..= 4096`).
pub fn encode(pixels: &[u8], min_code: u8, clear_at: u16) -> Vec<u8> {
    encode_cut(pixels, min_code, clear_at, &[])
}

/// [`encode`], ending the match at each pixel of `cuts` (ascending) although the table could carry it further —
/// how an encoder whose dictionary lost the longer string writes. The string it then adds takes a new code though
/// the table holds it, and later matches find the newer code.
pub fn encode_cut(pixels: &[u8], min_code: u8, clear_at: u16, cuts: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pixels.len() / 2 + 16);
    let (mut buf, mut have) = (0u64, 0u32);
    let mut pending = cuts.iter().map(|&c| c as usize).peekable();
    walk(
        pixels,
        min_code,
        clear_at,
        |at, _| {
            while pending.next_if(|&c| c < at).is_some() {}
            pending.next_if_eq(&at).is_some()
        },
        |code, bits| {
            buf |= (code as u64) << have;
            have += bits;
            while have >= 8 {
                out.push(buf as u8);
                buf >>= 8;
                have -= 8;
            }
        },
    );
    if have > 0 {
        out.push(buf as u8);
    }
    out
}

/// The pixels at which the encoder of `data` — a code stream of `min_code` bits per pixel whose table clears at
/// `clear_at` entries — ended a match the table could have carried further, with the decoded pixels: from them
/// [`encode_cut`] writes the codes again. `None` when `data` does not decode within `max_out` pixels or one of its
/// matches runs where the table cannot carry it.
pub fn cuts(data: &[u8], min_code: u8, clear_at: u16, max_out: usize) -> Option<(Vec<u8>, Vec<u32>)> {
    let mut ends = Vec::new();
    let pixels = decode_with(data, min_code, max_out, |end| ends.push(end))?;
    let mut is_end = vec![false; pixels.len() + 1];
    for end in ends {
        is_end[end] = true;
    }
    let mut cuts = Vec::new();
    let mut fits = true;
    walk(
        &pixels,
        min_code,
        clear_at,
        |at, carries| {
            if is_end[at] && carries {
                cuts.push(at as u32);
            }
            fits &= is_end[at] || carries;
            is_end[at]
        },
        |_, _| {},
    );
    fits.then_some((pixels, cuts))
}

/// The greedy match over `pixels` that [`encode_cut`] writes. At each pixel the current match could end at,
/// `ends(at, carries)` says whether it does (`carries`: whether the table could carry it on; it ends anyway when
/// not); `emit` takes each code and its width in bits — the opening clear, the matches, the clears and the end.
fn walk(pixels: &[u8], min_code: u8, clear_at: u16, mut ends: impl FnMut(usize, bool) -> bool, mut emit: impl FnMut(usize, u32)) {
    let clear = 1usize << min_code;
    let end = clear + 1;
    let mut bits = u32::from(min_code) + 1;
    // The table as (code << 8 | next pixel) → extended code, in an open-addressed hash of a fixed size; a string
    // added again overwrites its key, so the newest code is the one found.
    let mut keys = vec![u32::MAX; 1 << 14];
    let mut values = vec![0u16; 1 << 14];
    let slot = |key: u32| ((key.wrapping_mul(0x9E37_79B1)) >> 18) as usize;
    let mut next = clear + 2;
    emit(clear, bits);
    let Some((&head, rest)) = pixels.split_first() else {
        emit(end, bits);
        return;
    };
    let mut cur = usize::from(head);
    for (k, &b) in rest.iter().enumerate() {
        let key = (cur as u32) << 8 | u32::from(b);
        let mut i = slot(key);
        while keys[i] != u32::MAX && keys[i] != key {
            i = (i + 1) & ((1 << 14) - 1);
        }
        let carries = keys[i] == key;
        if !ends(k + 1, carries) && carries {
            cur = usize::from(values[i]);
            continue;
        }
        emit(cur, bits);
        if next < usize::from(clear_at) {
            keys[i] = key;
            values[i] = next as u16;
            next += 1;
            if next > 1 << bits && bits < 12 {
                bits += 1;
            }
        } else {
            emit(clear, bits);
            keys.fill(u32::MAX);
            next = clear + 2;
            bits = u32::from(min_code) + 1;
        }
        cur = usize::from(b);
    }
    emit(cur, bits);
    emit(end, bits);
}
