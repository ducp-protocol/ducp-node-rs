# Contributing to DUCP Node

Thanks for your interest in the DUCP reference implementation. This repository implements the protocol specified in [`ducp-protocol/spec`](https://github.com/ducp-protocol/spec).

## Before you start

- Read the spec (white paper v0.1.0) and open an issue to discuss substantial work first.
- **Protocol-level** changes belong in the [spec repo's proposals](https://github.com/ducp-protocol/spec/tree/main/proposals), not here — this repository *implements* the spec, it does not *define* it.

## Workflow

1. Open an issue describing the bug or change.
2. Fork, branch, and keep pull requests focused.
3. Before pushing: `cargo fmt`, `cargo clippy --all-targets`, and `cargo test`.
4. Submit a pull request referencing the issue.

## Contributor License Agreement

This repository is governed by the project's [DUCP CLA](https://github.com/ducp-protocol/spec/blob/main/CLA.md): contributions are accepted under the CLA, which assigns the contributed rights to the author and grants you a license back. You will be asked to agree before your first contribution is merged.

## Conduct & governance

All participation follows the [Code of Conduct](CODE_OF_CONDUCT.md). During the pre-1.0 phase the project is steered by its maintainer — see [GOVERNANCE](https://github.com/ducp-protocol/spec/blob/main/GOVERNANCE.md).
