# rt1060-rs

A cycle-lite, dependency-free **Rust emulator for the NXP i.MX RT1060**
(MIMXRT1062DVL6B, single Arm Cortex-M7). It runs **real, unmodified
firmware** — the end goal is booting **MadMachine SwiftIO** (embedded Swift)
on the **SwiftIO Micro** board, with Zephyr and Arduino images along the way.

Built in the lineage of [rp2040js](https://github.com/wokwi/rp2040js),
[rp2350-rs](https://github.com/ajsb85/rp2350-rs), and
[mg24-rs](https://github.com/ajsb85/mg24-rs): the CPU is register state only,
every memory access goes through a borrowed `Bus`, and there is no
`Rc<RefCell>`, no trait objects, and no allocation in the hot loop.

## Target hardware — MadMachine SwiftIO Micro

| | |
|---|---|
| MCU | NXP **MIMXRT1062** (i.MX RT1060) |
| Core | Arm **Cortex-M7** r1p2, FPv5-D16, L1 I/D cache, MPU |
| Clock | 600 MHz |
| RAM | 32 MB SDRAM (SEMC @ `0x8000_0000`) + 512 KB FlexRAM TCM + 1 MB OCRAM |
| Flash | 16 MB FlexSPI NOR (`0x6000_0000`) |
| I/O | 44 GPIO, 14 ADC, 14 PWM, 2 I²C, 2 SPI, 3 UART, 1 I²S, 1 CAN, SD, RGB LED |

## Status

Milestone tracking lives in [`ROADMAP.md`](ROADMAP.md). Snapshot:

| Area | State |
|---|---|
| Cortex-M7 core (Thumb-2 + DSP, exceptions, NVIC 158 IRQs, SysTick, MPU) | ✅ ported, 98 tests |
| Memory map + AIPS bus (ITCM/DTCM/OCRAM/FlexSPI/SDRAM) | ✅ |
| Loader (ELF32, raw bin, MadMachine `micro.img`) | ✅ |
| LPUART (console), GPIO, CCM, GPT, SRC, WDOG | ✅ |
| CCM_ANALOG PLLs (LOCK/OSC/DCDC), SEMC status, boot spin-loops | ✅ terminate |
| RGB LED observable (GPIO1 9/10/11, active-low) via `led_rgb()` | ✅ |
| Clock-tree frequency model (core/IPG/PERCLK/UART) → `core_hz()` | ✅ |
| PIT, GPT (clock-domain accurate) | ✅ |
| eDMA (32-ch TCD through the bus) + DMAMUX hw requests, LPI2C, LPSPI, ADC | ✅ + device hooks |
| FlexPWM (`pwm_duty()`) + QTMR (PERCLK counting) | ✅ |
| USDHC + attachable **SD card** (init + block read/write) | ✅ |
| USB OTG register block (init runs; full CDC enumeration ⬜) | ✅ |
| **Boots a real NXP SDK Cortex-M7 blinky — LED toggles** | ✅ |
| **GDB remote stub** (`gdb-multiarch`) + `RT1060_TRACE` logging | ✅ |
| USB CDC enumeration, SEMC command decode, ENET/CAN/SAI | ⬜ ROADMAP |
| IOMUXC full 44-pin table, PLL PFD fractional math | ⏳ |
| Double-precision FPU (FPv5-D16) | ⬜ ROADMAP (SwiftIO builds soft-float) |
| WASM front-end, boot a real SwiftIO Micro image | ⬜ ROADMAP |

## Quick start

```rust
use rt1060_rs::{Rt1060, loader};

// Load a MadMachine user image (runs from SDRAM at 0x8000_0000).
let bytes = std::fs::read("micro.img").unwrap();
let image = loader::load_micro_img(&bytes).unwrap();
let mut soc = Rt1060::boot(&image);

// Run and read what the firmware printed on the LPUART1 console.
soc.run(1_000_000);
print!("{}", soc.console_string());
```

Raw binaries and ELFs load the same way via `loader::load_bin(addr, &data)`
and `loader::load_elf(&bytes)`.

### Debug with GDB

```bash
cargo run --example gdbserver -- firmware.elf 3333
gdb-multiarch -ex "target remote :3333" firmware.elf
```

Register/memory access, breakpoints, and step/continue all work. Set
`RT1060_TRACE=1` to log peripheral writes (`=all` for reads too) while a
firmware image brings the chip up — the fastest way to see which registers a
driver pokes.

## Build & test

```bash
cargo test                                   # unit + integration tests
cargo clippy --all-targets -- -D warnings    # lint gate
cargo fmt --all -- --check                   # format gate
```

The library is `#![no_std]`-friendly in spirit (host-side `std` for I/O
sinks) and has **zero runtime dependencies**.

## Register truth

Every nontrivial register cites its source: the CMSIS header
`MIMXRT1062.h`, the CMSIS-SVD `MIMXRT1062.xml`, the i.MX RT1060 reference
manual, or the cross-checked Renode `IMX*` models (RT1064 is memory-map
identical to RT1062). See [`CLAUDE.md`](CLAUDE.md) for the hard rules and
[`CONTRIBUTING.md`](CONTRIBUTING.md) for the workflow.

## License

MIT © 2026 Alexander Salas Bastidas `<ajsb85@firechip.dev>`. Portions ported
from mg24-rs / rp2350-rs and inspired by rp2040js (Uri Shaked, MIT).
