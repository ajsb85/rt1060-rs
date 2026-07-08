# Building `swiftio_teensy_blink.{elf,hex}`

The **real MadMachine SwiftIO stack** (`import SwiftIO`, HalSwiftIO, Zephyr,
embedded Swift) running on a **Teensy 4.1** (same NXP MIMXRT1062 silicon as the
SwiftIO Micro). Unlike the SwiftIO Micro, the Teensy has **no SDRAM**, so the
image is re-based to run entirely from the RT1062's dedicated **512 KB OCRAM at
`0x2020_0000`** (the whole blink footprint is ~406 KB — it fits), and wrapped in a
Teensy flash image (FlexSPI config block + IVT + a first-stage) the Boot ROM can
launch.

## 1. Re-base the SwiftIO build off SDRAM into OCRAM

`mm build` links via `mm-sdk/boards/SwiftIOMicro/linker/sdram.ld`, which places
everything in SDRAM `0x8000_0000`. Copy it to `ocram.ld` and re-base the `SRAM`
region (three edits — the whole layout just shifts base):

- `SRAM (wx) : ORIGIN = 0x80000000, LENGTH = (32768 * 1K - 16 * 1K)`
  → `ORIGIN = 0x20200000, LENGTH = (512 * 1K)`
- `. = 0x80000000;` → `. = 0x20200000;`
- `__kernel_ram_end = 0x80000000 + (32768 * 1K - 16 * 1K);`
  → `__kernel_ram_end = 0x20200000 + (512 * 1K);`

The `mm` binary reads `sdram.ld` by that fixed name, so build with the OCRAM
layout in place: `cp ocram.ld sdram.ld` (keep a backup), then build the project
below and restore `sdram.ld` afterwards.

## 2. The Swift blink (`main.swift`)

`DigitalOut(Id.D16)` — SwiftIO logical id **D16 maps to GPIO2 IO3 = pad
`GPIO_B0_03`**, which is exactly the Teensy 4.1 onboard LED (pin 13). No HAL
change needed: the SwiftIO HAL drives the normal GPIO2 alias of that pad (the
Teensyduino core's GPIO7 is just the fast alias). Build with the mm SDK 2.2.0
flow (`mm build`), then flatten to a raw binary:

```
arm-none-eabi-objcopy -O binary .build/armv7em-none-none-eabi/release/DemoBlink swiftio.bin
```

## 3. Wrap as a Teensy flash image (`stage.c`, `payload.S`, `stage.ld`)

A minimal first-stage the Teensy Boot ROM launches: `stage.c`'s `ResetHandler`
(entered from the IVT) copies `swiftio.bin` (embedded via `payload.S`) into OCRAM
`0x2020_0000`, points `VTOR` there, and jumps to the SwiftIO reset vector.
`bootdata.c` (the FlexSPI config block + IVT + boot_data) is reused verbatim from
`teensy_cores/teensy4/bootdata.c`:

```
FLAGS="-mcpu=cortex-m7 -mthumb -mfloat-abi=soft -Os -ffreestanding -nostdlib -Wall"
arm-none-eabi-gcc $FLAGS -DARDUINO_TEENSY41 -c teensy_cores/teensy4/bootdata.c -o bootdata.o
arm-none-eabi-gcc $FLAGS -c stage.c -o stage.o
arm-none-eabi-gcc $FLAGS -c payload.S -o payload.o    # incbin swiftio.bin
arm-none-eabi-gcc $FLAGS -T stage.ld -Wl,--gc-sections -Wl,-e,ImageVectorTable \
    bootdata.o stage.o payload.o -o swiftio_teensy_blink.elf
arm-none-eabi-objcopy -O ihex swiftio_teensy_blink.elf swiftio_teensy_blink.hex
```

## 4. Flash the real board

```
teensy_loader_cli --mcu=TEENSY41 swiftio_teensy_blink.hex
```

The onboard LED blinks at ~1 Hz (`sleep(ms: 500)`) — verified on real hardware,
and reproduced in the emulator by `tests/teensy_hil.rs`.
