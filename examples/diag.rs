// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! Boot diagnostic: report the SysTick / exception / IRQ state while stepping,
//! for figuring out where a real RTOS image (e.g. Zephyr) gets stuck.
//!
//! `cargo run --release --example diag -- firmware.elf [max_steps]`

use rt1060_rs::{Rt1060, loader};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("usage: diag <firmware> [max_steps]");
    let max: u64 = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(50_000_000);

    let bytes = std::fs::read(path).unwrap();
    let img = loader::load_elf(&bytes).expect("elf");
    let mut soc = Rt1060::boot(&img);
    soc.quiet();

    let mut exceptions = 0u64;
    let mut last_ipsr = 0u16;
    let mut min_pc = u32::MAX;
    let mut max_pc = 0u32;
    let chunk = 1_000_000u64;
    let mut done = 0u64;
    while done < max {
        for _ in 0..chunk {
            soc.step();
            if soc.core.ipsr != 0 && last_ipsr == 0 {
                exceptions += 1;
            }
            last_ipsr = soc.core.ipsr;
            let pc = soc.core.regs[15];
            min_pc = min_pc.min(pc);
            max_pc = max_pc.max(pc);
        }
        done += chunk;
    }
    let c = &soc.core;
    println!("after {done} steps:");
    println!(
        "  PC = {:#010x}  IPSR = {}  (in exception: {})",
        c.regs[15],
        c.ipsr,
        c.ipsr != 0
    );
    println!("  exceptions taken (0→n transitions): {exceptions}");
    println!(
        "  SysTick: CSR = {:#010x} (enable={}, tickint={}, countflag={}), RVR = {}, CVR = {}",
        c.syst_csr,
        c.syst_csr & 1,
        (c.syst_csr >> 1) & 1,
        (c.syst_csr >> 16) & 1,
        c.syst_rvr,
        c.syst_cvr
    );
    println!("  ICSR pendst (SysTick pending) = {}", c.icsr_pendst);
    println!("  PC range this run: {min_pc:#010x} .. {max_pc:#010x}");
    // Which external IRQ lines are enabled / asserted right now?
    use rt1060_rs::cortex_m::{Bus, NUM_IRQS};
    let lines = soc.bus.irq_lines();
    let enabled: Vec<u32> = (0..NUM_IRQS as u32)
        .filter(|&i| c.nvic_enable.test(i))
        .collect();
    let asserted: Vec<u32> = (0..NUM_IRQS as u32).filter(|&i| lines.test(i)).collect();
    println!("  NVIC enabled IRQs: {enabled:?}");
    println!("  IRQ lines asserted now: {asserted:?}");
}
