// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! ADC — 12-bit SAR ADC (MIMXRT1062.h `ADC_Type`; RM §66). ADC1/ADC2, up to
//! 16 external channels each (the SwiftIO Micro exposes 14 as A0–A13).
//!
//! Software-triggered model: writing a channel number to a control register
//! `HCn` starts a conversion whose result lands in `Rn`, with `HS.COCOn` set
//! on completion; `HCn.AIEN` gates the interrupt (ADC1 = IRQ 67, ADC2 = 68).
//! Reading `Rn` clears its COCO flag (RM §66). Per-channel input values are
//! programmable via [`Adc::set_channel`] so firmware/tests can drive analog
//! inputs. Auto-calibration (`GC.CAL`) completes instantly so
//! `ADC_DoAutoCalibration` returns; hardware trigger (ADC_ETC) and averaging
//! are ROADMAP items.
//!
//! Register map: HC0..7 0x00..0x1C, HS 0x20, R0..7 0x24..0x40, CFG 0x44,
//! GC 0x48, GS 0x4C, CV 0x50, OFS 0x54, CAL 0x58.

const HC_ADCH: u32 = 0x1F; // channel select
const HC_AIEN: u32 = 1 << 7; // interrupt enable
const HC_DISABLED: u32 = 0x1F; // ADCH = 0b11111 → conversion disabled
const GC_CAL: u32 = 1 << 7; // GC.CAL: launch auto-calibration (self-clearing)

/// Number of control/result register pairs (HC0..7 / R0..7).
const SLOTS: usize = 8;
/// Number of external input channels modeled.
const CHANNELS: usize = 16;

pub struct Adc {
    pub index: u8,
    hc: [u32; SLOTS],
    r: [u32; SLOTS],
    /// HS.COCO flags, one per slot.
    coco: u32,
    cfg: u32,
    gc: u32,
    cal: u32,
    /// Programmable analog input value per channel (12-bit).
    channels: [u16; CHANNELS],
}

impl Adc {
    pub fn new(index: u8) -> Self {
        Self {
            index,
            hc: [HC_DISABLED; SLOTS],
            r: [0; SLOTS],
            coco: 0,
            cfg: 0,
            gc: 0,
            cal: 0,
            // Mid-scale (0x800) is a sensible default for an idle input.
            channels: [0x0800; CHANNELS],
        }
    }

    /// Set the value a given input channel converts to (12-bit, 0..=4095).
    pub fn set_channel(&mut self, channel: u8, value: u16) {
        if (channel as usize) < CHANNELS {
            self.channels[channel as usize] = value & 0x0FFF;
        }
    }

    pub fn read(&mut self, offset: u32) -> u32 {
        match offset {
            0x00..=0x1C => self.hc[(offset >> 2) as usize & 7],
            0x20 => self.coco, // HS
            0x24..=0x40 => {
                let n = ((offset - 0x24) >> 2) as usize & 7;
                self.coco &= !(1 << n); // reading a result clears its COCO
                self.r[n]
            }
            0x44 => self.cfg,
            0x48 => self.gc,
            0x4C => 0, // GS
            0x58 => self.cal,
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: u32, value: u32) {
        match offset {
            0x00..=0x1C => {
                let n = (offset >> 2) as usize & 7;
                self.hc[n] = value;
                self.convert(n);
            }
            0x44 => self.cfg = value,
            // GC.CAL (bit 7) launches auto-calibration; `ADC_DoAutoCalibration`
            // then spins `while (GC & CAL)`. Calibration completes instantly
            // here, so drop CAL on write (self-clear) and leave GS.CALF clear
            // (fsl_adc.c ADC_DoAutoCalibration).
            0x48 => self.gc = value & !GC_CAL,
            0x58 => self.cal = value,
            _ => {}
        }
    }

    /// Start (and immediately finish) a conversion for slot `n`.
    fn convert(&mut self, n: usize) {
        let ch = (self.hc[n] & HC_ADCH) as usize;
        if ch == HC_DISABLED as usize {
            self.coco &= !(1 << n); // disabling a slot clears its COCO
            return;
        }
        self.r[n] = u32::from(self.channels[ch.min(CHANNELS - 1)]);
        self.coco |= 1 << n;
    }

    /// Any completed conversion whose slot has AIEN set.
    pub fn irq_pending(&self) -> bool {
        (0..SLOTS).any(|n| self.coco & (1 << n) != 0 && self.hc[n] & HC_AIEN != 0)
    }

    /// DMA request: `GC.DMAEN` set and a conversion complete (a DMA read of
    /// the result register clears the flag).
    pub fn dma_request(&self) -> bool {
        self.gc & 0x2 != 0 && self.coco != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_produces_channel_value() {
        let mut adc = Adc::new(1);
        adc.set_channel(5, 0x321);
        // HC0: select channel 5, no interrupt.
        adc.write(0x00, 5);
        assert_ne!(adc.read(0x20) & 1, 0, "COCO0 set");
        assert_eq!(adc.read(0x24), 0x321, "R0 = channel 5 value");
        assert_eq!(adc.read(0x20) & 1, 0, "reading R0 cleared COCO0");
    }

    #[test]
    fn disabled_channel_no_conversion() {
        let mut adc = Adc::new(1);
        adc.write(0x00, HC_DISABLED); // ADCH = 0b11111
        assert_eq!(adc.read(0x20) & 1, 0, "no COCO for a disabled slot");
    }

    #[test]
    fn interrupt_gated_by_aien() {
        let mut adc = Adc::new(1);
        adc.set_channel(2, 0x555);
        adc.write(0x00, 2); // no AIEN
        assert!(!adc.irq_pending());
        adc.write(0x04, HC_AIEN | 2); // HC1: channel 2 with AIEN
        assert!(adc.irq_pending());
    }

    #[test]
    fn auto_calibration_self_clears() {
        // ADC_DoAutoCalibration: GS = CALF (clear), GC |= CAL, then
        // `while (GC & CAL)`. CAL must self-clear and GS.CALF stay 0.
        let mut adc = Adc::new(1);
        adc.write(0x4C, 0x2); // GS: clear CALF
        adc.write(0x48, GC_CAL | 0x1); // GC: launch calibration (+ ADACKEN)
        assert_eq!(adc.read(0x48) & GC_CAL, 0, "CAL self-cleared");
        assert_ne!(adc.read(0x48) & 0x1, 0, "other GC bits stick");
        assert_eq!(adc.read(0x4C) & 0x2, 0, "calibration passed (CALF clear)");
    }

    #[test]
    fn multiple_slots_independent() {
        let mut adc = Adc::new(1);
        adc.set_channel(1, 0x111);
        adc.set_channel(9, 0x999);
        adc.write(0x00, 1); // HC0 → channel 1
        adc.write(0x1C, 9); // HC7 → channel 9
        assert_eq!(adc.read(0x24), 0x111, "R0");
        assert_eq!(adc.read(0x40), 0x999, "R7");
    }
}
