// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! Boot-progress probe: load a firmware image, step it in chunks, and report
//! observable activity (GPIO output changes, LPUART console, PWM duty) plus
//! the final CPU state. A quick way to see how far a real image gets and what
//! it drives — the M8 bring-up tool.
//!
//! ```text
//! cargo run --example probe -- firmware.elf [max_steps]
//! ```

use rt1060_rs::peripherals::pwm::Chan;
use rt1060_rs::{Rt1060, loader};

fn gpio_snapshot(soc: &Rt1060) -> [u32; 9] {
    std::array::from_fn(|i| {
        let g = &soc.bus.periph.gpio[i];
        // Only output pins matter for observation.
        (0..32).fold(0u32, |acc, p| {
            if g.is_output(p) && g.output(p) {
                acc | (1 << p)
            } else {
                acc
            }
        })
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("usage: probe <firmware> [max_steps]");
    let max: u64 = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);

    let bytes = std::fs::read(path).expect("read firmware");
    let image = if path.ends_with(".elf") {
        loader::load_elf(&bytes).expect("elf")
    } else {
        loader::load_bin(loader::IMAGE_LOAD_ADDRESS, &bytes)
    };
    let mut soc = Rt1060::boot(&image);
    soc.quiet();
    println!(
        "booted: SP={:#010x} PC={:#010x}  ({} PT_LOAD segments @ base {:#010x})",
        soc.core.regs[13],
        soc.core.regs[15],
        image.segments.len(),
        image.base
    );

    let mut prev_gpio = gpio_snapshot(&soc);
    let mut gpio_edges = 0u64;
    let chunk = 50_000u64;
    let mut done = 0u64;
    while done < max {
        for _ in 0..chunk {
            soc.step();
        }
        done += chunk;
        let now = gpio_snapshot(&soc);
        if now != prev_gpio {
            for (i, (&a, &b)) in prev_gpio.iter().zip(now.iter()).enumerate() {
                if a != b {
                    println!("  @{done:>9} GPIO{} output {a:#010x} -> {b:#010x}", i + 1);
                }
            }
            gpio_edges += 1;
            prev_gpio = now;
        }
    }

    let console = soc.console_string();
    if !console.is_empty() {
        println!("LPUART1 console: {console:?}");
    }
    for inst in 1..=4u8 {
        for (sm, ch) in [(0, Chan::A), (1, Chan::A), (2, Chan::A), (3, Chan::A)] {
            if let Some(d) = soc.pwm_duty(inst, sm, ch) {
                println!("  PWM{inst} SM{sm} A duty = {d:.3}");
            }
        }
    }
    println!(
        "after {done} steps: PC={:#010x}  GPIO edge-events={gpio_edges}  LED(RGB on)={:?}",
        soc.core.regs[15],
        soc.led_rgb()
    );
}
