# CLAUDE.md — rt1060-rs

Rust emulator for the NXP **MIMXRT1062DVL6B** (i.MX RT1060; single Cortex-M7
r1p2, FPv5-D16, 512 KB FlexRAM TCM + OCRAM, external FlexSPI NOR + SEMC
SDRAM). Default board: the **MadMachine SwiftIO Micro** (RT1062 @ 600 MHz,
32 MB SDRAM, 16 MB flash, onboard RGB LED, 44 IO pins). Goal: run real,
unmodified firmware — MadMachine SwiftIO (embedded Swift), Zephyr, Arduino —
following the rp2040js / rp2350-rs / mg24-rs paradigm.

**Read `docs/DESIGN.md` before writing code** — it pins the architecture.
**Read `ROADMAP.md`** for what's done and what's next; keep both current.

## Hard rules

- **mg24-rs (`/home/ajsb8/xiao/EFR32MG24/mg24-rs`) and rp2350-rs
  (`/home/ajsb8/dev/RP2350/rp2350-rs`) are the paradigm and code quarry.**
  The `cortex_m` core, `Bus` trait, loader, and test scaffolding port
  nearly unchanged. The Cortex-M7 is Armv7E-M and runs the same Thumb-2
  mainline + DSP ISA as the M33/M4, so the decode/execute path is shared;
  M7 differences: **158 external IRQs** (widened `IrqMask`, not `u128`),
  no TrustZone/SAU, an L1 cache (modeled as coherent/no-op), and an
  optional FPv5-**D16** (double-precision) FPU. `rp2040-rs` remains the
  anti-pattern catalog: never copy its structure.
- **Register truth**, in this order:
  1. `../legacy-mcux-sdk/devices/MIMXRT1062/MIMXRT1062.h` (bases, IRQn
     table, memory map, every field/mask/reset);
  2. `../mcux-soc-svd/MIMXRT1062/MIMXRT1062.xml` (machine-readable SVD);
  3. `../IMXRT1060RM.pdf` (behavior);
  4. Renode `../renode/platforms/cpus/imxrt1064.repl` +
     `../renode-infrastructure/.../Peripherals/**/IMX*_*.cs` (cross-check;
     RT1064 is memory-map-identical to RT1062).
  Cite the header line / SVD / RM section in a comment for nontrivial
  behavior.
- **The AIPS bus routes by 16 KiB-aligned base** (`addr & !0x3FFF`). Unlike
  the EFR32, there is **no SET/CLR/TGL alias, no Secure/NS mirror** — plain
  word-register blocks. The PPB (`0xE000_0000+`) is intercepted by the core.
- **No `Rc<RefCell>`, no trait objects in the execution path.** Plain data +
  split borrows: `core.step(&mut bus)`. Zero allocations / no hashing in the
  hot loop. Library stays dependency-free (dev-dependencies fine).
- **The MadMachine model is two-stage.** A resident bootloader ("eboot") in
  FlexSPI flash reads the `micro.img`/`swiftio.bin` header (load address
  `0x8000_0000`, SHA-256/CRC verify) and copies the user image into SDRAM,
  then jumps to `__start`. For bring-up we **load the user image straight to
  SDRAM and set VTOR to `0x8000_0000`** (its vector table); the bootloader
  and SEMC/SDRAM init are modeled later (see ROADMAP). The SwiftIO firmware
  is built **`nofp` (soft-float)** — the D16 FPU can be deferred.
- **The console is LPUART1.** Zephyr's newlib/`printf` retarget and the
  USB-serial bridge used by `mm download` both route through it — it is the
  first peripheral that makes a boot observable.
- **Fixtures come from real hardware or unmodified releases**, provenance
  documented in `tests/fixtures/README.md`. If the emulator diverges from a
  real SwiftIO Micro, the emulator is wrong.

## Testing methodology (rp2040js-style)

- Every instruction lands with unit tests asserting registers AND all four
  flags (N/Z/C/V) — the ported `tests.rs`/`tests_wide.rs`/`asm.rs` encoders
  are already cross-validated against GNU `as`.
- Integration tests boot fixtures and assert LPUART1 output / GPIO state.
- Before each commit: `cargo fmt --all -- --check && cargo clippy
  --all-targets -- -D warnings && cargo test`. All three must be clean.

## Workflow

- Repo: https://github.com/ajsb85/rt1060-rs (GitHub Actions disabled by
  choice — quality gates run locally). Trunk-based on `main` via tbdflow;
  commit with every meaningful unit:
  `tbdflow commit -t <type> -s <scope> -m "<subject>" --body "..." --no-verify`
  (`--no-verify` skips the interactive DoD checklist, required non-tty;
  subject: lowercase, ≤72 chars, no period; body lines ≤80 chars).
  Commits are SSH-signed automatically (global `commit.gpgsign`).
- Types: feat fix perf refactor test docs chore ci. Scopes are **letters
  only** (the tbdflow lint rejects digits — use `spi`/`dma`/`adc`/`periph`,
  not `lpspi`/`lpi2c`): cpu memory bus gpio ccm clocks pit gpt dma spi adc
  periph src wdog iomuxc semc loader board repo design docs fixtures.
- **tbdflow stages EVERYTHING (`git add -A`)** — keep the working tree free
  of unrelated changes before committing; it also cannot create the root
  commit on an unborn branch (use plain git once, then tbdflow).
- Append to commit bodies:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

## Session startup checklist

1. Read `ROADMAP.md` → pick the top unchecked milestone.
2. Register truth is the CMSIS header / SVD — never guess a mask.
3. Update `ROADMAP.md` checkboxes + the status table in `README.md` when a
   milestone lands.
