# Design and Standards 📃

## Support

### Compiler

All crates must support the latest Rust stable compiler version. mpz has a focus on usability, and alienating users who require stable support would be the opposite of that. Nightly features can be gated behind feature flags, but a stable configuration must always be available.

### Architecture

Crates should support the big build targets:
- `x86_64`
- `aarch64`
- `armv7`
- `wasm32-unknown-unknown`

## Dependencies

New dependencies should always receive consideration prior to adding them. Ask yourself:

- Is it maintained?
- Is it heavy, eg negatively affects build time?
- Does it employ good coding practices, eg no sloppy unsafe usage
- Architecture support?
- What alternatives are there?

### Reinventing the wheel

Avoid reinventing the wheel as much as possible. Every line of code written is a line of code that needs to be reviewed, tested, and maintained. Before implementing some functionality, search for well-adopted crates which already implement it. The bias should be towards code-reuse, even if you think you can squeeze out a 5% improvement over some other implementation. Open a PR for them instead.

This does not mean that we should avoid re-implementing something if we can achieve an order-of-magnitude greater performance in a way which is likely not to be accepted by another crate.

**RustCrypto**

mpz heavily utilizes crates provided by `RustCrypto`. Their implementations and traits should be preferred over re-implementing it ourselves. Eg. before implementing a primitive for the 1000th time, check if `RustCrypto` has it.

## Modularity 📦

### Crate structure

Avoid monolithic crates, and remember that a nest of feature flags is not modularity. The decision to split up a crate
is a matter of discretion, but generally crates should not include multiple different classes of functionality.

### Interfaces

mpz strives to provide clean abstractions and generic code, and this means good interfaces (traits). Before composing functionalities together by coupling concrete types, evaluate whether there is an existing trait instead, or introduce one otherwise.

### Core vs IO

A protocol which requires communication should be implemented such that the core functionality is independent of the IO. This approach has some upfront cost, but provides many benefits including better separation of concerns, cleaner and more comprehensive testing, and conduciveness to [strong typing](#message-types).

This is typically realized by separating code into two crates: `mycrate-core` and `mycrate`.

Also see the [async topic](#async-).

### Transport agnostic

Along with [our commitment to strong message typing](#message-types), our code must be *transport agnostic*. This means we do not couple our protocol implementations to any particular transport such as `TCP`. This ensures maximum flexibility for our users, makes testing easier, and naturally supports concurrency via multiplexing.

### Private-by-default

Related to modularity, focus should be on minimizing the surface area of an API. This means limiting the use of `pub` and `pub(crate)` as much as possible, except for intentional visibility as part of a coherent API.

## Typing 🛡️

### Message types

Messages communicated over the wire should be clearly defined, ie strongly typed. A protocol's implementation should not be coupled to concerns regarding serialization. Serde makes our lives easier for achieving this, while remaining agnostic of the _serialization format_. This also means that a low-level protocol implementation should never depend on the `Async(Write/Read)` traits, rather on the `Sink/Stream` traits provided by the `futures` crate.

### Type-states

The use of the [type state pattern](https://cliffle.com/blog/rust-typestate/) is strongly encouraged. The benefits of using type-states are numerous, particularly in crypto protocols, as it eliminates the potential for a variety of bugs caused by unexpected states or simply prevents misuse.

Often type-states can negatively affect code composability, as it can cause a combinatorial explosion of states. This can be alleviated by re-introducing polymorphism using enums. This does remove some of the benefits of type-states, however it preserves the underlying type safety of the _implementation_, ie an invalid state is still unrepresentable, but errors can occur at runtime.

## Async ☵

mpz heavily utilizes the asynchronous programming features provided by Rust. However, core crates should never contain asynchronous code, and this separation is quite natural due to [our other practices](#core-vs-io).

### Blocking code

"Long" blocking code should never be present within an async function, as that defeats the entire purpose. Instead, blocking code is delegated to a worker thread, typically using a `rayon` thread pool in combination with async memory-channels to make it awaitable.

Synchronous mutexes can still be used when it is certain they won't cause deadlocks, eg the lock is only ever held for a short duration and release does not depend on other concurrent branches.

### Executor Agnostic

All code in mpz should be *executor agnostic*. This means no coupling to a particular executor/runtime, eg `tokio`'s runtime. This may take some getting used to, but it ensures maximum compatibility for dependent projects. See [this blog post](https://blog.yoshuawuyts.com/tree-structured-concurrency/) on structured concurrency to better understand how to avoid the traps that the `Spawn` functionality poses, and how remaining executor agnostic avoids it entirely by only utilizing `Future` primitives.