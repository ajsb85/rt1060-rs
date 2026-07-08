// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! Minimal CLI runner: load a firmware image, run it, print the LPUART1
//! console output.
//!
//! ```text
//! cargo run --example run_image -- <file> [load_addr_hex] [max_steps]
//! ```
//!
//! The loader is chosen by extension: `.elf` → ELF32, `.img` → MadMachine
//! `micro.img`, anything else → raw binary at `load_addr_hex` (default SDRAM
//! `0x80000000`).

use rt1060_rs::{Rt1060, loader};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        eprintln!("usage: run_image <file> [load_addr_hex] [max_steps]");
        std::process::exit(2);
    };
    let load_addr = args
        .get(2)
        .map(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap())
        .unwrap_or(loader::IMAGE_LOAD_ADDRESS);
    let max_steps: u64 = args
        .get(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);

    let bytes = std::fs::read(path).expect("read firmware file");
    let image = if path.ends_with(".elf") {
        loader::load_elf(&bytes).expect("parse ELF")
    } else if path.ends_with(".img") {
        loader::load_micro_img(&bytes).expect("parse micro.img")
    } else {
        loader::load_bin(load_addr, &bytes)
    };

    let mut soc = Rt1060::boot(&image);
    println!(
        "[rt1060-rs] booted: SP={:#010x} PC={:#010x}",
        soc.core.regs[13], soc.core.regs[15]
    );
    let halt = soc.run(max_steps);
    let out = soc.console_string();
    if !out.is_empty() {
        println!("--- LPUART1 console ---\n{out}");
    }
    if let Some(cause) = halt {
        println!("[rt1060-rs] halted: {cause:?} after {} cycles", soc.cycles);
    } else {
        println!("[rt1060-rs] ran {max_steps} steps ({} cycles)", soc.cycles);
    }
}
