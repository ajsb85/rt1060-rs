// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! Firmware loaders: ELF32, raw binary, and the MadMachine download image.
//!
//! The MadMachine toolchain emits an ELF, `objcopy`s it to a raw `.bin`, then
//! wraps it for download (SwiftIO Micro `micro.img` = a 4 KiB header + the
//! raw image; SwiftIO Board `swiftio.bin` = the raw image + a CRC32 trailer).
//! The user image links and runs from SDRAM at `0x8000_0000`
//! (`IMAGE_LOAD_ADDRESS`, `mm-sdk/mm/src/image.py`).

/// One contiguous load segment: raw bytes destined for `addr`.
pub struct Segment {
    pub addr: u32,
    pub data: Vec<u8>,
}

/// A parsed firmware image ready to copy into the bus.
pub struct LoadedImage {
    pub segments: Vec<Segment>,
    /// Program entry (ELF `e_entry`), if known. For images whose first word
    /// is the vector table, the SoC instead reads SP/PC from the table.
    pub entry: Option<u32>,
    /// The lowest load address seen — a good default VTOR when the image
    /// begins with its vector table.
    pub base: u32,
}

impl LoadedImage {
    fn from_single(addr: u32, data: Vec<u8>) -> Self {
        Self {
            segments: vec![Segment { addr, data }],
            entry: None,
            base: addr,
        }
    }
}

/// The SwiftIO Micro/Board user image links and runs from SDRAM.
pub const IMAGE_LOAD_ADDRESS: u32 = 0x8000_0000;

/// Load a raw binary at an explicit address (e.g. `objcopy -O binary`).
pub fn load_bin(addr: u32, data: &[u8]) -> LoadedImage {
    LoadedImage::from_single(addr, data.to_vec())
}

/// Parse a little-endian 32-bit ELF, taking every `PT_LOAD` segment at its
/// physical address. Returns `Err` on a malformed or non-ELF32-LE header.
pub fn load_elf(bytes: &[u8]) -> Result<LoadedImage, String> {
    if bytes.len() < 52 || &bytes[0..4] != b"\x7fELF" {
        return Err("not an ELF file".into());
    }
    if bytes[4] != 1 {
        return Err("not ELF32".into());
    }
    if bytes[5] != 1 {
        return Err("not little-endian".into());
    }
    let rd32 = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
    let rd16 = |o: usize| u16::from_le_bytes(bytes[o..o + 2].try_into().unwrap());

    let entry = rd32(24);
    let phoff = rd32(28) as usize;
    let phentsize = rd16(42) as usize;
    let phnum = rd16(44) as usize;

    let mut segments = Vec::new();
    let mut base = u32::MAX;
    for i in 0..phnum {
        let ph = phoff + i * phentsize;
        if ph + 32 > bytes.len() {
            return Err("truncated program header".into());
        }
        let p_type = rd32(ph);
        if p_type != 1 {
            continue; // PT_LOAD only
        }
        let p_offset = rd32(ph + 4) as usize;
        let p_paddr = rd32(ph + 12);
        let p_filesz = rd32(ph + 16) as usize;
        if p_filesz == 0 {
            continue;
        }
        if p_offset + p_filesz > bytes.len() {
            return Err("segment runs past end of file".into());
        }
        base = base.min(p_paddr);
        segments.push(Segment {
            addr: p_paddr,
            data: bytes[p_offset..p_offset + p_filesz].to_vec(),
        });
    }
    if segments.is_empty() {
        return Err("no PT_LOAD segments".into());
    }
    Ok(LoadedImage {
        segments,
        entry: Some(entry),
        base: if base == u32::MAX { entry } else { base },
    })
}

