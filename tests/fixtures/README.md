# Test fixtures

Real firmware binaries used by the integration tests. Every fixture must be a
real hardware artifact or an unmodified public release, with provenance
recorded here. If the emulator diverges from a physical SwiftIO Micro running
the same fixture, the emulator is wrong.

## Candidates (to be added as milestones land)

- **MadMachine SwiftIO examples** — build `../Template/Examples/Blink` or
  `../MadExamples/Examples/SwiftIOPlayground/01LED` with the `mm` toolchain;
  the `micro.img` (SwiftIO Micro) loads to SDRAM `0x8000_0000`.
- **SerialLoader.bin** — `../mm-sdk/boards/SerialLoader.bin`, the MadMachine
  second-stage RAM loader (initial SP `0x2002_b238`, reset `0x0000_4138`;
  runs from ITCM `0x0`). Good early CPU/vector-table bring-up fixture.
- **NXP SDK LED blink** — `../mcu-boot-utility/apps/.../led_blinky_0x*_iar.elf`
  (RT1050 ≈ RT1062) at ITCM `0x0000_2000` or FlexSPI `0x6000_2000`.

## Committed fixtures

### `rt1050_led_blinky_itcm.elf`

A real, **unmodified** NXP MCUXpresso SDK LED-blinky for the i.MX RT1050 EVK,
IAR-built to run from ITCM at `0x0000_2000` (vector table there; reset handler
`0x0000_4fc0`, initial SP `0x2002_0000`). Copied verbatim from
`../mcu-boot-utility/apps/NXP_MIMXRT1050-EVKB_Rev.A/led_blinky_0x00002000_iar.elf`.

RT1050 and RT1062 share the Cortex-M7 core and are register-compatible for the
blocks this image touches (CCM clocks, IOMUXC, GPIO). Exercised by
`tests/boot_fixture.rs`:

- it boots and runs its clock/pin-mux/GPIO init with **zero unmapped-register
  or unimplemented-instruction hits**, and configures the EVK user LED
  (`GPIO1_IO09`) as an output (fast test);
- run long enough, `GPIO1_IO09` **toggles** — the LED blinks (the `--ignored`
  deep test, ~250M instructions).

### `rt1050_led_blinky_flexspi.elf` / `rt1050_led_blinky_sdram.elf`

The same SDK blinky linked to run **XIP from FlexSPI NOR** (`0x6000_2000`) and
**from SDRAM** (`0x8000_2000`, the MadMachine run location). Both boot, run
init cleanly, and blink `GPIO1_IO09` — proving the flash-XIP and SDRAM code
paths. From the same SDK app dir as the ITCM variant.

Still available (not committed): `../mm-sdk/boards/SerialLoader.bin` (the
MadMachine second-stage RAM loader).
