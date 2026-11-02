//! The boot image the board's flash holds after its bitstream: a header the boot ROM checks, then a DEFLATE-compressed
//! body of sections — the shim, the compiler (a static riscv64 Linux ELF), the bundle of LOGOS sources it may open,
//! and, when a run is given them, the compiler's arguments. The boot ROM refuses an image whose RTL fingerprint is not
//! the bitstream's, digests the compressed stream as it inflates it, and refuses one whose digest does not match.

use super::soc::{IMAGE_HEADER_SIZE, IMAGE_MAGIC, IMAGE_VERSION, SECTION_ARGS, SECTION_BUNDLE, SECTION_FIRMWARE, SECTION_SHIM};
use crate::crypto_accel::sha256;

/// What a boot image carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootImage {
    /// The fingerprint of the RTL the image was built for (the bitstream's `SYS_RTL_MD5`).
    pub rtl_md5: [u8; 16],
    /// The shim, as a raw binary for `SHIM_BASE`.
    pub shim: Vec<u8>,
    /// The compiler: a static riscv64 Linux ELF linked at `FIRMWARE_BASE`.
    pub firmware: Vec<u8>,
    /// The files the compiler may open, by path.
    pub bundle: Vec<(String, Vec<u8>)>,
    /// The compiler's arguments after its name, if the image runs it with any.
    pub args: Vec<String>,
}

/// The bundle's bytes: a u32 count, then per file its u32 name length, the name, its u64 length and its bytes.
pub fn bundle_bytes(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = (files.len() as u32).to_le_bytes().to_vec();
    for (name, data) in files {
        out.extend_from_slice(&(name.len() as u32).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&(data.len() as u64).to_le_bytes());
        out.extend_from_slice(data);
    }
    out
}

/// The arguments' bytes: each NUL-terminated.
fn args_bytes(args: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    for a in args {
        out.extend_from_slice(a.as_bytes());
        out.push(0);
    }
    out
}

/// The inflated body: a u32 section count and a reserved u32, the table of (u32 kind, u32 flags, u64 offset,
/// u64 length), then each section at an 8-byte-aligned offset from the body's start.
pub fn body(image: &BootImage) -> Vec<u8> {
    let mut sections: Vec<(u32, Vec<u8>)> = vec![
        (SECTION_SHIM, image.shim.clone()),
        (SECTION_FIRMWARE, image.firmware.clone()),
        (SECTION_BUNDLE, bundle_bytes(&image.bundle)),
    ];
    if !image.args.is_empty() {
        sections.push((SECTION_ARGS, args_bytes(&image.args)));
    }
    let table_end = 8 + 24 * sections.len();
    let mut offset = (table_end + 7) & !7;
    let mut table = Vec::new();
    let mut placed = Vec::new();
    for (kind, data) in &sections {
        table.extend_from_slice(&kind.to_le_bytes());
        table.extend_from_slice(&0u32.to_le_bytes());
        table.extend_from_slice(&(offset as u64).to_le_bytes());
        table.extend_from_slice(&(data.len() as u64).to_le_bytes());
        placed.push(offset);
        offset = (offset + data.len() + 7) & !7;
    }
    let mut out = vec![0u8; offset];
    out[0..4].copy_from_slice(&(sections.len() as u32).to_le_bytes());
    out[8..table_end].copy_from_slice(&table);
    for ((_, data), at) in sections.iter().zip(placed) {
        out[at..at + data.len()].copy_from_slice(data);
    }
    out
}

/// The body's compression: raw DEFLATE at the strongest level, the stream the boot ROM inflates.
pub fn pack(bytes: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec(bytes, 10)
}

/// The image as the flash holds it at `IMAGE_OFFSET`: the 128-byte header, then the compressed body.
pub fn build(image: &BootImage) -> Vec<u8> {
    let body = body(image);
    let packed = pack(&body);
    let mut header = vec![0u8; IMAGE_HEADER_SIZE as usize];
    header[0..8].copy_from_slice(&IMAGE_MAGIC.to_le_bytes());
    header[8..12].copy_from_slice(&IMAGE_VERSION.to_le_bytes());
    header[12..16].copy_from_slice(&(IMAGE_HEADER_SIZE as u32).to_le_bytes());
    header[0x10..0x20].copy_from_slice(&image.rtl_md5);
    header[0x20..0x28].copy_from_slice(&(body.len() as u64).to_le_bytes());
    header[0x28..0x30].copy_from_slice(&(packed.len() as u64).to_le_bytes());
    header[0x30..0x50].copy_from_slice(&sha256(&packed));
    header.extend_from_slice(&packed);
    header
}

/// An image read back: its fingerprint and its inflated body, every check the boot ROM makes applied.
pub fn open(flash_image: &[u8]) -> Result<([u8; 16], Vec<u8>), String> {
    let h = flash_image.get(..IMAGE_HEADER_SIZE as usize).ok_or("the image is shorter than its header")?;
    let word = |o: usize| u64::from_le_bytes(h[o..o + 8].try_into().unwrap());
    if word(0) != IMAGE_MAGIC {
        return Err("no boot image here".into());
    }
    if u32::from_le_bytes(h[8..12].try_into().unwrap()) != IMAGE_VERSION {
        return Err("the boot image's version is not this one".into());
    }
    let (body_len, packed_len) = (word(0x20) as usize, word(0x28) as usize);
    let packed = flash_image
        .get(IMAGE_HEADER_SIZE as usize..IMAGE_HEADER_SIZE as usize + packed_len)
        .ok_or("the image is shorter than its body")?;
    if sha256(packed)[..] != h[0x30..0x50] {
        return Err("the boot image's digest does not match".into());
    }
    let body = miniz_oxide::inflate::decompress_to_vec(packed).map_err(|e| format!("the body does not inflate: {e:?}"))?;
    if body.len() != body_len {
        return Err(format!("the body inflates to {} bytes, not {body_len}", body.len()));
    }
    Ok((h[0x10..0x20].try_into().unwrap(), body))
}