/// Parse a MadMachine SwiftIO Micro `micro.img`: a 4 KiB header followed by
/// the raw payload. The header (little-endian words, `image.py`) carries a
/// CRC32, the payload `offset` (0x1000), `size`, and `load_address`
/// (0x8000_0000). We honour the header's `offset`/`size`/`load_address`.
pub fn load_micro_img(bytes: &[u8]) -> Result<LoadedImage, String> {
    if bytes.len() < 0x1000 {
        return Err("image shorter than its 4 KiB header".into());
    }
    let rd32 = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
    // Header layout (image.py `create_image`), all little-endian:
    // [0] header CRC32, [4] offset (u64), [12] size (u64), [20] load_address
    // (u64), [28] type, [32] verify_type, [36] hash/CRC — then 0xFF pad to 4K.
    let offset = rd32(4) as usize;
    let size = rd32(12) as usize;
    let load_address = rd32(20);
    if offset == 0 || offset > bytes.len() {
        return Err("implausible payload offset in header".into());
    }
    let end = (offset + size).min(bytes.len());
    let load_address = if map_looks_like_ram(load_address) {
        load_address
    } else {
        IMAGE_LOAD_ADDRESS
    };
    Ok(LoadedImage::from_single(
        load_address,
        bytes[offset..end].to_vec(),
    ))
}

/// A sanity gate for a header-declared load address (SDRAM / OCRAM / TCM).
fn map_looks_like_ram(addr: u32) -> bool {
    matches!(addr >> 24, 0x80..=0x83 | 0x20 | 0x00)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bin_is_single_segment() {
        let img = load_bin(0x8000_0000, &[1, 2, 3, 4]);
        assert_eq!(img.segments.len(), 1);
        assert_eq!(img.segments[0].addr, 0x8000_0000);
        assert_eq!(img.base, 0x8000_0000);
    }

    #[test]
    fn rejects_non_elf() {
        assert!(load_elf(b"not an elf at all............").is_err());
    }

    #[test]
    fn parses_minimal_elf32_le() {
        // Hand-build a 1-segment ELF32-LE: header + one PT_LOAD ph + 4 bytes.
        let mut b = vec![0u8; 52 + 32 + 4];
        b[0..4].copy_from_slice(b"\x7fELF");
        b[4] = 1; // ELFCLASS32
        b[5] = 1; // little-endian
        b[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
        b[18..20].copy_from_slice(&40u16.to_le_bytes()); // EM_ARM
        b[24..28].copy_from_slice(&0x8000_0004u32.to_le_bytes()); // e_entry
        b[28..32].copy_from_slice(&52u32.to_le_bytes()); // e_phoff
        b[42..44].copy_from_slice(&32u16.to_le_bytes()); // e_phentsize
        b[44..46].copy_from_slice(&1u16.to_le_bytes()); // e_phnum
        let ph = 52;
        b[ph..ph + 4].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD
        b[ph + 4..ph + 8].copy_from_slice(&(52u32 + 32).to_le_bytes()); // p_offset
        b[ph + 8..ph + 12].copy_from_slice(&0x8000_0000u32.to_le_bytes()); // p_vaddr
        b[ph + 12..ph + 16].copy_from_slice(&0x8000_0000u32.to_le_bytes()); // p_paddr
        b[ph + 16..ph + 20].copy_from_slice(&4u32.to_le_bytes()); // p_filesz
        let payload = 52 + 32;
        b[payload..payload + 4].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);
        let img = load_elf(&b).expect("valid elf");
        assert_eq!(img.entry, Some(0x8000_0004));
        assert_eq!(img.base, 0x8000_0000);
        assert_eq!(img.segments[0].data, vec![0xAA, 0xBB, 0xCC, 0xDD]);
    }

    #[test]
    fn micro_img_honours_header() {
        // Real layout (image.py): [0] header CRC, [4] offset u64, [12] size u64,
        // [20] load_address u64 — the fields the loader must read.
        let mut b = vec![0xFFu8; 0x1000 + 8];
        b[4..12].copy_from_slice(&0x1000u64.to_le_bytes()); // offset
        b[12..20].copy_from_slice(&8u64.to_le_bytes()); // size
        b[20..28].copy_from_slice(&0x8000_0000u64.to_le_bytes()); // load_address
        b[0x1000..0x1000 + 8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let img = load_micro_img(&b).expect("valid img");
        assert_eq!(img.segments[0].addr, 0x8000_0000);
        assert_eq!(img.segments[0].data, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }
}
