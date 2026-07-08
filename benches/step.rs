// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! Instruction-throughput benchmarks for the step loop. Guards the "no
//! allocation in the hot loop" invariant indirectly: sustained tens of
//! millions of instructions per second is only possible if `step` allocates
//! nothing per instruction.
//!
//! `cargo bench`

use criterion::{Criterion, criterion_group, criterion_main};
use rt1060_rs::memory::map;
use rt1060_rs::{Rt1060, loader};
use std::hint::black_box;

/// A tiny SDRAM image: vector table + a 3-instruction ALU/branch spin loop.
fn alu_loop() -> loader::LoadedImage {
    let code_addr = map::SDRAM_BASE + 0x100;
    let mut data = Vec::new();
    data.extend_from_slice(&(map::DTCM_BASE + 0x1000).to_le_bytes()); // SP
    data.extend_from_slice(&(code_addr | 1).to_le_bytes()); // reset (thumb)
    data.resize(0x100, 0);
    // loop: adds r0,#1 ; adds r1,#1 ; b loop
    for hw in [0x3001u16, 0x3101, 0xE7FC] {
        data.extend_from_slice(&hw.to_le_bytes());
    }
    loader::load_bin(map::SDRAM_BASE, &data)
}

fn bench_step(c: &mut Criterion) {
    // Synthetic ALU/branch loop — the CPU-core hot path.
    c.bench_function("step_alu_loop_10k", |b| {
        let img = alu_loop();
        let mut soc = Rt1060::boot(&img);
        soc.quiet();
        b.iter(|| {
            for _ in 0..10_000 {
                soc.step();
            }
            black_box(soc.core.regs[0]);
        });
    });

    // Real firmware: the RT1050 SDK blinky (clock/GPIO init + delay loop).
    c.bench_function("step_rt1050_blinky_10k", |b| {
        let bytes = include_bytes!("../tests/fixtures/rt1050_led_blinky_itcm.elf");
        let img = loader::load_elf(bytes).expect("elf");
        let mut soc = Rt1060::boot(&img);
        soc.quiet();
        b.iter(|| {
            for _ in 0..10_000 {
                soc.step();
            }
            black_box(soc.core.regs[15]);
        });
    });
}

criterion_group!(benches, bench_step);
criterion_main!(benches);
