// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! End-to-end smoke test: build a tiny SDRAM image whose reset handler
//! enables LPUART1 and transmits a string, then assert the console output —
//! exercising the loader, the vector-table reset path, the AIPS bus, and the
//! LPUART model together.

use rt1060_rs::memory::map;
use rt1060_rs::peripherals::base;
use rt1060_rs::{Rt1060, loader};

/// Encode the handful of Thumb-16 instructions we need without pulling the
/// crate's test-only `asm` module (it is `#[cfg(test)]`, not available to an
/// integration test). These encodings are fixed by the Armv7-M ARM.
mod thumb {
    pub fn movs_imm8(rd: u16, imm8: u16) -> u16 {
        0b00100 << 11 | (rd & 7) << 8 | (imm8 & 0xff)
    }
    pub fn lsls_imm(rd: u16, rm: u16, imm5: u16) -> u16 {
        (imm5 & 0x1f) << 6 | (rm & 7) << 3 | (rd & 7)
    }
    pub fn adds_imm8(rd: u16, imm8: u16) -> u16 {
        0b00110 << 11 | (rd & 7) << 8 | (imm8 & 0xff)
    }
    /// str rt, [rn, #imm] (imm is a byte offset, word-scaled in the encoding).
    pub fn str_imm(rt: u16, rn: u16, imm: u16) -> u16 {
        0b01100 << 11 | ((imm >> 2) & 0x1f) << 6 | (rn & 7) << 3 | (rt & 7)
    }
    pub fn b(offset: i32) -> u16 {
        (0b11100 << 11 | ((offset >> 1) as u32 & 0x7ff)) as u16
    }
}

#[test]
fn firmware_prints_ok_on_lpuart1() {
    use thumb::*;
    // With only 8-bit immediates we build the LPUART1 base (0x4018_4000) in
    // two parts and add them: r0 = 0x4018_0000 (from 0x4018 << 16) plus
    // r1 = 0x4000 (from 0x40 << 8). Then set CTRL.TE (1<<19) and push bytes.
    let code = [
        movs_imm8(0, 0x40), // r0 = 0x40
        lsls_imm(0, 0, 8),  // r0 = 0x4000
        adds_imm8(0, 0x18), // r0 = 0x4018
        lsls_imm(0, 0, 16), // r0 = 0x4018_0000
        movs_imm8(1, 0x40), // r1 = 0x40
        lsls_imm(1, 1, 8),  // r1 = 0x4000
        // r0 = 0x4018_0000 + 0x4000 = 0x4018_4000 via a store base trick:
        // add r0, r1 is a Thumb-16 (0x1840). Encode directly:
        0x1840, // adds r0, r0, r1  -> 0x4018_4000 (LPUART1 base)
        // r1 = CTRL TE (1<<19): r1 = 0x08; r1 <<= 16 -> 0x0008_0000; store CTRL@0x18
        movs_imm8(1, 0x08),  // r1 = 0x08
        lsls_imm(1, 1, 16),  // r1 = 0x0008_0000 (1<<19)
        str_imm(1, 0, 0x18), // CTRL = TE
        movs_imm8(1, b'O' as u16),
        str_imm(1, 0, 0x1C), // DATA = 'O'
        movs_imm8(1, b'K' as u16),
        str_imm(1, 0, 0x1C), // DATA = 'K'
        b(-4),               // spin
    ];

    let code_addr = map::SDRAM_BASE + 0x100;
    let mut data = Vec::new();
    data.extend_from_slice(&(map::DTCM_BASE + 0x1000).to_le_bytes()); // SP
    data.extend_from_slice(&(code_addr | 1).to_le_bytes()); // reset (thumb)
    data.resize(0x100, 0);
    for hw in code {
        data.extend_from_slice(&hw.to_le_bytes());
    }

    let image = loader::load_bin(map::SDRAM_BASE, &data);
    let mut soc = Rt1060::boot(&image);
    soc.quiet();

    assert_eq!(soc.core.regs[15] & !1, code_addr, "reset vector honoured");
    soc.run(60);
    assert_eq!(soc.console_string(), "OK");
    // Sanity: the base really was LPUART1.
    assert_eq!(base::LPUART1, 0x4018_4000);
}
