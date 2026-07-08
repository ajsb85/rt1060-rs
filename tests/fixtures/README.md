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

None are committed yet — the current tests build their images in-process
(`tests/smoke.rs`).
