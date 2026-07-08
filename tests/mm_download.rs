// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! Emulate MadMachine's `mm download` end-to-end. Boots the real SwiftIO
//! `SerialLoader` recovery bootloader in the emulator and drives the same
//! framed serial protocol `mm-sdk/mm/src/serial_download.py` uses — a frame is
//! `PREAMBLE(8) | tag(4 BE) | length(4 BE) | payload | crc32(4 BE)` — over
//! LPUART1 (the bootloader's download UART; it logs on LPUART2). A RAM download
//! is driven and the bytes are checked to land in the emulated SDRAM, exactly
//! as `mm download` deploys to a physical board over USB-serial.

use rt1060_rs::{Rt1060, loader};

const BOOTLOADER: &[u8] = include_bytes!("fixtures/mm_serial_loader.bin");
const PREAMBLE: [u8; 8] = [0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x5D];
const SYNC_TAG: u32 = 0x02;
const RAM_BEGIN_TAG: u32 = 0x0A;
const RAM_DATA_TAG: u32 = 0x0B;
const RAM_END_TAG: u32 = 0x0C;

/// CRC-32/ISO-HDLC (zlib `crc32`) over `tag | length | payload`.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn frame(tag: u32, payload: &[u8]) -> Vec<u8> {
    let mut body = tag.to_be_bytes().to_vec();
    body.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    body.extend_from_slice(payload);
    let mut f = PREAMBLE.to_vec();
    f.extend_from_slice(&body);
    f.extend_from_slice(&crc32(&body).to_be_bytes());
    f
}

/// Send a request frame on LPUART1, run until the response frame arrives, and
/// assert it is an ACK for `tag` (byte 1 = success, byte 3 echoes the tag).
fn transact(soc: &mut Rt1060, tag: u32, payload: &[u8]) {
    for b in frame(tag, payload) {
        soc.bus.periph.lpuart[0].rx_push(b);
    }
    let mut out = Vec::new();
    for _ in 0..40 {
        for _ in 0..500_000u64 {
            soc.step();
        }
        out.extend(soc.bus.periph.lpuart[0].take_output());
        if let Some(p) = out.windows(8).position(|w| w == PREAMBLE)
            && out.len() >= p + 16
        {
            let len = u32::from_be_bytes(out[p + 12..p + 16].try_into().unwrap()) as usize;
            if out.len() >= p + 16 + len + 4 {
                let resp = &out[p + 8..p + 16 + len + 4];
                assert_eq!(resp[1], 0, "download tag {tag:#x} returned an error");
                assert_eq!(
                    resp[3], tag as u8,
                    "download tag {tag:#x} response mismatch"
                );
                return;
            }
        }
    }
    panic!("no response to download tag {tag:#x}");
}

/// The real deploy flow: SYNC, then a RAM download whose bytes must appear in
/// SDRAM — the emulated equivalent of `mm download <image>`.
#[test]
#[ignore = "boots the bootloader + drives the protocol (~100M steps); cargo test --release -- --ignored"]
fn mm_download_ram_lands_in_sdram() {
    let mut soc = Rt1060::boot(&loader::load_bin(0x0, BOOTLOADER));
    soc.quiet();
    // Boot the recovery bootloader to its idle "waiting for download" state.
    for _ in 0..40_000_000u64 {
        soc.step();
    }
    for i in 0..2 {
        soc.bus.periph.lpuart[i].take_output();
    }

    let addr: u32 = 0x8000_0000; // SDRAM (the MadMachine user-image location)
    let data: Vec<u8> = (0..1024u32)
        .map(|i| (i.wrapping_mul(31) ^ 0xA5) as u8)
        .collect();

    transact(&mut soc, SYNC_TAG, &[]);
    let mut begin = (addr as u64).to_be_bytes().to_vec();
    begin.extend_from_slice(&(data.len() as u32).to_be_bytes());
    transact(&mut soc, RAM_BEGIN_TAG, &begin);
    transact(&mut soc, RAM_DATA_TAG, &data);
    transact(&mut soc, RAM_END_TAG, &crc32(&data).to_be_bytes());

    let landed: Vec<u8> = (0..data.len() as u32)
        .map(|i| soc.bus.read8(addr + i))
        .collect();
    assert_eq!(
        landed, data,
        "the RAM download should place its bytes verbatim in SDRAM"
    );
}
