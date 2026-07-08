// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! Teensy 4.x (i.MX RT1062) boot runner + diagnostic.
//!
//! Models the i.MX RT Boot ROM path the PJRC Teensyduino core relies on: the
//! ROM reads the FlexSPI NOR config at `0x6000_0000`, parses the Image Vector
//! Table (IVT) at `0x6000_1000`, and branches to `IVT.entry` (the `naked`
//! `ResetHandler`, which sets its own SP via `mov sp, _estack`). Reports the
//! pin-13 LED (`GPIO_B0_03`), unimplemented instructions, and stuck PCs.
//!
//! `cargo run --release --example teensy -- blink_Teensy41.elf [max_steps]`

use rt1060_rs::{Rt1060, loader};
use std::collections::BTreeMap;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .expect("usage: teensy <firmware.elf> [max_steps]");
    let max: u64 = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(50_000_000);

    let bytes = std::fs::read(path).expect("read firmware");
    let img = loader::load_elf(&bytes).expect("elf");
    let mut soc = Rt1060::cold_boot_from_ivt(&img).expect("i.MX RT IVT boot");
    soc.quiet();
    println!(
        "IVT @{:#010x}: entered ResetHandler at PC={:#010x}",
        img.entry.unwrap(),
        soc.core.regs[15]
    );

    // Pin 13 = pad GPIO_B0_03, driven by GPIO7 IO3 (Teensyduino's fast-GPIO
    // alias) OR GPIO2 IO3 (the normal alias the SwiftIO HAL uses for Id.D16).
    // Report the LED as on if either alias drives the pad high.
    let led_pin = 3;
    let led = |soc: &Rt1060| {
        let g7 =
            soc.bus.periph.gpio[6].is_output(led_pin) && soc.bus.periph.gpio[6].output(led_pin);
        let g2 =
            soc.bus.periph.gpio[1].is_output(led_pin) && soc.bus.periph.gpio[1].output(led_pin);
        g7 || g2
    };

    let mut unimpl: BTreeMap<u32, (u32, u64)> = BTreeMap::new();
    let mut min_pc = u32::MAX;
    let mut max_pc = 0u32;
    let mut toggles = 0u64;
    let mut last_led = false;
    let mut last_pc = 0u32;
    let mut last_toggle_cyc = 0u64;
    let chunk = 1_000_000u64;
    let mut done = 0u64;
    let mut stuck: BTreeMap<u32, u64> = BTreeMap::new();
    while done < max {
        for _ in 0..chunk {
            let pc = soc.core.regs[15];
            soc.step();
            if let Some(rt1060_rs::cortex_m::BreakCause::Unimplemented(hw)) = soc.core.break_cause {
                let e = unimpl.entry(hw).or_insert((pc, 0));
                e.1 += 1;
                soc.core.break_cause = None;
            }
            let npc = soc.core.regs[15];
            if npc == pc {
                *stuck.entry(pc).or_insert(0) += 1;
            }
            min_pc = min_pc.min(npc);
            max_pc = max_pc.max(npc);
            let l = led(&soc);
            if l != last_led {
                let ms = soc.cycles as f64 / soc.core_hz() as f64 * 1000.0;
                let dms = (soc.cycles - last_toggle_cyc) as f64 / soc.core_hz() as f64 * 1000.0;
                if toggles < 12 {
                    println!(
                        "  toggle {:2}: LED={} @ {:.2} ms (+{:.2} ms, {} cyc)",
                        toggles + 1,
                        l as u8,
                        ms,
                        dms,
                        soc.cycles - last_toggle_cyc
                    );
                }
                last_toggle_cyc = soc.cycles;
                toggles += 1;
                last_led = l;
            }
            last_pc = npc;
        }
        done += chunk;
    }
    println!("after {done} steps ({} cycles):", soc.cycles);
    println!(
        "  PC now {:#010x} (range {min_pc:#010x}..{max_pc:#010x})",
        last_pc
    );
    println!("  LED (pad GPIO_B0_03, pin {led_pin}) toggles: {toggles}, now {last_led}");
    println!(
        "  core_hz={} systick csr={:#x} rvr={}",
        soc.core_hz(),
        soc.core.syst_csr,
        soc.core.syst_rvr
    );
    let mut r = |a: u32| soc.bus.read32(a);
    println!(
        "  CCM: CACRR={:#010x} CBCDR={:#010x} CBCMR={:#010x} CSCMR1={:#010x}",
        r(0x400F_C010),
        r(0x400F_C014),
        r(0x400F_C018),
        r(0x400F_C01C)
    );
    println!(
        "  ANALOG: PLL_ARM={:#010x} PFD_528={:#010x}",
        r(0x400D_8000),
        r(0x400D_8100)
    );
    if !unimpl.is_empty() {
        println!("  UNIMPLEMENTED (encoding → first PC, count):");
        for (hw, (pc, n)) in &unimpl {
            println!("    {hw:#06x} at {pc:#010x}  ×{n}");
        }
    }
    let hot: Vec<_> = {
        let mut v: Vec<_> = stuck.iter().collect();
        v.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
        v.into_iter().take(5).collect()
    };
    if !hot.is_empty() {
        println!("  hottest self-loop PCs:");
        for (pc, n) in hot {
            println!("    {pc:#010x}  ×{n}");
        }
    }
    let out = soc.console_string();
    if !out.is_empty() {
        println!("--- LPUART1 ---\n{out}");
    }
}
