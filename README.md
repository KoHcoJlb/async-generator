# async-generator

Asynchronous generators on stable Rust. Write an async body with `yield` and
consume it as a `futures::Stream`.

Requires Rust 1.88 or later. The library does not require a particular async
runtime; the examples use the executor provided by `futures`.

## Installation

```toml
[dependencies]
async-generator = "0.1"
futures = "0.3"
```

## Yielding values

```rust
# #[cfg(feature = "macros")]
# {
use async_generator::generator;
use futures::{executor::block_on, StreamExt};

let stream = generator!({
    for value in 1..=3 {
        yield value;
    }
});

assert_eq!(block_on(stream.collect::<Vec<_>>()), vec![1, 2, 3]);
# }
```

The body can use `.await`. Use `generator!(move { ... })` to move captured values
into the generator. Execution starts when the stream is polled, and resumes as
the consumer requests more items. When calling `.next().await` directly, pin the
stream first with `std::pin::pin!`.

## Fallible generators

`try_generator!` bodies return `Result<(), E>` and can use `?`. Each yielded value
becomes `Ok(value)`; a terminal error becomes one final `Err(error)` item.

```rust
# #[cfg(feature = "macros")]
# {
use async_generator::try_generator;
use futures::{executor::block_on, StreamExt};

let stream = try_generator!({
    for text in ["1", "2", "invalid"] {
        yield text.parse::<u32>()?;
    }
    Ok::<(), std::num::ParseIntError>(())
});

let values = block_on(stream.collect::<Vec<_>>());
assert_eq!(values.len(), 3);
assert_eq!(values[0], Ok(1));
assert_eq!(values[1], Ok(2));
assert!(values[2].is_err());
# }
```

If a poll both yields a value and completes the body, the yielded value is
returned before the terminal result. After completion and delivery of any
terminal error, subsequent polls return `None`.

## Without procedural macros

The closure-based `generator` and `try_generator` functions are always available.
Disable the default `macros` feature to omit the procedural-macro dependency:

```toml
async-generator = { version = "0.1", default-features = false }
```

```rust
use async_generator::generator;
use futures::{executor::block_on, StreamExt};

let stream = generator(async |tx| {
    tx.send(1).await;
    tx.send(2).await;
});

assert_eq!(block_on(stream.collect::<Vec<_>>()), vec![1, 2]);
```

## Constraints

- Yielded values must implement `Unpin`.
- The sender is a scoped handoff, not a channel. Await sends inside their
  generator; polling them in a spawned task or another generator panics.
  Combinators such as `join` and `FuturesUnordered` can be used inside the body.
- Use `yield ();` for a unit value; bare `yield;` is unsupported.
- Yields inside another macro's arguments are not rewritten.
- Attributes directly on a `yield` expression are not preserved. Put attributes
  such as `#[cfg(...)]` on an enclosing block instead.
- The macros currently require the dependency name `async-generator`; renaming
  it in `Cargo.toml` is unsupported.

## License

Licensed under either the MIT license or the Apache License, Version 2.0, at your
option.
