// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! QTMR — Quad Timer (MIMXRT1062.h `TMR_Type`; RM §53). TMR1..4, each with
//! four 16-bit channels. The SwiftIO Micro uses these for PWM and timing.
//!
//! Each channel counts its primary source and compares against `COMP1`; on a
//! match it sets `SCTRL.TCF` (and raises the shared TMR IRQ if `SCTRL.TCFIE`),
//! reloading `CNTR` from `LOAD` when `CTRL.LENGTH` is set. We model the
//! IP-bus (PERCLK) clock sources — `CTRL.PCS` 8..15 = PERCLK ÷ 2^(PCS-8) —
//! which cover timing/interval use; external-pin sources and capture are
//! ROADMAP items. Channel registers are 16-bit and packed two-per-word, so
//! the bus routes width-accurate access here (as for the eDMA TCD / FlexPWM).
//!
//! Channel map (0x20 stride): COMP1 0x00, LOAD 0x06, CNTR 0x0A, CTRL 0x0C,
//! SCTRL 0x0E.

const CH_STEP: u32 = 0x20;
const R_COMP1: u32 = 0x00;
const R_LOAD: u32 = 0x06;
const R_CNTR: u32 = 0x0A;
const R_CTRL: u32 = 0x0C;
const R_SCTRL: u32 = 0x0E;

const CTRL_LENGTH: u16 = 1 << 7;
const SCTRL_TCF: u16 = 1 << 15;
const SCTRL_TCFIE: u16 = 1 << 14;

pub struct Qtmr {
    pub index: u8,
    mem: Box<[u8; 0x4000]>,
    /// Per-channel prescaler accumulator toward the PCS divisor.
    presc: [u64; 4],
}

impl Qtmr {
    pub fn new(index: u8) -> Self {
        Self {
            index,
            mem: Box::new([0; 0x4000]),
            presc: [0; 4],
        }
    }

    #[inline]
    fn rd16(&self, off: u32) -> u16 {
        let o = off as usize;
        u16::from_le_bytes([self.mem[o], self.mem[o + 1]])
    }
    #[inline]
    fn wr16(&mut self, off: u32, v: u16) {
        let o = off as usize;
        self.mem[o..o + 2].copy_from_slice(&v.to_le_bytes());
    }

    // --- width-accurate register access ------------------------------------

    pub fn read16(&self, off: u32) -> u16 {
        self.rd16(off)
    }
    pub fn read8(&self, off: u32) -> u8 {
        self.mem[off as usize]
    }
    pub fn read32(&self, off: u32) -> u32 {
        u32::from(self.rd16(off)) | (u32::from(self.rd16(off + 2)) << 16)
    }
    pub fn write16(&mut self, off: u32, value: u16) {
        // SCTRL flags TCF/TOF/IEF are W0C (write 0 to clear); model TCF that
        // way so the driver's clear sticks, and let other bits store.
        if off % CH_STEP == R_SCTRL {
            let cur = self.rd16(off);
            // Keep control bits from the write; clear TCF only if written 0.
            let tcf = cur & value & SCTRL_TCF;
            self.wr16(off, (value & !SCTRL_TCF) | tcf);
        } else {
            self.wr16(off, value);
        }
    }
    pub fn write8(&mut self, off: u32, value: u8) {
        self.mem[off as usize] = value;
    }
    pub fn write32(&mut self, off: u32, value: u32) {
        self.write16(off, value as u16);
        self.write16(off + 2, (value >> 16) as u16);
    }

    // --- timing -------------------------------------------------------------

    /// Advance every counting channel by `perclk_ticks` PERCLK cycles.
    pub fn tick(&mut self, perclk_ticks: u64) {
        if perclk_ticks == 0 {
            return;
        }
        for ch in 0..4u32 {
            let base = ch * CH_STEP;
            let ctrl = self.rd16(base + R_CTRL);
            let cm = (ctrl >> 13) & 0x7;
            let pcs = (ctrl >> 9) & 0xF;
            if cm == 0 || pcs < 8 {
                continue; // stopped, or an unmodeled (external-pin) source
            }
            let div = 1u64 << (pcs - 8); // PERCLK ÷ 2^(PCS-8)
            let acc = self.presc[ch as usize] + perclk_ticks;
            let steps = acc / div;
            self.presc[ch as usize] = acc % div;
            if steps == 0 {
                continue;
            }
            self.advance(base, steps);
        }
    }

    fn advance(&mut self, base: u32, mut steps: u64) {
        let comp1 = self.rd16(base + R_COMP1);
        let load = self.rd16(base + R_LOAD);
        let length = self.rd16(base + R_CTRL) & CTRL_LENGTH != 0;
        let mut cntr = self.rd16(base + R_CNTR);
        let mut sctrl = self.rd16(base + R_SCTRL);
        while steps > 0 {
            if cntr == comp1 {
                sctrl |= SCTRL_TCF;
                cntr = if length { load } else { cntr.wrapping_add(1) };
            } else {
                cntr = cntr.wrapping_add(1);
            }
            steps -= 1;
        }
        self.wr16(base + R_CNTR, cntr);
        self.wr16(base + R_SCTRL, sctrl);
    }

    /// Any channel with `SCTRL.TCF` set and `TCFIE` enabled → the shared IRQ.
    pub fn irq_pending(&self) -> bool {
        (0..4u32).any(|ch| {
            let s = self.rd16(ch * CH_STEP + R_SCTRL);
            s & SCTRL_TCF != 0 && s & SCTRL_TCFIE != 0
        })
    }
}

impl Default for Qtmr {
    fn default() -> Self {
        Self::new(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_perclk_and_flags_compare() {
        let mut t = Qtmr::new(1);
        // CH0: COMP1=9, LOAD=0, CTRL: CM=1 (count), PCS=8 (PERCLK/1), LENGTH.
        t.write16(R_COMP1, 9);
        t.write16(R_LOAD, 0);
        t.write16(R_CTRL, (1 << 13) | (8 << 9) | CTRL_LENGTH);
        t.write16(R_SCTRL, SCTRL_TCFIE);
        t.tick(9);
        assert_eq!(t.read16(R_CNTR), 9);
        assert!(!t.irq_pending());
        t.tick(1); // CNTR == COMP1 → TCF, reload to LOAD
        assert_ne!(t.read16(R_SCTRL) & SCTRL_TCF, 0, "TCF set");
        assert!(t.irq_pending());
        assert_eq!(t.read16(R_CNTR), 0, "reloaded from LOAD");
        // W0C clear.
        t.write16(R_SCTRL, SCTRL_TCFIE);
        assert!(!t.irq_pending());
    }

    #[test]
    fn prescaler_divides_perclk() {
        let mut t = Qtmr::new(1);
        t.write16(R_COMP1, 0xFFFF);
        t.write16(R_CTRL, (1 << 13) | (10 << 9)); // PCS=10 → PERCLK/4
        t.tick(4);
        assert_eq!(t.read16(R_CNTR), 1);
        t.tick(12);
        assert_eq!(t.read16(R_CNTR), 4);
    }

    #[test]
    fn stopped_channel_does_not_count() {
        let mut t = Qtmr::new(1);
        t.write16(R_CTRL, 8 << 9); // PCS set but CM = 0 (no count)
        t.tick(100);
        assert_eq!(t.read16(R_CNTR), 0);
    }
}
