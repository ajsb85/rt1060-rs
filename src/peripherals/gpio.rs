// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! GPIO — general-purpose I/O controller (MIMXRT1062.h `GPIO_Type`; RM §12).
//!
//! The SwiftIO Micro's onboard RGB LED is the first observable GPIO output
//! (active-low). Each controller drives 32 pins; the board's `SwiftIO` pin
//! ids map to (controller, pin) through IOMUXC — see `rt1060::board`.
//!
//! Register offsets: DR 0x00, GDIR 0x04, PSR 0x08, ICR1 0x0C, ICR2 0x10,
//! IMR 0x14, ISR 0x18 (W1C), EDGE_SEL 0x1C, DR_SET 0x84, DR_CLEAR 0x88,
//! DR_TOGGLE 0x8C.

pub struct Gpio {
    /// Controller number (1..9) for trace labels.
    pub index: u8,
    /// DR — data register (output latch).
    dr: u32,
    /// GDIR — direction (1 = output).
    gdir: u32,
    /// PSR — pad sample of input pins (external drive; 0 by default).
    input: u32,
    /// IMR — per-pin interrupt mask.
    imr: u32,
    /// ISR — interrupt status (W1C).
    isr: u32,
    icr1: u32,
    icr2: u32,
    edge_sel: u32,
}

impl Gpio {
    pub fn new(index: u8) -> Self {
        Self {
            index,
            dr: 0,
            gdir: 0,
            input: 0,
            imr: 0,
            isr: 0,
            icr1: 0,
            icr2: 0,
            edge_sel: 0,
        }
    }

    /// Output level currently driven on `pin` (0..31). Only meaningful when
    /// the pin is configured as an output in GDIR.
    pub fn output(&self, pin: u8) -> bool {
        (self.dr >> pin) & 1 != 0
    }

    /// Is `pin` configured as an output?
    pub fn is_output(&self, pin: u8) -> bool {
        (self.gdir >> pin) & 1 != 0
    }

    /// Host drives an external input level onto `pin`. When the pin is an
    /// input, an edge/level matching its interrupt configuration latches
    /// `ISR` (RM §12.5.6: ICR1/ICR2 select LOW/HIGH/RISING/FALLING per pin;
    /// `EDGE_SEL` overrides ICR with any-edge). Level configurations latch
    /// at injection time only — a held level does not re-assert after W1C.
    pub fn set_input(&mut self, pin: u8, level: bool) {
        let bit = 1u32 << pin;
        let prev = self.input & bit != 0;
        if level {
            self.input |= bit;
        } else {
            self.input &= !bit;
        }
        // Outputs sample DR through PSR — the injected level is inert.
        if self.gdir & bit != 0 {
            return;
        }
        let latch = if self.edge_sel & bit != 0 {
            prev != level // EDGE_SEL: any edge
        } else {
            // ICR: 2 bits per pin — 00 LOW, 01 HIGH, 10 RISING, 11 FALLING.
            let icr = if pin < 16 { self.icr1 } else { self.icr2 };
            match (icr >> ((pin & 15) * 2)) & 0b11 {
                0b00 => !level,
                0b01 => level,
                0b10 => !prev && level,
                _ => prev && !level,
            }
        };
        if latch {
            self.isr |= bit;
        }
    }

    /// PSR — reads output-latched pins as their DR value and input pins as
    /// the externally driven level.
    fn psr(&self) -> u32 {
        (self.dr & self.gdir) | (self.input & !self.gdir)
    }

