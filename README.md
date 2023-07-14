[![CI](https://github.com/tlsnotary/mpz/actions/workflows/rust.yml/badge.svg)](https://github.com/tlsnotary/mpz/actions)

# Warning

The 2PC protocols in this library are designed so that at the end of execution they fully reveal all the secrets of party A and leak 1-bit information about the secrets of party B.

Thus, this protocol should only be used in certain very exotic scenarios. Please make sure you fully understand these ramifications before using it.

# MPZ

Multi Party computation made eaZy in Rust

MPC crates for the development of [TLSNotary](https://github.com/tlsnotary/tlsn)

## ⚠️ Notice

This project is currently under active development and should not be used in production. Expect bugs and regular major breaking changes.

## License
All crates in this repository are licensed under either of

- [Apache License, Version 2.0](http://www.apache.org/licenses/LICENSE-2.0)
- [MIT license](http://opensource.org/licenses/MIT)

at your option.

## Overview

Home of multi-party computation libraries:

  - oblivious transfer: Core building block used a lot in our codebase.
  - garbling: We use several variants of garbled circuit executions in our codebase
    (DEAP, Dual-Ex, ZK)
  - circuits: code to build circuits, with some basic circuit components
    available.
  - share-conversion: supports converting between additive and multiplicative
    shares for performing finite-field arithmetic in 2PC.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.

See [CONTRIBUTING.md](CONTRIBUTING.md).
