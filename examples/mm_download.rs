// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! Emulate MadMachine's `mm download` over the framed serial protocol.
//!
//! Boots the SwiftIO `SerialLoader` recovery bootloader in the emulator and
//! speaks the same protocol `mm-sdk/mm/src/serial_download.py` uses — a frame
//! is `PREAMBLE(8) | tag(4 BE) | length(4 BE) | payload | crc32(4 BE)` — over
//! LPUART1 (the bootloader's download UART; it logs on LPUART2). This drives a
//! RAM download and verifies the bytes land in the emulated SDRAM, exactly as
//! `mm download` would flash a physical board over USB-serial.
//!
//! `cargo run --release --example mm_download -- [file] [hex-addr]`
//! (defaults to a 1 KiB test pattern at `0x8000_0000`).

use rt1060_rs::{Rt1060, loader};

const PREAMBLE: [u8; 8] = [0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x5D];
const SYNC_TAG: u32 = 0x02;
const RAM_BEGIN_TAG: u32 = 0x0A;
const RAM_DATA_TAG: u32 = 0x0B;
const RAM_END_TAG: u32 = 0x0C;

/// CRC-32/ISO-HDLC (zlib `crc32`), computed over `tag | length | payload`.
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

/// Send one request frame into LPUART1 and return the device's response frame
/// body (`tag | length | payload | crc`), if one arrives.
fn transact(soc: &mut Rt1060, tag: u32, payload: &[u8]) -> Option<Vec<u8>> {
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
                return Some(out[p + 8..p + 16 + len + 4].to_vec());
            }
        }
    }
    None
}

/// A response is an ACK when tag byte 1 is 0 (success) and byte 3 echoes the
/// request tag.
fn acked(resp: &Option<Vec<u8>>, req_tag: u32) -> bool {
    matches!(resp, Some(r) if r.len() >= 4 && r[1] == 0 && r[3] == req_tag as u8)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let addr: u32 = args
        .get(2)
        .and_then(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0x8000_0000);
    let data: Vec<u8> = match args.get(1) {
        Some(path) => std::fs::read(path).expect("read file"),
        None => (0..1024u32).map(|i| (i ^ 0xA5) as u8).collect(),
    };

    let boot = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mm_serial_loader.bin"
    ))
    .expect("read SerialLoader.bin");
    let mut soc = Rt1060::boot(&loader::load_bin(0x0, &boot));
    soc.quiet();
    println!("booting the SerialLoader recovery bootloader…");
    for _ in 0..40_000_000u64 {
        soc.step();
    }
    for i in 0..2 {
        soc.bus.periph.lpuart[i].take_output();
    }

    let sync = transact(&mut soc, SYNC_TAG, &[]);
    println!(
        "SYNC:      {}",
        if acked(&sync, SYNC_TAG) {
            "ACK"
        } else {
            "FAIL"
        }
    );

    let mut begin = (addr as u64).to_be_bytes().to_vec();
    begin.extend_from_slice(&(data.len() as u32).to_be_bytes());
    println!("downloading {} bytes to {addr:#010x} …", data.len());
    let b = transact(&mut soc, RAM_BEGIN_TAG, &begin);
    println!(
        "RAM_BEGIN: {}",
        if acked(&b, RAM_BEGIN_TAG) {
            "ACK"
        } else {
            "FAIL"
        }
    );
    for chunk in data.chunks(65536) {
        let d = transact(&mut soc, RAM_DATA_TAG, chunk);
        println!(
            "RAM_DATA:  {} ({} bytes)",
            if acked(&d, RAM_DATA_TAG) {
                "ACK"
            } else {
                "FAIL"
            },
            chunk.len()
        );
    }
    let e = transact(&mut soc, RAM_END_TAG, &crc32(&data).to_be_bytes());
    println!(
        "RAM_END:   {}",
        if acked(&e, RAM_END_TAG) {
            "ACK"
        } else {
            "FAIL"
        }
    );

    let ok = (0..data.len() as u32).all(|i| soc.bus.read8(addr + i) == data[i as usize]);
    println!(
        "\n{} — {} bytes verified in SDRAM at {addr:#010x}",
        if ok { "download OK" } else { "MISMATCH" },
        data.len()
    );
}