    pub fn read(&mut self, offset: u32) -> u32 {
        match offset {
            0x00 => self.dr,
            0x04 => self.gdir,
            0x08 => self.psr(),
            0x0C => self.icr1,
            0x10 => self.icr2,
            0x14 => self.imr,
            0x18 => self.isr,
            0x1C => self.edge_sel,
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: u32, value: u32) {
        match offset {
            0x00 => self.dr = value,
            0x04 => self.gdir = value,
            0x0C => self.icr1 = value,
            0x10 => self.icr2 = value,
            0x14 => self.imr = value,
            0x18 => self.isr &= !value, // W1C
            0x1C => self.edge_sel = value,
            0x84 => self.dr |= value,  // DR_SET
            0x88 => self.dr &= !value, // DR_CLEAR
            0x8C => self.dr ^= value,  // DR_TOGGLE
            _ => {}
        }
    }

    /// Combined interrupt for pins 0..15 pending (ISR & IMR).
    pub fn irq_pending_low(&self) -> bool {
        (self.isr & self.imr & 0x0000_FFFF) != 0
    }

    /// Combined interrupt for pins 16..31 pending.
    pub fn irq_pending_high(&self) -> bool {
        (self.isr & self.imr & 0xFFFF_0000) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dr_set_clear_toggle() {
        let mut g = Gpio::new(1);
        g.write(0x04, 0xFFFF_FFFF); // all outputs
        g.write(0x00, 0x0000_00F0); // DR
        assert_eq!(g.read(0x00), 0x0000_00F0);
        g.write(0x84, 0x0000_0001); // DR_SET pin0
        assert!(g.output(0));
        g.write(0x88, 0x0000_0010); // DR_CLEAR pin4
        assert!(!g.output(4));
        g.write(0x8C, 0x0000_0002); // DR_TOGGLE pin1
        assert!(g.output(1));
    }

    #[test]
    fn psr_reflects_direction() {
        let mut g = Gpio::new(1);
        g.write(0x04, 0x0000_0001); // pin0 output, pin1 input
        g.write(0x00, 0x0000_0001); // drive pin0 high
        g.set_input(1, true); // external high on pin1
        let psr = g.read(0x08);
        assert_eq!(psr & 0x3, 0x3, "output latch + external input both read");
    }

    #[test]
    fn input_edges_latch_isr_per_icr() {
        let mut g = Gpio::new(1);
        // pin2 RISING (ICR1 cfg 0b10), pin3 FALLING (0b11).
        g.write(0x0C, (0b10 << 4) | (0b11 << 6));
        g.set_input(2, true); // rising
        g.set_input(3, true); // rising — not the configured edge
        assert_eq!(g.read(0x18), 1 << 2, "only the rising pin latched");
        g.set_input(3, false); // falling
        assert_eq!(g.read(0x18), (1 << 2) | (1 << 3));
        g.write(0x18, 1 << 2); // W1C
        assert_eq!(g.read(0x18), 1 << 3);
        g.set_input(2, false); // falling on a rising-configured pin: no latch
        assert_eq!(g.read(0x18), 1 << 3);
    }

    #[test]
    fn input_levels_latch_isr_per_icr() {
        let mut g = Gpio::new(1);
        // pin16 HIGH level (ICR2 cfg 0b01); pin17 LOW level (0b00, reset).
        g.write(0x10, 0b01);
        g.set_input(16, true);
        g.set_input(17, false);
        assert_eq!(g.read(0x18), (1 << 16) | (1 << 17));
    }

    #[test]
    fn edge_sel_latches_any_edge() {
        let mut g = Gpio::new(1);
        g.write(0x1C, 1 << 5); // EDGE_SEL pin5 (ICR cfg 0b00 would be LOW)
        g.set_input(5, true);
        assert_eq!(g.read(0x18), 1 << 5);
        g.write(0x18, 1 << 5); // W1C
        g.set_input(5, false);
        assert_eq!(g.read(0x18), 1 << 5, "both edges latch");
    }

    #[test]
    fn outputs_do_not_latch_isr() {
        let mut g = Gpio::new(1);
        g.write(0x04, 1 << 4); // pin4 output
        g.write(0x1C, 1 << 4); // EDGE_SEL — would latch any edge on an input
        g.set_input(4, true);
        assert_eq!(g.read(0x18), 0, "injected level is inert on an output");
    }

    #[test]
    fn interrupt_status_masked() {
        let mut g = Gpio::new(1);
        g.isr = 0x0001_0004; // pin2 (low) + pin16 (high)
        g.write(0x14, 0x0000_0004); // IMR: only pin2
        assert!(g.irq_pending_low());
        assert!(!g.irq_pending_high());
        g.write(0x18, 0x0000_0004); // W1C pin2
        assert!(!g.irq_pending_low());
    }
}
