# SigmaOS-boot-manager

Boot manager component for SigmaOS.

## Status

This repository is a component scaffold. Its current behavior is limited to the code in [`src/lib.rs`](src/lib.rs); the registry description is intended scope, not a claim that the end-user feature is implemented.

## Build

Requires a stable Rust toolchain. Check the library with:

```sh
cargo check --lib
```

## Design approach

SigmaOS components aim to keep responsibilities explicit and integrations reviewable. This follows the small, focused component boundaries used by projects such as Omarchy. Changes should describe implemented behavior and prerequisites clearly, and should avoid claiming system integration until that integration exists.

## License

Apache-2.0. See [`LICENSE`](LICENSE).
