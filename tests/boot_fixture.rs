// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! Boot **real, unmodified** NXP SDK Cortex-M7 firmware and confirm the
//! emulator runs it from every code region — the M8 milestone. The fixtures
//! are IAR-built LED blinkies for the i.MX RT1050 EVK (RT1050 ≈ RT1062: same
//! core, register-compatible peripherals) linked for three different memories:
//! ITCM (`0x2000`), FlexSPI NOR XIP (`0x6000_2000`), and SDRAM (`0x8000_2000`,
//! the MadMachine run location). Provenance: `tests/fixtures/README.md`.

use rt1060_rs::cortex_m::BreakCause;
use rt1060_rs::{Rt1060, loader};

const ITCM: &[u8] = include_bytes!("fixtures/rt1050_led_blinky_itcm.elf");
const FLEXSPI: &[u8] = include_bytes!("fixtures/rt1050_led_blinky_flexspi.elf");
const SDRAM: &[u8] = include_bytes!("fixtures/rt1050_led_blinky_sdram.elf");

/// Boot `elf`, run its init, and assert it reached GPIO setup without hitting
/// an unimplemented instruction — from whichever memory it is linked for.
fn boots_and_configures_led(elf: &[u8], expected_base: u32) {
    let image = loader::load_elf(elf).expect("parse ELF");
    let mut soc = Rt1060::boot(&image);
    soc.quiet();
    assert_eq!(image.base, expected_base, "linked load base");
    assert_eq!(soc.core.regs[13], 0x2002_0000, "initial SP");

    for _ in 0..2_000_000 {
        soc.step();
        if let Some(BreakCause::Unimplemented(hw)) = soc.core.break_cause {
            panic!("unimplemented instruction {hw:#06x} booting from {expected_base:#010x}");
        }
    }
    assert!(
        soc.bus.periph.gpio[0].is_output(9),
        "EVK user LED (GPIO1_IO09) configured as output from base {expected_base:#010x}"
    );
}

#[test]
fn boots_from_itcm() {
    boots_and_configures_led(ITCM, 0x0000_2000);
}

#[test]
fn boots_from_flexspi_nor_xip() {
    boots_and_configures_led(FLEXSPI, 0x6000_2000);
}

#[test]
fn boots_from_sdram() {
    boots_and_configures_led(SDRAM, 0x8000_2000);
}

/// Boot a **real embedded-Swift** program (compiled with Swift 6.2 for
/// `armv7em-none-none-eabi`, linked bare-metal for the RT1062) that blinks the
/// SwiftIO RGB LED, and observe it toggle the RED + BLUE pins **by their
/// SwiftIO logical ids** — end-to-end proof that Swift runs on the emulated
/// chip and drives the pins recovered from the HalSwiftIO binary. Build
/// sources: `tests/fixtures/swiftio_blink_src/`.
#[test]
fn embedded_swift_blinks_the_swiftio_rgb_led() {
    const SWIFT_BLINK: &[u8] = include_bytes!("fixtures/swiftio_blink_embedded_swift.elf");
    let image = loader::load_elf(SWIFT_BLINK).expect("parse ELF");
    let mut soc = Rt1060::boot(&image);
    soc.quiet();
    assert_eq!(
        image.base, 0x8000_0000,
        "runs from SDRAM (MadMachine location)"
    );
    assert_eq!(soc.core.regs[13], 0x2002_0000, "initial SP = top of DTCM");

    // RED = id 44 (GPIO1 pin 9), BLUE = id 46 (GPIO1 pin 11) — the Swift code
    // drives both together. Count RED transitions via the swiftio_pin id API.
    let mut transitions = 0;
    let mut last = soc.swiftio_pin(44);
    for _ in 0..300_000 {
        soc.step();
        let red = soc.swiftio_pin(44);
        assert_eq!(
            red,
            soc.swiftio_pin(46),
            "Swift drives RED and BLUE together"
        );
        if red != last {
            transitions += 1;
            last = red;
        }
    }
    assert!(
        transitions >= 4,
        "the Swift blink toggled the RGB LED (saw {transitions})"
    );
    // GREEN (id 45, GPIO1 pin 10) is never configured/driven by this program.
    assert_eq!(soc.swiftio_pin(45), Some(false), "GREEN untouched");
}

