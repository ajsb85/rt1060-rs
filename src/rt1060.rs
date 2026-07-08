// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! The i.MX RT1060 SoC — a Cortex-M7 core plus the system bus.
//!
//! Mirrors the rp2350-rs `Rp2350` / mg24-rs `Mg24` top level: plain data,
//! split borrows (`core.step(&mut bus)`), no `Rc<RefCell>`. The default board
//! is the MadMachine **SwiftIO Micro** (see [`board`]).

use crate::cortex_m::{BreakCause, CortexM7};
use crate::loader::LoadedImage;
use crate::memory::SystemBus;

/// MadMachine SwiftIO Micro board constants.
///
/// SwiftIO Micro: NXP RT1062 Cortex-M7 @ 600 MHz, 32 MB SDRAM, 16 MB FlexSPI
/// NOR, onboard RGB LED (active-low), 44 IO pins. The RGB LED and pin ids
/// below are the SwiftIO logical ids (MadBoards `SwiftIOMicro`); the id →
/// (GPIO controller, pin) pad map is compiled into the Zephyr board archive
/// and is extracted in the ROADMAP's board-bring-up milestone.
pub mod board {
    /// Core clock the SwiftIO Micro configures (600 MHz ARM PLL).
    pub const CORE_CLOCK_HZ: u64 = 600_000_000;
    /// SDRAM size (32 MiB).
    pub const SDRAM_BYTES: usize = 32 * 1024 * 1024;
    /// FlexSPI NOR flash size (16 MiB).
    pub const FLASH_BYTES: usize = 16 * 1024 * 1024;

    /// SwiftIO logical pin id for the onboard RGB LED (active-low).
    pub const LED_RED: u8 = 44;
    pub const LED_GREEN: u8 = 45;
    pub const LED_BLUE: u8 = 46;
    /// Download-status LED.
    pub const LED_DL: u8 = 47;
    /// Total user-exposed GPIO count.
    pub const IO_PINS: u8 = 44;
}

pub struct Rt1060 {
    pub core: CortexM7,
    pub bus: SystemBus,
    /// Retired-cycle accumulator, for wall-clock ↔ core-cycle conversions.
    pub cycles: u64,
}

impl Default for Rt1060 {
    fn default() -> Self {
        Self::new()
    }
}

impl Rt1060 {
    /// A powered-on SoC with erased flash and zeroed RAM. Load an image with
    /// [`Rt1060::load_image`] (or use [`Rt1060::boot`]) before stepping.
    pub fn new() -> Self {
        Self {
            core: CortexM7::new(),
            bus: SystemBus::new(),
            cycles: 0,
        }
    }

    /// Load a parsed image and reset the core so SP/PC come from the image's
    /// vector table (placed at `image.base` — SDRAM `0x8000_0000` for a
    /// MadMachine build). Peripheral state is left at power-on defaults.
    pub fn load_image(&mut self, image: &LoadedImage) {
        self.bus.load_segments(image);
        self.core.vtor = image.base;
        self.core.reset(&mut self.bus);
    }

    /// Build and boot a SoC from a parsed image in one step.
    pub fn boot(image: &LoadedImage) -> Self {
        let mut soc = Self::new();
        soc.load_image(image);
        soc
    }

    /// Silence unmapped/unknown-peripheral logging (tests, benchmarks).
    pub fn quiet(&mut self) {
        self.bus.log_unmapped = false;
        self.bus.periph.log_unknown = false;
    }

    /// Execute one instruction (plus any exception it triggers) and advance
    /// the time-driven peripherals by the cycles it retired.
    pub fn step(&mut self) {
        let before = self.core.cycles;
        self.core.step(&mut self.bus);
        let elapsed = self.core.cycles.wrapping_sub(before);
        self.cycles = self.cycles.wrapping_add(elapsed);
        self.bus.periph.tick(elapsed);

        // Software-requested resets (SRC.SCR, SCB AIRCR.SYSRESETREQ).
        if self.core.sysreset_request || self.bus.periph.src.reset_requested {
            self.chip_reset();
        }
    }

