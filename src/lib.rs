// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! rt1060-rs — NXP i.MX RT1060 (MIMXRT1062DVL6B, Cortex-M7) emulator.
//!
//! Runs real, unmodified firmware — MadMachine SwiftIO (embedded Swift),
//! Zephyr, Arduino — in the rp2040js / rp2350-rs / mg24-rs paradigm: the
//! CPU is register state only, and every memory access goes through a
//! borrowed [`cortex_m::Bus`]. No `Rc<RefCell>`, no trait objects in the hot
//! loop, no allocation per step.
//!
//! Read `docs/DESIGN.md` for the pinned architecture and `ROADMAP.md` for
//! the milestone plan. Register truth: the CMSIS headers and SVD under
//! `../legacy-mcux-sdk/devices/MIMXRT1062/` and `../mcux-soc-svd/MIMXRT1062/`
//! (see CLAUDE.md hard rules).

pub mod cortex_m;
pub mod gdb;
pub mod loader;
pub mod memory;
pub mod peripherals;
pub mod rt1060;

pub use rt1060::Rt1060;
