# Contributing to rhcs

Thanks for helping. Bug reports, hardware reports and pull requests are all welcome.

## Reporting hardware

Most of rhcs depends on how real devices behave. If you test it on a router or phone
that isn't listed in the README, open an issue with:

- device model and firmware version
- what worked and what didn't (`rhcs router --json` and `rhcs signal --once --json` output helps)

**Remove passwords, IMEIs, serial numbers, SIM numbers, MAC addresses and cell/tower IDs
before posting.** Tower IDs reveal your approximate location.

## Pull requests

1. Fork, create a branch, keep the change focused.
2. `cargo fmt`, `cargo clippy`, `cargo test` must pass.
3. Never commit `.env`, logs or real device identifiers. Tests that need real hardware
   values read them from the local `.env` and skip when it's missing.
4. Describe what you tested and on which hardware.

## Contributor agreement

rhcs is dual-licensed (AGPL-3.0 and a commercial license, see the README). To keep that
possible, every contribution is accepted under these terms. By opening a pull request
you agree that:

1. You wrote the contribution, or have the right to submit it.
2. Your contribution is licensed to the public under the AGPL-3.0, like the rest of rhcs.
3. You also grant the project maintainer a perpetual, worldwide, non-exclusive,
   royalty-free, irrevocable license to use, modify, sublicense and distribute your
   contribution under other license terms, including commercial licenses.
4. You keep the copyright to your contribution.

If you can't agree to this, say so in the pull request before it's merged.
