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
/// The real Blink `micro.img` (mm SDK: a 4 KiB header + the SDRAM payload).
const MICRO_IMG: &[u8] = include_bytes!("fixtures/madmachine_swiftio_blink.img");
const PREAMBLE: [u8; 8] = [0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x5D];
const SYNC_TAG: u32 = 0x02;
const RAM_BEGIN_TAG: u32 = 0x0A;
const RAM_DATA_TAG: u32 = 0x0B;
const RAM_END_TAG: u32 = 0x0C;
const PART_BEGIN_TAG: u32 = 0x38;
const PART_DATA_TAG: u32 = 0x39;
const PART_END_TAG: u32 = 0x3A;
const PART_SETBOOT_TAG: u32 = 0x3B;
const FS_FILE_BEGIN_TAG: u32 = 0x52;
const FS_FILE_DATA_TAG: u32 = 0x53;
const FS_FILE_END_TAG: u32 = 0x54;

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

/// `mm copy` a file to the on-board **littlefs** filesystem over the FS_FILE
/// tags (`fs_file_begin(path)` / `fs_file_data` / `fs_file_end`). The bootloader
/// mounts littlefs on the NOR (`/lfs`) and writes the file into it; the content
/// must then be present in the NOR littlefs region. The emulated equivalent of
/// `mm copy <file> /lfs/...`.
#[test]
#[ignore = "boots the bootloader + writes a littlefs file (~60M steps); cargo test --release -- --ignored"]
fn mm_download_writes_a_file_to_littlefs() {
    let mut soc = Rt1060::boot(&loader::load_bin(0x0, BOOTLOADER));
    soc.quiet();
    for _ in 0..40_000_000u64 {
        soc.step();
    }
    for i in 0..2 {
        soc.bus.periph.lpuart[i].take_output();
    }

    let content = b"HELLO_FROM_MM_FS_DOWNLOAD_0123456789_ABCDEF".to_vec();
    transact(&mut soc, SYNC_TAG, &[]);
    let mut begin = (content.len() as u32).to_be_bytes().to_vec();
    begin.extend_from_slice(b"/lfs/test.txt\0");
    transact(&mut soc, FS_FILE_BEGIN_TAG, &begin);
    transact(&mut soc, FS_FILE_DATA_TAG, &content);
    transact(&mut soc, FS_FILE_END_TAG, &crc32(&content).to_be_bytes());

    // The file's bytes must now be stored in the NOR littlefs partition (0x80_0000).
    let mut found = None;
    'scan: for off in 0x80_0000u32..0x81_0000 {
        for (i, &c) in content.iter().enumerate() {
            if soc.bus.read8(0x6000_0000 + off + i as u32) != c {
                continue 'scan;
            }
        }
        found = Some(off);
        break;
    }
    assert!(
        found.is_some(),
        "the file written over FS_FILE should land in the NOR littlefs"
    );
}

/// The full production deploy: `mm download` a real `micro.img` to the **NOR
/// flash** `user` partition (PART_BEGIN/DATA/END + SETBOOT), then **two-stage
/// boot** it — read the image back from NOR, parse its header, load the payload
/// to SDRAM, and run it, reaching the Zephyr application. This is the emulated
/// equivalent of `mm download <micro.img>` followed by a reset.
#[test]
#[ignore = "downloads a 154 KiB image + two-stage boots (~200M steps); cargo test --release -- --ignored"]
fn mm_download_flashes_micro_img_and_two_stage_boots() {
    let mut soc = Rt1060::boot(&loader::load_bin(0x0, BOOTLOADER));
    soc.quiet();
    for _ in 0..40_000_000u64 {
        soc.step();
    }
    for i in 0..2 {
        soc.bus.periph.lpuart[i].take_output();
    }

    // Program the image to the "user" flash partition.
    let mut name = b"user".to_vec();
    name.resize(64, 0);
    transact(&mut soc, SYNC_TAG, &[]);
    let mut begin = name.clone();
    begin.extend_from_slice(&(MICRO_IMG.len() as u32).to_be_bytes());
    transact(&mut soc, PART_BEGIN_TAG, &begin);
    for chunk in MICRO_IMG.chunks(65536) {
        transact(&mut soc, PART_DATA_TAG, chunk);
    }
    transact(&mut soc, PART_END_TAG, &crc32(MICRO_IMG).to_be_bytes());
    transact(&mut soc, PART_SETBOOT_TAG, &name);

    // The bootloader wrote the image to the NOR `user` partition (0xA_0000).
    let nor: Vec<u8> = (0..MICRO_IMG.len() as u32)
        .map(|i| soc.bus.read8(0x6000_0000 + 0xA_0000 + i))
        .collect();
    assert_eq!(nor, MICRO_IMG, "the micro.img should land in NOR verbatim");

    // Two-stage boot in the SAME SoC — model the Boot ROM / first-stage loader
    // reading the image out of NOR and staging it into SDRAM (not the harness).
    let load_addr = soc
        .cold_boot_from_flash(0xA_0000)
        .expect("cold-boot the flashed micro.img");
    assert_eq!(load_addr, 0x8000_0000, "payload staged to SDRAM");

    let mut console = String::new();
    for _ in 0..40 {
        for _ in 0..1_000_000u64 {
            soc.step();
        }
        console.push_str(&soc.console_string());
        if console.contains("LittleFS") {
            break;
        }
    }
    assert!(
        console.contains("LittleFS"),
        "the flashed image should two-stage boot into the Zephyr application"
    );
}
