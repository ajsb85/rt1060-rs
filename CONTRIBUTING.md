# Contributing to rt1060-rs

Thanks for your interest! This project moves fast and stays disciplined.

## Workflow: Trunk-Based Development with tbdflow

We use [tbdflow](https://crates.io/crates/tbdflow) for a trunk-based workflow:

- Work happens on `main` (the trunk) or on **short-lived** branches (< 1 day).
- Commit with `tbdflow commit -t <type> -s <scope> -m "<message>"` — it
  validates the message, commits, and pushes in one step.
- For a short-lived branch: `tbdflow branch -t feature -n <name>`, then
  `tbdflow complete -t feature -n <name>` to merge it back and delete it.
- `tbdflow sync` before starting work.
- Commits are SSH-signed (repo relies on the global `commit.gpgsign` /
  `gpg.format ssh` config).

## Commit messages: Conventional Commits

Every commit follows [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <imperative, present-tense description>
```

Subject: lowercase, ≤ 72 characters, no trailing period. Body lines ≤ 80
characters.

Types: `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `chore`, `ci`,
`build`, `revert`, `style`.

Scopes match the module tree: `cpu`, `memory`, `bus`, `gpio`, `ccm`,
`lpuart`, `gpt`, `src`, `wdog`, `iomuxc`, `loader`, `board`, `repo`,
`design`, `docs`, `fixtures`.

Examples:

```
feat(lpuart): model the RX FIFO watermark and idle-line interrupt
fix(ccm): read CDHIPR as not-busy so CLOCK_SetDiv spin-loops terminate
perf(cpu): pre-decode the flash icache into the op cache
test(gpio): assert the combined interrupt lines for GPIO2
```

## Quality bar

Before pushing (tbdflow runs the commit; you run the checks):

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

- **Instruction semantics are sacred.** Every new instruction lands with unit
  tests covering flags (N/Z/C/V), edge operands, and PC-relative behavior.
- **No allocations in the hot loop.** If your change allocates per-step,
  rework it.
- **Registers come from the sources.** Cite the CMSIS header
  (`MIMXRT1062.h`), the SVD (`MIMXRT1062.xml`), the i.MX RT1060 reference
  manual section, or the cross-checked Renode model in a comment for any
  nontrivial register behavior. Never guess a mask.

## Testing against real hardware

Test fixtures under `tests/fixtures/` are real binaries (MadMachine SwiftIO
builds, unmodified NXP SDK examples, or dumps from a physical SwiftIO Micro).
If you add a fixture, document how it was built or extracted in
`tests/fixtures/README.md`. If the emulator diverges from the attached board,
the emulator is wrong.
