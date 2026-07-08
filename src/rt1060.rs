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
    /// Download-status LED. Its pad is not published in any workspace source
    /// (only in the compiled HAL archive), so it is intentionally unmapped.
    pub const LED_DL: u8 = 47;
    /// Total user-exposed GPIO count.
    pub const IO_PINS: u8 = 44;

    /// The RGB LED is wired to **GPIO1** pins 9/10/11 via pads
    /// `GPIO_AD_B0_09/10/11` at ALT5 (mux offsets 0x0E0/0x0E4/0x0E8), and is
    /// **active-low** — a pin driven low turns its LED on. Sources: Zephyr
    /// `boards/arm/mm_swiftio/{mm_swiftio.dts,pinmux.c}` and the SwiftIO
    /// `01LED/RGBLED` example (`DigitalOut(value: true)` = off).
    ///
    /// The full SwiftIO id 0..43 → (GPIO, pin) table lives only inside the
    /// prebuilt HAL archive (`lib..__HalSwiftIO__driver__zephyr.a`) and is
    /// deliberately not reconstructed by guesswork — it is a ROADMAP task.
    pub const RGB_GPIO: u8 = 1; // GPIO1
    pub const RGB_RED_PIN: u8 = 9;
    pub const RGB_GREEN_PIN: u8 = 10;
    pub const RGB_BLUE_PIN: u8 = 11;
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

    /// Silence unmapped/unknown-peripheral logging and any `RT1060_TRACE`
    /// output (tests, benchmarks).
    pub fn quiet(&mut self) {
        self.bus.log_unmapped = false;
        self.bus.periph.log_unknown = false;
        self.bus.trace_writes = false;
        self.bus.trace_reads = false;
    }

    /// Execute one instruction (plus any exception it triggers) and advance
    /// the time-driven peripherals by the cycles it retired.
    pub fn step(&mut self) {
        let before = self.core.cycles;
        self.core.step(&mut self.bus);
        let elapsed = self.core.cycles.wrapping_sub(before);
        self.cycles = self.cycles.wrapping_add(elapsed);
        self.bus.periph.tick(elapsed);
        // Service DMA channels driven by peripheral hardware requests
        // (cheap-gated: a no-op unless some channel has ERQ enabled).
        self.bus.edma_service_hw();

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

    /// Current clock roots (Hz) derived from the CCM/CCM_ANALOG register
    /// state firmware programmed. After `BOARD_BootClockRUN` the SwiftIO
    /// Micro reports `core = 600 MHz`.
    pub fn clocks(&self) -> crate::peripherals::clocks::Clocks {
        self.bus.periph.clocks()
    }

    /// The core (CPU/AHB) clock in Hz.
    pub fn core_hz(&self) -> u64 {
        self.bus.periph.clock_hz().0
    }

    /// The PERCLK (GPT/PIT) clock in Hz.
    pub fn perclk_hz(&self) -> u64 {
        self.bus.periph.clock_hz().1
    }

    /// Configured duty cycle (0.0..=1.0) of a FlexPWM output, or `None` when
    /// the output is disabled / unconfigured. `instance` is 1..4, `submodule`
    /// 0..3.
    pub fn pwm_duty(
        &self,
        instance: u8,
        submodule: usize,
        chan: crate::peripherals::pwm::Chan,
    ) -> Option<f64> {
        let i = instance.checked_sub(1)? as usize;
        self.bus.periph.pwm.get(i)?.duty(submodule, chan)
    }

    /// The onboard RGB LED **on** states `(red, green, blue)`. The LED is
    /// active-low, so a channel is on when its GPIO1 pin is driven low (and
    /// configured as an output). See [`board`] for the wiring/sources.
    pub fn led_rgb(&self) -> (bool, bool, bool) {
        let g = &self.bus.periph.gpio[(board::RGB_GPIO - 1) as usize];
        let on = |pin: u8| g.is_output(pin) && !g.output(pin);
        (
            on(board::RGB_RED_PIN),
            on(board::RGB_GREEN_PIN),
            on(board::RGB_BLUE_PIN),
        )
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