    /// Run until the core halts (BKPT/UDF/unimplemented) or `max_steps` is
    /// reached. Returns the halt cause, if any.
    pub fn run(&mut self, max_steps: u64) -> Option<BreakCause> {
        for _ in 0..max_steps {
            self.step();
            if let Some(cause) = self.core.break_cause {
                return Some(cause);
            }
        }
        None
    }

    /// Warm reset: re-fetch SP/PC from the current VTOR, latch the reset
    /// cause in SRC.SRSR, and leave RAM/flash intact (like a real M7 reset).
    pub fn chip_reset(&mut self) {
        use crate::peripherals::src::SRSR_POR;
        let vtor = self.core.vtor;
        self.core = CortexM7::new();
        self.core.vtor = vtor;
        self.bus.periph.src.reset_requested = false;
        self.bus.periph.src.set_reset_cause(SRSR_POR);
        self.core.reset(&mut self.bus);
    }

    /// Convenience: drain the LPUART1 (console) output as a string.
    pub fn console_string(&mut self) -> String {
        self.bus.periph.lpuart[0].take_output_string()
    }

    /// Feed bytes into the LPUART1 (console) RX FIFO, as if typed on the
    /// USB-serial bridge. Interactive firmware (a Swift REPL prompt, a
    /// Zephyr shell) reads them back through DATA / raises the RX interrupt.
    pub fn push_console_input(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.bus.periph.lpuart[0].rx_push(b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cortex_m::asm::*;
    use crate::loader;
    use crate::memory::map;

    /// Assemble a tiny SDRAM image: a vector table (SP, reset) + a short
    /// Thumb program that stores a marker word into DTCM, then spins.
    fn boot_image(code: &[u16]) -> loader::LoadedImage {
        let code_addr = map::SDRAM_BASE + 0x100;
        let mut data = Vec::new();
        // Vector table: [0] = initial SP (top of DTCM), [1] = reset (thumb).
        data.extend_from_slice(&(map::DTCM_BASE + 0x1000).to_le_bytes());
        data.extend_from_slice(&(code_addr | 1).to_le_bytes());
        data.resize(0x100, 0);
        for hw in code {
            data.extend_from_slice(&hw.to_le_bytes());
        }
        loader::load_bin(map::SDRAM_BASE, &data)
    }

    #[test]
    fn boots_from_vector_table_and_executes() {
        // r1 = 0x20 << 24 = DTCM_BASE; r0 = 0x42; str r0,[r1]; b .
        let code = [
            movs_imm8(1, 0x20),
            lsls_imm(1, 1, 24),
            movs_imm8(0, 0x42),
            str_imm(0, 1, 0),
            b(-4), // spin on self
        ];
        let img = boot_image(&code);
        let mut soc = Rt1060::boot(&img);
        soc.quiet();
        // SP/PC came from the vector table.
        assert_eq!(soc.core.regs[13], map::DTCM_BASE + 0x1000);
        assert_eq!(soc.core.regs[15] & !1, map::SDRAM_BASE + 0x100);
        soc.run(20);
        // The store reached DTCM.
        assert_eq!(soc.bus.read32(map::DTCM_BASE), 0x42);
        // And the core is spinning at the branch.
        assert!(soc.core.regs[15] >= map::SDRAM_BASE + 0x100);
    }

    #[test]
    fn console_input_reaches_lpuart1_rx() {
        let img = boot_image(&[b(-2)]); // reset handler just spins
        let mut soc = Rt1060::boot(&img);
        soc.quiet();
        soc.push_console_input(b"hi");
        // LPUART1 STAT.RDRF (bit 21) is set and DATA pops the bytes in order.
        assert_ne!(soc.bus.read32(0x4018_4014) & (1 << 21), 0);
        assert_eq!(soc.bus.read32(0x4018_401C), u32::from(b'h'));
        assert_eq!(soc.bus.read32(0x4018_401C), u32::from(b'i'));
    }

    #[test]
    fn board_constants_match_swiftio_micro() {
        assert_eq!(board::CORE_CLOCK_HZ, 600_000_000);
        assert_eq!(board::FLASH_BYTES, crate::memory::map::FLASH_SIZE);
        assert_eq!(board::SDRAM_BYTES, crate::memory::map::SDRAM_SIZE);
    }
}
