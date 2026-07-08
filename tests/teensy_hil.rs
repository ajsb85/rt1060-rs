// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! HIL parity against a **physical Teensy 4.1** (PJRC, same NXP MIMXRT1062
//! silicon as the SwiftIO Micro). The fixture is PJRC's own unmodified
//! `blink_fast_Teensy41` (`tests/fixtures/teensy41_blink_fast.{elf,hex}`); the
//! `.hex` is flashed to the real board with `teensy_loader_cli --mcu=TEENSY41`
//! and the `.elf` is booted here through the i.MX RT Boot-ROM → IVT path. Both
//! run the same Arduino sketch: `pinMode(13, OUTPUT)` then toggle pin 13 with
//! `delay(100)`. Provenance + cross-check notes: `tests/fixtures/README.md`.
//!
//! Pin 13 on a Teensy 4.x is pad `GPIO_B0_03`, which the Teensyduino core
//! remaps to the high-speed **GPIO7** bit 3 (`IOMUXC_GPR_GPR27 = 0xFFFFFFFF`),
//! so the observable LED is `gpio[6]` (GPIO7) pin 3.

use rt1060_rs::cortex_m::BreakCause;
use rt1060_rs::{Rt1060, loader};

const BLINK: &[u8] = include_bytes!("fixtures/teensy41_blink_fast.elf");

/// Teensy 4.x LED: pin 13 = `GPIO_B0_03` → fast GPIO7, bit 3.
const LED_GPIO: usize = 6; // GPIO7
const LED_PIN: u8 = 3;

fn led_high(soc: &Rt1060) -> bool {
    soc.bus.periph.gpio[LED_GPIO].is_output(LED_PIN)
        && soc.bus.periph.gpio[LED_GPIO].output(LED_PIN)
}

/// The Teensyduino `ResetHandler` boots via the i.MX RT Boot ROM (FlexSPI
/// config + IVT), reconfigures FlexRAM, copies `.text.itcm` into ITCM, runs the
/// clock tree up to F_CPU, then reaches the Arduino runtime — all with **zero
/// unimplemented instructions**. This fast check stops before the 300 ms USB
/// startup delay and asserts the boot model + clock parity.
#[test]
fn teensy41_blink_boots_the_arduino_core() {
    let image = loader::load_elf(BLINK).expect("parse ELF");
    // e_entry points at the IVT at the FlexSPI-NOR base + 0x1000.
    assert_eq!(image.entry, Some(0x6000_1000), "ELF e_entry = IVT");
    let mut soc = Rt1060::cold_boot_from_ivt(&image).expect("i.MX RT IVT boot");
    soc.quiet();
    // The ROM handed off to the naked ResetHandler in flash.
    assert_eq!(soc.core.regs[15] & !1, 0x6000_1030, "PC = IVT.entry");

    let mut min_pc = u32::MAX;
    for _ in 0..5_000_000 {
        soc.step();
        if let Some(BreakCause::Unimplemented(hw)) = soc.core.break_cause {
            panic!(
                "unimplemented instruction {hw:#06x} at PC {:#010x}",
                soc.core.regs[15]
            );
        }
        min_pc = min_pc.min(soc.core.regs[15]);
    }
    // `ResetHandler` copied `.text.itcm` into ITCM and is now executing the
    // Arduino runtime there (millis()/delay live in ITCM fast code).
    assert!(
        min_pc < 0x2000,
        "executed copied code from ITCM (min PC {min_pc:#010x})"
    );
    // `set_arm_clock(F_CPU)` ran: this build targets 396 MHz (PLL_ARM
    // DIV_SELECT = 66 → VCO 792 MHz ÷ 2 ÷ ARM_PODF 2). Resolving this from the
    // programmed CCM/CCM_ANALOG registers requires the CBCMR reset default
    // (PRE_PERIPH_CLK_SEL = PLL_ARM), which the firmware never rewrites.
    assert_eq!(
        soc.core_hz(),
        396_000_000,
        "F_CPU parity with the programmed ARM PLL"
    );
}

/// **HIL timing parity.** The same `.hex` flashed to the physical Teensy 4.1
/// blinks pin 13 at 5 Hz (`delay(100)`), timed by SysTick off its 100 kHz
/// external reference clock so it is independent of the ARM clock. The emulator
/// must reproduce the exact wall-clock cadence: a first toggle at ~300 ms (the
/// Teensyduino `TEENSY_INIT_USB_DELAY` = 20 + 280 ms before `main()`), then a
/// toggle every 100 ms.
#[test]
#[ignore = "runs ~300M instructions to observe the LED cadence; cargo test --release -- --ignored"]
fn teensy41_blink_matches_the_hardware_led_cadence() {
    let image = loader::load_elf(BLINK).expect("parse ELF");
    let mut soc = Rt1060::cold_boot_from_ivt(&image).expect("i.MX RT IVT boot");
    soc.quiet();
    let hz = 396_000_000f64; // core clock this build programs (see fast test)

    let mut toggles: Vec<f64> = Vec::new(); // wall-clock ms of each transition
    let mut last = led_high(&soc);
    for _ in 0..320_000_000u64 {
        soc.step();
        let now = led_high(&soc);
        if now != last {
            toggles.push(soc.cycles as f64 / hz * 1000.0);
            last = now;
            if toggles.len() >= 6 {
                break;
            }
        }
    }
    assert!(
        toggles.len() >= 6,
        "expected ≥6 LED transitions, saw {}",
        toggles.len()
    );
    // First transition (pin driven HIGH) at the 300 ms USB startup delay.
    assert!(
        (toggles[0] - 300.0).abs() < 5.0,
        "first toggle at the ~300 ms startup delay, saw {:.2} ms",
        toggles[0]
    );
    // Every subsequent interval is delay(100) = 100 ms, within 1 %.
    for w in toggles.windows(2).skip(1) {
        let dt = w[1] - w[0];
        assert!(
            (dt - 100.0).abs() < 1.0,
            "toggle interval should be 100 ms (delay(100)), saw {dt:.3} ms"
        );
    }
}
