// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! FlexPWM — flexible PWM (MIMXRT1062.h `PWM_Type`; RM §54). PWM1..4, each
//! with four submodules (SM0..3) of two channels (A/B). The SwiftIO Micro
//! exposes 14 PWM outputs across these.
//!
//! Like a physical PWM, the useful observable is the *configured waveform*,
//! not an instantaneous pin level: this model stores the submodule registers
//! (16-bit, packed two-per-word — so the bus routes width-accurate access
//! here, as it does for the eDMA TCD) and computes each channel's period and
//! duty from INIT/VAL1..VAL5:
//!
//! - period = `VAL1 - INIT`  (the counter runs INIT..VAL1 then reloads)
//! - channel A high span = `VAL3 - VAL2`; channel B = `VAL5 - VAL4`
//!
//! An output is "live" when its `OUTEN` bit is set (module reg 0x180:
//! PWMA_EN = bits [11:8], PWMB_EN = [7:4], indexed by submodule).
//!
//! `MCTRL.LDOK` buffered loads are transparent here (VAL writes take effect
//! immediately); fault handling, capture, and fractional (FRACVAL) delay are
//! ROADMAP items.

/// Submodule stride and value-register offsets (RM §54; MIMXRT1062.h).
const SM_STEP: u32 = 0x60;
const R_INIT: u32 = 0x02;
const R_VAL1: u32 = 0x0E;
const R_VAL2: u32 = 0x12;
const R_VAL3: u32 = 0x16;
const R_VAL4: u32 = 0x1A;
const R_VAL5: u32 = 0x1E;
const R_OUTEN: u32 = 0x180;

/// Which channel of a submodule.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Chan {
    A,
    B,
}

pub struct Pwm {
    pub index: u8,
    mem: Box<[u8; 0x4000]>,
}

impl Pwm {
    pub fn new(index: u8) -> Self {
        Self {
            index,
            mem: Box::new([0; 0x4000]),
        }
    }

    #[inline]
    fn rd16(&self, off: u32) -> u16 {
        let o = off as usize;
        u16::from_le_bytes([self.mem[o], self.mem[o + 1]])
    }

    // --- width-accurate register access (packed 16-bit fields) -------------

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
        let o = off as usize;
        self.mem[o..o + 2].copy_from_slice(&value.to_le_bytes());
    }

    pub fn write8(&mut self, off: u32, value: u8) {
        self.mem[off as usize] = value;
    }

    pub fn write32(&mut self, off: u32, value: u32) {
        self.write16(off, value as u16);
        self.write16(off + 2, (value >> 16) as u16);
    }

    // --- observable waveform -----------------------------------------------

    #[inline]
    fn sm(&self, sm: usize, reg: u32) -> i32 {
        i32::from(self.rd16(sm as u32 * SM_STEP + reg) as i16)
    }

    /// Is this submodule/channel output enabled (`OUTEN`)?
    pub fn output_enabled(&self, sm: usize, chan: Chan) -> bool {
        let outen = self.rd16(R_OUTEN);
        let bit = match chan {
            Chan::A => 8 + sm, // PWMA_EN [11:8]
            Chan::B => 4 + sm, // PWMB_EN [7:4]
        };
        outen & (1 << bit) != 0
    }

    /// `(period, high)` counts for a submodule channel, or `None` if the
    /// period is non-positive.
    pub fn waveform(&self, sm: usize, chan: Chan) -> Option<(i32, i32)> {
        if sm >= 4 {
            return None;
        }
        let period = self.sm(sm, R_VAL1) - self.sm(sm, R_INIT);
        if period <= 0 {
            return None;
        }
        let high = match chan {
            Chan::A => self.sm(sm, R_VAL3) - self.sm(sm, R_VAL2),
            Chan::B => self.sm(sm, R_VAL5) - self.sm(sm, R_VAL4),
        };
        Some((period, high.clamp(0, period)))
    }

    /// Duty cycle (0.0..=1.0) of a submodule channel, or `None` when the
    /// output is disabled or the period is invalid.
    pub fn duty(&self, sm: usize, chan: Chan) -> Option<f64> {
        if !self.output_enabled(sm, chan) {
            return None;
        }
        let (period, high) = self.waveform(sm, chan)?;
        Some(f64::from(high) / f64::from(period))
    }
}

impl Default for Pwm {
    fn default() -> Self {
        Self::new(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Configure submodule `sm` for a centred pulse of `high`/`period` counts.
    fn program(pwm: &mut Pwm, sm: usize, period: u16, high: u16) {
        let b = sm as u32 * SM_STEP;
        pwm.write16(b + R_INIT, 0);
        pwm.write16(b + R_VAL1, period);
        pwm.write16(b + R_VAL2, 0);
        pwm.write16(b + R_VAL3, high);
    }

    #[test]
    fn duty_from_val_registers() {
        let mut pwm = Pwm::new(1);
        program(&mut pwm, 0, 1000, 250);
        // Not live until OUTEN.PWMA0 is set.
        assert_eq!(pwm.duty(0, Chan::A), None);
        pwm.write16(R_OUTEN, 1 << 8); // PWMA_EN submodule 0
        assert_eq!(pwm.duty(0, Chan::A), Some(0.25));
    }

    #[test]
    fn packed_16bit_fields_isolated() {
        let mut pwm = Pwm::new(1);
        // INIT (0x02) and CNT (0x00) share a word; VAL0 (0x0A)/FRACVAL near
        // VAL1 (0x0E). Writing one must not disturb the other.
        pwm.write16(R_INIT, 0xBEEF);
        pwm.write16(0x00, 0x1234); // CNT
        assert_eq!(pwm.read16(R_INIT), 0xBEEF);
        assert_eq!(pwm.read16(0x00), 0x1234);
    }

    #[test]
    fn channel_b_and_multiple_submodules() {
        let mut pwm = Pwm::new(2);
        let b = SM_STEP; // submodule 1
        pwm.write16(b + R_INIT, 0);
        pwm.write16(b + R_VAL1, 800);
        pwm.write16(b + R_VAL4, 100);
        pwm.write16(b + R_VAL5, 500); // B high = 400 → 0.5
        pwm.write16(R_OUTEN, 1 << (4 + 1)); // PWMB_EN submodule 1
        assert_eq!(pwm.duty(1, Chan::B), Some(0.5));
        assert_eq!(pwm.duty(1, Chan::A), None, "A not enabled");
    }
}
