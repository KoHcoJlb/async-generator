use std::{
    any::type_name,
    fmt,
    fmt::Formatter,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use futures::Stream;
use pin_project::pin_project;

use crate::yielder::{YielderRx, YielderTx, yielder};

pub trait Mode<Y, R>: Sized {
    type Item;

    fn on_yield(value: Y) -> Self::Item;
    fn on_exit(value: R) -> Option<Self::Item>;
    fn debug<G: Future>(_: &GeneratorImpl<G, Y, Self>, fmt: &mut Formatter<'_>) -> fmt::Result;
}

impl<Y> Mode<Y, ()> for () {
    type Item = Y;

    #[inline]
    fn on_yield(value: Y) -> Self::Item {
        value
    }

    #[inline]
    fn on_exit(_: ()) -> Option<Self::Item> {
        None
    }

    fn debug<G: Future>(_: &GeneratorImpl<G, Y, Self>, fmt: &mut Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("Generator").field("fut", &type_name::<G>()).finish()
    }
}

pub struct Try;

impl<Y, E> Mode<Y, Result<(), E>> for Try {
    type Item = Result<Y, E>;

    #[inline]
    fn on_yield(value: Y) -> Self::Item {
        Ok(value)
    }

    #[inline]
    fn on_exit(value: Result<(), E>) -> Option<Self::Item> {
        match value {
            Ok(()) => None,
            Err(err) => Some(Err(err)),
        }
    }

    fn debug<G: Future>(
        _generator: &GeneratorImpl<G, Y, Self>, fmt: &mut Formatter<'_>,
    ) -> fmt::Result {
        fmt.debug_struct("TryGenerator").field("fut", &type_name::<G>()).finish()
    }
}

/// A stream of values yielded by an async body returning `()`.
pub type Generator<G, Y> = GeneratorImpl<G, Y, ()>;

/// A stream of yielded `Ok` values followed by an optional terminal `Err`.
pub type TryGenerator<G, Y> = GeneratorImpl<G, Y, Try>;

#[pin_project]
pub struct GeneratorImpl<G: Future, Y, M> {
    #[pin]
    generator: G,
    rx: YielderRx<Y>,
    done: bool,
    exit: Option<G::Output>,
    _mode: PhantomData<fn(M) -> M>,
}

impl<G, Y, M, R> fmt::Debug for GeneratorImpl<G, Y, M>
where
    G: Future<Output = R>,
    M: Mode<Y, R>,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        M::debug(self, f)
    }
}

impl<G, Y, M, R> GeneratorImpl<G, Y, M>
where
    G: Future<Output = R>,
    M: Mode<Y, R>,
{
    fn new<F>(func: F) -> Self
    where
        F: FnOnce(YielderTx<Y>) -> G,
    {
        let (tx, rx) = yielder();
        Self { generator: func(tx), rx, done: false, exit: None, _mode: PhantomData }
    }
}

impl<G, Y, M, R> Stream for GeneratorImpl<G, Y, M>
where
    G: Future<Output = R>,
    M: Mode<Y, R>,
{
    type Item = M::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let proj = self.project();

        let mut value = None;
        if !*proj.done {
            let res = {
                let _scope = proj.rx.enter(&mut value);
                proj.generator.poll(cx)
            };

            if let Poll::Ready(res) = res {
                *proj.done = true;
                *proj.exit = Some(res);
            }
        }

        // A yield takes precedence over an exit produced by the same poll.
        match value {
            Some(value) => Poll::Ready(Some(M::on_yield(value))),
            None if *proj.done => Poll::Ready(proj.exit.take().and_then(M::on_exit)),
            None => Poll::Pending,
        }
    }
}

/// Creates a stream from a closure that receives a sender and returns a future.
///
/// Await `tx.send(value)` inside the future to yield a value. Values must be
/// `Unpin`, and send futures must be polled within this generator rather than in
/// independently spawned tasks. Pin the stream before calling `next().await`.
///
/// This function is available without the `macros` feature.
#[inline]
pub fn generator<G, Y>(func: impl FnOnce(YielderTx<Y>) -> G) -> Generator<G, Y>
where
    G: Future<Output = ()>,
{
    GeneratorImpl::<G, Y, ()>::new(func)
}

/// Creates a fallible stream from an async body returning `Result<(), E>`.
///
/// Await `tx.send(value)` inside the body to yield `Ok(value)`. A terminal error
/// is emitted once, after any value yielded during the same poll. After the
/// terminal result has been consumed, subsequent polls return `None`.
///
/// The sender has the same constraints as [`generator`]. This function is
/// available without the `macros` feature.
#[inline]
pub fn try_generator<G, Y, E>(func: impl FnOnce(YielderTx<Y>) -> G) -> TryGenerator<G, Y>
where
    G: Future<Output = Result<(), E>>,
{
    GeneratorImpl::<G, Y, Try>::new(func)
}

#[cfg(test)]
mod tests {
    use std::{pin::pin, task::Waker};

    use futures::future::{poll_fn, ready, select};

    use super::*;

    #[test]
    fn completed_send_survives_generator_completion() {
        let stream = generator(async |tx| {
            let mut send = pin!(tx.send(1));
            poll_fn(|cx| {
                assert!(send.as_mut().poll(cx).is_pending());
                send.as_mut().poll(cx)
            })
            .await;
        });
        let mut stream = pin!(stream);
        let mut cx = Context::from_waker(Waker::noop());

        assert_eq!(stream.as_mut().poll_next(&mut cx), Poll::Ready(Some(1)));
        assert_eq!(stream.as_mut().poll_next(&mut cx), Poll::Ready(None));
        assert_eq!(stream.as_mut().poll_next(&mut cx), Poll::Ready(None));
    }

    #[test]
    fn try_generator_preserves_yield_before_exit() {
        for exit in [Ok(()), Err("end")] {
            for should_yield in [false, true] {
                let stream = try_generator(async |tx| {
                    if should_yield {
                        select(tx.send(1), ready(())).await;
                    }
                    exit
                });
                let mut stream = pin!(stream);
                let mut cx = Context::from_waker(Waker::noop());

                if should_yield {
                    assert_eq!(stream.as_mut().poll_next(&mut cx), Poll::Ready(Some(Ok(1))));
                }
                assert_eq!(stream.as_mut().poll_next(&mut cx), Poll::Ready(exit.err().map(Err)));
                assert_eq!(stream.as_mut().poll_next(&mut cx), Poll::Ready(None));
                assert_eq!(stream.as_mut().poll_next(&mut cx), Poll::Ready(None));
            }
        }
    }
}
