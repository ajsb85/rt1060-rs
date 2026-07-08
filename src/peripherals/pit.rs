// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! PIT — Periodic Interrupt Timer (MIMXRT1062.h `PIT_Type`; RM §52).
//!
//! Four independent 32-bit down-counters clocked from PERCLK. Each channel
//! loads `LDVAL`, counts down to 0 (that is `LDVAL + 1` PERCLK ticks), sets
//! `TFLG.TIF` and reloads. A channel with `TCTRL.CHN` counts the previous
//! channel's timeouts instead of PERCLK (cascade). All four share one NVIC
//! line, `PIT_IRQn = 122`.
//!
//! Register map: MCR 0x00, then per channel `n` at 0x100 + n*0x10:
//! LDVAL (+0x00), CVAL (+0x04, RO), TCTRL (+0x08), TFLG (+0x0C).

const MCR_FRZ: u32 = 0x1;
const MCR_MDIS: u32 = 0x2;
const TCTRL_TEN: u32 = 0x1;
const TCTRL_TIE: u32 = 0x2;
const TCTRL_CHN: u32 = 0x4;
const TFLG_TIF: u32 = 0x1;

/// External interrupt line shared by all four channels.
pub const PIT_IRQ: u32 = 122;

#[derive(Clone, Copy, Default)]
struct Channel {
    ldval: u32,
    cval: u32,
    tctrl: u32,
    tflg: u32,
}

pub struct Pit {
    mcr: u32,
    ch: [Channel; 4],
}

impl Pit {
    pub fn new() -> Self {
        // MCR reset value = MDIS set (module disabled until firmware clears it,
        // RM §52.9.1).
        Self {
            mcr: MCR_MDIS,
            ch: [Channel::default(); 4],
        }
    }

    pub fn read(&mut self, offset: u32) -> u32 {
        match offset {
            0x00 => self.mcr,
            0xE4 => self.ch[0].cval, // LTMR64L (lifetime low ~ ch0 CVAL)
            0xE0 => self.ch[1].cval, // LTMR64H (lifetime high ~ ch1 CVAL)
            _ if (0x100..0x140).contains(&offset) => {
                let c = ((offset - 0x100) / 0x10) as usize;
                match (offset - 0x100) % 0x10 {
                    0x00 => self.ch[c].ldval,
                    0x04 => self.ch[c].cval,
                    0x08 => self.ch[c].tctrl,
                    0x0C => self.ch[c].tflg,
                    _ => 0,
                }
            }
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: u32, value: u32) {
        match offset {
            0x00 => self.mcr = value & (MCR_FRZ | MCR_MDIS),
            _ if (0x100..0x140).contains(&offset) => {
                let c = ((offset - 0x100) / 0x10) as usize;
                match (offset - 0x100) % 0x10 {
                    0x00 => self.ch[c].ldval = value,
                    0x08 => {
                        let was_en = self.ch[c].tctrl & TCTRL_TEN != 0;
                        self.ch[c].tctrl = value;
                        // Enabling a channel loads CVAL from LDVAL.
                        if value & TCTRL_TEN != 0 && !was_en {
                            self.ch[c].cval = self.ch[c].ldval;
                        }
                    }
                    0x0C => self.ch[c].tflg &= !(value & TFLG_TIF), // W1C
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// Advance every running channel by `perclk_ticks` PERCLK cycles.
    pub fn tick(&mut self, perclk_ticks: u64) {
        if self.mcr & MCR_MDIS != 0 || perclk_ticks == 0 {
            return;
        }
        let mut prev_timeouts = 0u64;
        for c in 0..4 {
            let ch = &mut self.ch[c];
            if ch.tctrl & TCTRL_TEN == 0 {
                prev_timeouts = 0;
                continue;
            }
            // A cascaded channel is clocked by the previous channel's timeouts.
            let input = if c > 0 && ch.tctrl & TCTRL_CHN != 0 {
                prev_timeouts
            } else {
                perclk_ticks
            };
            let timeouts = Self::advance(ch, input);
            if timeouts > 0 {
                ch.tflg |= TFLG_TIF;
            }
            prev_timeouts = timeouts;
        }
    }

    /// Count `input` ticks into one channel; return how many times it timed
    /// out (reached 0 and reloaded).
    fn advance(ch: &mut Channel, input: u64) -> u64 {
        if input == 0 {
            return 0;
        }
        let period = u64::from(ch.ldval) + 1; // LDVAL..0 inclusive
        let cval = u64::from(ch.cval);
        if input <= cval {
            ch.cval = (cval - input) as u32;
            return 0;
        }
        // Consume down to (and through) the first timeout, then whole periods.
        let after_first = input - cval - 1;
        let timeouts = 1 + after_first / period;
        ch.cval = (ch.ldval as u64 - (after_first % period)) as u32;
        timeouts
    }

    /// Any channel with `TIF` set and `TIE` enabled → the shared PIT IRQ.
    pub fn irq_pending(&self) -> bool {
        self.ch
            .iter()
            .any(|c| c.tflg & TFLG_TIF != 0 && c.tctrl & TCTRL_TIE != 0)
    }
}

impl Default for Pit {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enable_ch0(pit: &mut Pit, ldval: u32, tie: bool) {
        pit.write(0x00, 0); // MCR: clear MDIS
        pit.write(0x100, ldval); // LDVAL0
        let tctrl = TCTRL_TEN | if tie { TCTRL_TIE } else { 0 };
        pit.write(0x108, tctrl); // TCTRL0
    }

    #[test]
    fn counts_down_and_fires() {
        let mut pit = Pit::new();
        enable_ch0(&mut pit, 9, true); // period = 10 ticks
        assert_eq!(pit.read(0x104), 9, "CVAL loaded from LDVAL on enable");
        pit.tick(9);
        assert_eq!(pit.read(0x104), 0);
        assert!(!pit.irq_pending(), "not timed out until it passes 0");
        pit.tick(1); // 10th tick → timeout + reload
        assert_ne!(pit.read(0x10C) & TFLG_TIF, 0, "TIF set");
        assert!(pit.irq_pending());
        assert_eq!(pit.read(0x104), 9, "reloaded");
        pit.write(0x10C, TFLG_TIF); // W1C
        assert!(!pit.irq_pending());
    }

    #[test]
    fn disabled_module_does_not_count() {
        let mut pit = Pit::new(); // MDIS set at reset
        pit.write(0x100, 4);
        pit.write(0x108, TCTRL_TEN);
        pit.tick(100);
        assert!(!pit.irq_pending());
    }

    #[test]
    fn large_tick_count_multiple_periods() {
        let mut pit = Pit::new();
        enable_ch0(&mut pit, 9, true); // period 10
        pit.tick(25); // 2 full timeouts + 5 into the third period
        assert!(pit.irq_pending());
        assert_eq!(pit.read(0x104), 4, "cval = 9 - (25-10) % 10 = 4");
    }

    #[test]
    fn cascade_channel_counts_previous_timeouts() {
        let mut pit = Pit::new();
        pit.write(0x00, 0); // enable module
        // ch0: period 5. ch1: chained, period 2 (counts ch0 timeouts).
        pit.write(0x100, 4);
        pit.write(0x108, TCTRL_TEN);
        pit.write(0x110, 1); // LDVAL1 = 1 → period 2
        pit.write(0x118, TCTRL_TEN | TCTRL_CHN | TCTRL_TIE);
        pit.tick(20); // ch0 times out 4× → ch1 counts 4 → times out 2×
        assert_ne!(pit.read(0x11C) & TFLG_TIF, 0, "chained TIF set");
    }
}
