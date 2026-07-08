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
/// SwiftIO Micro: NXP RT1062 Cortex-M7 @ 600 MHz, 32 MB SDRAM (Micron
/// MT48LC16M16A2), 16 MB FlexSPI NOR (ISSI IS25WP128, JEDEC `9d 70 17`),
/// onboard RGB LED (active-low), 44 IO pins.
///
/// **SwiftIO runs on Zephyr**, so the authoritative hardware pin assignments
/// are MadMachine's Zephyr board `mm_feather` (the Micro's internal name):
/// `../zephyr/boards/arm/mm_feather/{mm_feather.dts,pinmux.c}`. The concrete
/// pads below are transcribed from there (see [`board::pinmux`]). The SwiftIO
/// *logical* id (`D0`..`D43`, MadBoards `SwiftIOMicro`) → pad ordering is
/// resolved inside the prebuilt HalSwiftIO `swifthal_gpio_open(id)` driver
/// (`.a`) plus the `SwiftIOPinout` image — that thin translation layer is not
/// in machine-readable source form; do not guess it (ROADMAP).
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
    /// `GPIO_AD_B0_09/10/11` at ALT5, and is **active-low** — a pin driven low
    /// turns its LED on. Source: `mm_feather/{mm_feather.dts,pinmux.c}`
    /// (`red_led = &gpio1 9`, `green = 10`, `blue = 11`) + the SwiftIO
    /// `01LED/RGBLED` example (`DigitalOut(value: true)` = off).
    pub const RGB_GPIO: u8 = 1; // GPIO1
    pub const RGB_RED_PIN: u8 = 9;
    pub const RGB_GREEN_PIN: u8 = 10;
    pub const RGB_BLUE_PIN: u8 = 11;

    /// Concrete pad / GPIO assignments transcribed from the SwiftIO Micro
    /// Zephyr board `mm_feather` (`pinmux.c` + `mm_feather.dts`). These are the
    /// hardware-truth pins; the SwiftIO logical-id ordering that maps onto them
    /// lives in the prebuilt HalSwiftIO driver (see the module docs).
    pub mod pinmux {
        /// LPUART1 is the console (`zephyr,console`): TX = `GPIO_AD_B0_12`,
        /// RX = `GPIO_AD_B0_13`.
        pub const CONSOLE_LPUART: u8 = 1;
        /// LPI2C1: SCL = `GPIO_AD_B1_00`, SDA = `GPIO_AD_B1_01`.
        /// LPI2C3: SCL = `GPIO_AD_B1_07`, SDA = `GPIO_AD_B1_06`.
        pub const I2C_INSTANCES: [u8; 2] = [1, 3];
        /// SPI buses wired out: LPSPI3 and LPSPI4.
        pub const SPI_INSTANCES: [u8; 2] = [3, 4];
        /// SD card on **USDHC1**; card-detect on `GPIO2` pin 28
        /// (`GPIO_B1_12`, active-low), card power on `GPIO1` pin 5
        /// (`GPIO_AD_B0_05`).
        pub const SD_USDHC: u8 = 1;
        pub const SD_CARD_DETECT_GPIO: u8 = 2;
        pub const SD_CARD_DETECT_PIN: u8 = 28;
        pub const SD_POWER_GPIO: u8 = 1;
        pub const SD_POWER_PIN: u8 = 5;
        /// I²S is `SAI1` (`i2s_rxtx = &sai1`).
        pub const I2S_SAI: u8 = 1;
        /// FlexPWM submodules enabled: PWM1_3, PWM2_0..3, PWM4_0..3 (14 PWM).
        /// GPT1/GPT2, ADC1/ADC2, and USB1 are all enabled.
        pub const USB_OTG: u8 = 1;
    }
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

    /// Insert an SD card into a USDHC port (1 or 2) — the SwiftIO Micro reads
    /// its user image and assets from the card on USDHC1.
    pub fn insert_sd_card(&mut self, port: u8, card: crate::peripherals::usdhc::SdCard) {
        if let Some(u) = self
            .bus
            .periph
            .usdhc
            .get_mut(port.saturating_sub(1) as usize)
        {
            u.insert(card);
        }
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
