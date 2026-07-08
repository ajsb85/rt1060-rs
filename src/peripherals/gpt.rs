// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! GPT — General Purpose Timer (MIMXRT1062.h `GPT_Type`; RM §45).
//!
//! A free-running up-counter with three output-compare channels. The model
//! advances the counter from the SoC clock (`tick`) through the prescaler
//! and raises the compare status/interrupt when CNT reaches an OCR. Enough
//! for Zephyr's counter driver and busy-wait delays; capture/restart modes
//! land in the ROADMAP.
//!
//! Register offsets: CR 0x00, PR 0x04, SR 0x08, IR 0x0C, OCR1 0x10,
//! OCR2 0x14, OCR3 0x18, ICR1 0x1C, ICR2 0x20, CNT 0x24.

const CR_EN: u32 = 1 << 0;
const CR_ENMOD: u32 = 1 << 1;
const CR_SWR: u32 = 1 << 15; // software reset (self-clearing; RM §45.5.1)
/// CR.CLKSRC (bits [8:6], RM §45.5.1): 0 = off, 1 = ipg PERCLK, 4 = low-freq
/// 32.768 kHz, 5 = 24 MHz crystal; 2/3 (high-freq/external) approximated as
/// PERCLK for now.
const CR_CLKSRC_SHIFT: u32 = 6;

pub struct Gpt {
    cr: u32,
    pr: u32,
    sr: u32, // status: OF1..3 (bits 0..2), IF1..2 (3..4), ROV (5)
    ir: u32, // interrupt enables, same bit layout as SR
    ocr: [u32; 3],
    cnt: u32,
    /// Prescaler accumulator (counts source ticks toward PR+1).
    presc_acc: u32,
}

impl Gpt {
    pub fn new() -> Self {
        Self {
            cr: 0,
            pr: 0,
            sr: 0,
            ir: 0,
            ocr: [0xFFFF_FFFF; 3],
            cnt: 0,
            presc_acc: 0,
        }
    }

    pub fn read(&mut self, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.pr,
            0x08 => self.sr,
            0x0C => self.ir,
            0x10 => self.ocr[0],
            0x14 => self.ocr[1],
            0x18 => self.ocr[2],
            0x24 => self.cnt,
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: u32, value: u32) {
        match offset {
            0x00 => {
                // CR.SWR resets the block and self-clears (GPT_Init spins on
                // `while (CR & SWR)`), so it must never read back set.
                if value & CR_SWR != 0 {
                    *self = Gpt::new();
                    return;
                }
                let was_en = self.cr & CR_EN != 0;
                self.cr = value;
                if value & CR_EN != 0 && !was_en && value & CR_ENMOD != 0 {
                    // ENMOD: restart the counter from 0 on enable.
                    self.cnt = 0;
                    self.presc_acc = 0;
                }
            }
            0x04 => self.pr = value & 0x0FFF,
            0x08 => self.sr &= !value, // W1C
            0x0C => self.ir = value,
            0x10 => self.ocr[0] = value,
            0x14 => self.ocr[1] = value,
            0x18 => self.ocr[2] = value,
            _ => {}
        }
    }

    /// CR.CLKSRC — which clock source drives the counter (0 = stopped).
    pub fn clksrc(&self) -> u32 {
        (self.cr >> CR_CLKSRC_SHIFT) & 0x7
    }

    /// Advance the counter by `cycles` **source** clocks through the
    /// prescaler. `cycles` is already in the domain selected by `clksrc`
    /// (the aggregate converts core cycles to the right frequency).
    pub fn tick(&mut self, cycles: u64) {
        if self.cr & CR_EN == 0 || self.clksrc() == 0 {
            return;
        }
        let div = self.pr + 1; // PR is "divide by PR+1"
        let mut ticks = self.presc_acc as u64 + cycles;
        let steps = ticks / div as u64;
        self.presc_acc = (ticks % div as u64) as u32;
        ticks = steps;
        while ticks > 0 {
            let prev = self.cnt;
            self.cnt = self.cnt.wrapping_add(1);
            if self.cnt < prev {
                self.sr |= 1 << 5; // ROV rollover
            }
            for (i, &ocr) in self.ocr.iter().enumerate() {
                if self.cnt == ocr {
                    self.sr |= 1 << i; // OFn
                }
            }
            ticks -= 1;
        }
    }

    pub fn irq_pending(&self) -> bool {
        self.sr & self.ir & 0x3F != 0
    }
}

impl Default for Gpt {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_compares() {
        let mut g = Gpt::new();
        g.write(0x10, 10); // OCR1 = 10
        g.write(0x0C, 1 << 0); // IR: OF1 enable
        g.write(0x00, CR_EN | (1 << 6)); // EN, CLKSRC=perclk, PR=0
        g.tick(9);
        assert_eq!(g.read(0x24), 9);
        assert!(!g.irq_pending());
        g.tick(1); // CNT -> 10 == OCR1
        assert_ne!(g.read(0x08) & 1, 0, "OF1 set");
        assert!(g.irq_pending());
        g.write(0x08, 1 << 0); // W1C OF1
        assert!(!g.irq_pending());
    }

    #[test]
    fn prescaler_divides() {
        let mut g = Gpt::new();
        g.write(0x04, 3); // PR=3 -> divide by 4
        g.write(0x00, CR_EN | (1 << 6));
        g.tick(4);
        assert_eq!(g.read(0x24), 1);
        g.tick(12);
        assert_eq!(g.read(0x24), 4);
    }
}
