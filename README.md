[![CI](https://github.com/privacy-scaling-explorations/mpz/actions/workflows/rust.yml/badge.svg)](https://github.com/privacy-scaling-explorations/mpz/actions)

<p align="center">
    <img src="./mpz-banner.png" width=1280 />
</p>

# mpz

mpz is a collection of multi-party computation libraries written in Rust 🦀.

This project strives to provide safe, performant, modular and portable MPC software with a focus on usability.

See [our design doc](./DESIGN.md) for information on design choices, standards and project structure.

## ⚠️ Notice

This project is currently under active development and should not be used in production. Expect bugs and regular major breaking changes. Use at your own risk.

## Crates

**Core**
  - `mpz-core` - Assortment of low-level primitives.
  - `matrix-transpose` - Bit-wise matrix transposition.
  - `clmul` - Carry-less multiplication

**Circuits**
  - `mpz-circuits` - Boolean circuit DSL
  - `mpz-circuits-macros` - Proc-macros for `mpz-circuits`

**Oblivious Transfer**
  - `mpz-ot` - High-level async APIs
  - `mpz-ot-core` - Low-level types for OT, and core implementations of OT protocols.
  
**Garbled Circuits**
  - `mpz-garble` - High-level APIs for boolean garbled circuit protocols, including VM abstractions.
  - `mpz-garble-core` - Low-level types for boolean half-gate garbling algorithm.

**Share Conversion**
  - `mpz-share-conversion` - High-level APIs for Multiplicative-to-Additive and Additive-to-Multiplicative share conversion protocols for a variety of fields.
  - `mpz-share-conversion-core` - Low-level types for share conversion protocols.

## License
All crates in this repository are licensed under either of

- [Apache License, Version 2.0](http://www.apache.org/licenses/LICENSE-2.0)
- [MIT license](http://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.

See [CONTRIBUTING.md](CONTRIBUTING.md).

## Contributors

- [TLSNotary](https://github.com/tlsnotary)


### Pronounciation

mpz is pronounced "em-peasy".