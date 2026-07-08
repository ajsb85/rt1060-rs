// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! Boot a **real, unmodified** NXP SDK Cortex-M7 firmware image and confirm
//! the emulator runs it — the M8 milestone. The fixture is an IAR-built LED
//! blinky for the i.MX RT1050 EVK (RT1050 ≈ RT1062: same core, register-
//! compatible peripherals), running from ITCM at 0x2000. Provenance:
//! `tests/fixtures/README.md`.

use rt1060_rs::cortex_m::BreakCause;
use rt1060_rs::{Rt1060, loader};

const BLINKY: &[u8] = include_bytes!("fixtures/rt1050_led_blinky_itcm.elf");

/// Fast check: the image boots, its reset/init runs, and the SDK configures
/// the EVK user LED (GPIO1_IO09) as an output — all without hitting an
/// unimplemented instruction or (verified separately) an unmapped register.
#[test]
fn rt1050_blinky_boots_and_configures_the_led_gpio() {
    let image = loader::load_elf(BLINKY).expect("parse ELF");
    let mut soc = Rt1060::boot(&image);
    soc.quiet();

    // Reset vector came from the image's table at ITCM 0x2000.
    assert_eq!(image.base, 0x0000_2000);
    assert_eq!(soc.core.regs[13], 0x2002_0000, "initial SP");
    assert_eq!(soc.core.regs[15] & !1, 0x0000_4fc0, "reset handler");

    // Run through clock init, pin-mux, and GPIO setup.
    for _ in 0..2_000_000 {
        soc.step();
        if let Some(BreakCause::Unimplemented(hw)) = soc.core.break_cause {
            panic!("hit an unimplemented instruction {hw:#06x} at boot");
        }
    }

    // The SDK's GPIO init drove GPIO1_IO09's direction to output.
    assert!(
        soc.bus.periph.gpio[0].is_output(9),
        "EVK user LED (GPIO1_IO09) configured as output during boot"
    );
}

/// Deep check: run long enough for the ~1 s delay loop to elapse and observe
/// the LED actually toggle. Ignored by default (hundreds of millions of
/// instructions); run with `cargo test --release -- --ignored`.
#[test]
#[ignore = "runs ~250M instructions; cargo test --release -- --ignored"]
fn rt1050_blinky_toggles_the_led() {
    let image = loader::load_elf(BLINKY).expect("parse ELF");
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
