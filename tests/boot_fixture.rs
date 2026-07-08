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