/// The **real, unmodified MadMachine SwiftIO Blink** — built with the
/// MadMachine SDK 2.2.0 (`mm build`) as the full SwiftIO + Zephyr + embedded-
/// Swift image, running from SDRAM. This fast check boots it through the whole
/// Zephyr kernel + device init (clocks, ADC calibration, GPT, LPUART, LPI2C,
/// LPSPI, FlexSPI/littlefs) and asserts it reached Zephyr's console logging —
/// proof the real firmware runs. Provenance: `tests/fixtures/README.md`.
#[test]
fn madmachine_swiftio_blink_boots_zephyr() {
    const BLINK: &[u8] = include_bytes!("fixtures/madmachine_swiftio_blink.elf");
    let image = loader::load_elf(BLINK).expect("parse ELF");
    let mut soc = Rt1060::boot(&image);
    soc.quiet();
    assert_eq!(image.base, 0x8000_0000, "MadMachine image runs from SDRAM");

    for _ in 0..12_000_000 {
        soc.step();
        if let Some(BreakCause::Unimplemented(hw)) = soc.core.break_cause {
            panic!("unimplemented instruction {hw:#06x} in MadMachine boot");
        }
    }
    // Zephyr's console (LPUART1) logged the littlefs bring-up — the kernel and
    // device init are running.
    let console = soc.console_string();
    assert!(
        console.contains("LittleFS"),
        "expected Zephyr console output, got: {console:?}"
    );
    // The FlexSPI IP flash engine is correct end-to-end: littlefs formats and
    // mounts on the emulated NOR (an earlier bug wrote the superblock to the
    // wrong address, so the mount failed "Superblock unwritable").
    assert!(
        console.contains("/lfs mounted"),
        "littlefs should format and mount on the emulated FlexSPI NOR"
    );
}

/// Deep check: run the real MadMachine Blink long enough for the `sleep(ms:500)`
/// loop to toggle the RGB LED — RED (id 44, GPIO1 pin 9) and BLUE (id 46, pin
/// 11). Ignored by default (~1 billion instructions through the full Zephyr
/// stack); run with `cargo test --release -- --ignored`.
#[test]
#[ignore = "runs ~1B instructions through Zephyr; cargo test --release -- --ignored"]
fn madmachine_swiftio_blink_toggles_the_led() {
    const BLINK: &[u8] = include_bytes!("fixtures/madmachine_swiftio_blink.elf");
    let image = loader::load_elf(BLINK).expect("parse ELF");
    let mut soc = Rt1060::boot(&image);
    soc.quiet();

    let mut transitions = 0;
    let mut last = soc.swiftio_pin(44); // RED
    for _ in 0..1_100_000_000u64 {
        soc.step();
        let red = soc.swiftio_pin(44);
        if red != last {
            transitions += 1;
            last = red;
            if transitions >= 3 {
                break;
            }
        }
    }
    assert!(
        transitions >= 3,
        "the MadMachine Blink should toggle the RGB LED (saw {transitions})"
    );
}

/// Deep check: run long enough for the delay loop to elapse and observe the
/// LED actually toggle. Ignored by default (hundreds of millions of
/// instructions); run with `cargo test --release -- --ignored`.
#[test]
#[ignore = "runs ~250M instructions; cargo test --release -- --ignored"]
fn sdram_blinky_toggles_the_led() {
    let image = loader::load_elf(SDRAM).expect("parse ELF");
    let mut soc = Rt1060::boot(&image);
    soc.quiet();

    let led = |soc: &Rt1060| soc.bus.periph.gpio[0].output(9);
    let mut transitions = 0;
    let mut last = led(&soc);
    for _ in 0..250_000_000u64 {
        soc.step();
        let now = led(&soc);
        if now != last {
            transitions += 1;
            last = now;
            if transitions >= 2 {
                break;
            }
        }
    }
    assert!(
        transitions >= 2,
        "the LED should have blinked at least twice (saw {transitions})"
    );
}
