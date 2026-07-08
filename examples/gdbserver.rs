// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! Boot a firmware image and serve it to a GDB client over the Remote Serial
//! Protocol — attach with `gdb-multiarch` (or `arm-none-eabi-gdb`) for the
//! autonomous Swift / Zephyr debug workflow.
//!
//! ```text
//! cargo run --example gdbserver -- firmware.elf 3333
//! gdb-multiarch -ex "target remote :3333" firmware.elf
//! ```
//!
//! The loader is chosen by extension (`.elf`/`.img`/raw), same as
//! `run_image`. A raw binary loads to SDRAM `0x8000_0000`.

use rt1060_rs::{Rt1060, gdb::GdbServer, loader};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        eprintln!("usage: gdbserver <firmware> [port]");
        std::process::exit(2);
    };
    let port = args.get(2).map(String::as_str).unwrap_or("3333");

    let bytes = std::fs::read(path)?;
    let image = if path.ends_with(".elf") {
        loader::load_elf(&bytes).expect("parse ELF")
    } else if path.ends_with(".img") {
        loader::load_micro_img(&bytes).expect("parse micro.img")
    } else {
        loader::load_bin(loader::IMAGE_LOAD_ADDRESS, &bytes)
    };
    let mut soc = Rt1060::boot(&image);

    let addr = format!("127.0.0.1:{port}");
    let (mut server, local) = GdbServer::accept(&addr)?;
    eprintln!("[rt1060-rs] gdb server on {local}; connect with:");
    eprintln!("           gdb-multiarch -ex \"target remote :{port}\" {path}");
    server.run(&mut soc)?;
    eprintln!("[rt1060-rs] gdb client detached");
    Ok(())
}
