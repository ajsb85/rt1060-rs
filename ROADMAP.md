# ROADMAP — rt1060-rs

Milestone plan for emulating the NXP i.MX RT1060 (MIMXRT1062, SwiftIO Micro)
well enough to boot MadMachine SwiftIO firmware, then Zephyr and Arduino.
Ordered by dependency. Check items off as they land and mirror the snapshot
table in `README.md`.

Legend: ✅ done · ⏳ partial / stored-readback · ⬜ not started.

## M0 — Project bootstrap ✅

- [x] Cargo crate, MIT license headers, `.gitignore`/`.gitattributes`
- [x] tbdflow config, DoD checklist, Conventional Commits, SSH-signed commits
- [x] CLAUDE.md / CONTRIBUTING.md / README.md / ROADMAP.md / DESIGN.md
- [x] GitHub repo (Actions disabled), topics + description

## M1 — Cortex-M7 core ✅

Ported from the mg24-rs Cortex-M33 core (shared Thumb-2 mainline + DSP ISA).

- [x] Thumb-16 + Thumb-2 wide decode/execute, IT blocks, DSP/SIMD, sat
- [x] Exceptions: SVC/PendSV/SysTick/NVIC, stack framing, EXC_RETURN, nesting
- [x] **NVIC widened to 158 external IRQs** (`IrqMask`, 5×32-bit words)
- [x] SysTick, MPU (PMSAv7, 16 regions, stored-readback), CPUID = M7 r1p2
- [x] 98 instruction/exception unit tests green
- [ ] FPv5-**D16** double-precision FPU (SP path ported; DP ops + `vldm.64`
      / `vstm.64` widening still needed — SwiftIO is soft-float, so deferred)
- [ ] L1 I/D cache maintenance ops (CCR IC/DC, `mcr` cache ops) as no-ops
- [ ] Decode cache for the flash/SDRAM instruction stream (rp2350-rs precedent)

## M2 — Memory map + AIPS bus ✅

- [x] ITCM `0x0` / DTCM `0x2000_0000` / OCRAM `0x2020_0000`
- [x] FlexSPI NOR flash `0x6000_0000` (16 MB, erased 0xFF)
- [x] SEMC SDRAM `0x8000_0000` (32 MB)
- [x] AIPS peripheral routing by 16 KiB base, narrow-IO lanes, warn-once unmapped
- [ ] FlexRAM bank routing via `IOMUXC_GPR_GPR14/16/17` (TCM/OCRAM partition)
- [ ] Boot ROM stub at `0x0020_0000` (mask-ROM API entry points)

## M3 — Loader ✅

- [x] ELF32-LE (PT_LOAD segments, `e_entry`)
- [x] Raw binary at an explicit address
- [x] MadMachine `micro.img` (4 KiB header → SDRAM payload)
- [ ] `swiftio.bin` (payload + CRC32 trailer) + CRC/SHA-256 verification
- [ ] Intel HEX / SREC (NXP SDK example artifacts)

## M4 — Starter peripherals ✅ / ⏳

- [x] **LPUART** (console TX/RX FIFO, STAT/CTRL, TX/TC/RX interrupts)
- [x] **GPIO** (DR/GDIR/PSR, DR_SET/CLR/TOGGLE, combined interrupts)
- [x] **CCM** (clock gates readback, CDHIPR-not-busy)
- [x] **GPT** (free-running counter, prescaler, output compare + interrupt)
- [x] **SRC** (reset cause SRSR, boot-mode/GPR readback, SW reset request)
- [x] **WDOG1/2 + RTWDOG** (stored-readback, no bite yet)
- [x] CCM_ANALOG (PLL LOCK bits seeded), IOMUXC, GPC, SNVS, DCDC, OCOTP, PIT
      as stored-readback `RawRegs`

## M5 — Clock tree & board bring-up ⬜

- [ ] CCM_ANALOG PLL/PFD model (ARM PLL 600 MHz, SYS PLL, USB/ENET/AUDIO)
- [ ] CCM clock-root dividers feeding GPT/PIT/LPUART baud
- [ ] SEMC controller + SDRAM init handshake (so a bootloader path works)
- [ ] IOMUXC pad mux + the SwiftIO id → (GPIO, pin) map (extract from
      `../mm-sdk/boards/SwiftIOMicro/lib/.../libboards__arm__mm_feather.a`)
- [ ] Observable RGB LED (SwiftIO ids 44/45/46, active-low) and 44-pin table

## M6 — DMA & serial/analog peripherals ⬜

- [ ] eDMA + DMAMUX (Zephyr routes LPUART/LPSPI/ADC through it)
- [ ] LPSPI ×2, LPI2C ×2 (SwiftIO SPI/I²C buses + MadDrivers)
- [ ] ADC (14 channels), PWM (14), FlexPWM, QTMR
- [ ] PIT (periodic interrupt timer) full model

## M7 — Storage, USB, connectivity ⬜

- [ ] USDHC + SD card image (SwiftIO loads user apps / assets from SD)
- [ ] USB OTG1 device (CDC-ACM: the `mm download` / console bridge)
- [ ] FlexSPI controller register model + XIP program/erase
- [ ] ENET, CAN (FlexCAN), I²S (SAI)

## M8 — Boot a real SwiftIO image ⬜

- [ ] Boot the MadMachine bootloader ("eboot") from FlexSPI flash
- [ ] Two-stage boot: verify `micro.img`, copy to SDRAM, jump to `__start`
- [ ] Bring Zephyr up to `PRE_KERNEL → APPLICATION`; assert console banner
- [ ] Run the SwiftIO `01LED` / `Blink` example; assert RGB LED toggling
- [ ] HIL parity: compare against a physical SwiftIO Micro over USB-serial

## M9 — Tooling ⬜

- [ ] GDB remote-serial stub (rp2350-rs precedent): breakpoints, mem, regs
- [ ] WASM front-end (`wasm-bindgen`) for an in-browser SwiftIO playground
- [ ] `criterion` benchmarks; keep the hot loop allocation-free
- [ ] Trace/log gating by env var (`RT1060_TRACE`)

## Known simplifications (revisit as firmware demands)

- Caches are coherent no-ops; no wait states or bus contention (cycle-lite).
- Watchdogs never bite; PLLs lock instantly; SDRAM is pre-mapped RAM.
- The `unknown` peripheral fallback aliases distinct unmodeled blocks onto
  one page — every first touch logs, forming the TODO list for M5–M7.
- SAU registers in the core are vestigial (M7 is Armv7E-M, no TrustZone).
