use std::{
    cell::Cell,
    marker::PhantomData,
    pin::Pin,
    sync::atomic::{AtomicU64, Ordering},
    task::{Context, Poll},
};

#[derive(Clone, Copy)]
struct Data {
    value: *mut Option<()>,
    id: u64,
}

thread_local! {
    static DATA: Cell<Option<Data>> = const { Cell::new(None) };
}

pub(crate) fn yielder<T>() -> (YielderTx<T>, YielderRx<T>) {
    static ID: AtomicU64 = AtomicU64::new(0);

    let id = ID.fetch_add(1, Ordering::Relaxed);
    (YielderTx { id, _data: PhantomData }, YielderRx { id, _data: PhantomData })
}

pub struct YielderTx<T> {
    id: u64,
    _data: PhantomData<fn(T) -> T>,
}

pub(crate) struct YielderRx<T> {
    id: u64,
    _data: PhantomData<T>,
}

impl<T> YielderTx<T> {
    #[must_use]
    pub fn send(&self, value: T) -> Yield<'_, T> {
        Yield { value: Some(value), tx: self }
    }
}

impl<T> YielderRx<T> {
    pub(crate) fn enter(&mut self, value: &mut Option<T>) -> YielderRxScope<'_, T> {
        let data = Data { value: value as *mut _ as *mut Option<()>, id: self.id };
        YielderRxScope { _rx: self, prev: DATA.replace(Some(data)) }
    }
}

pub(crate) struct YielderRxScope<'a, T> {
    _rx: &'a mut YielderRx<T>,
    prev: Option<Data>,
}

impl<T> Drop for YielderRxScope<'_, T> {
    fn drop(&mut self) {
        DATA.set(self.prev);
    }
}

pub struct Yield<'a, T> {
    value: Option<T>,
    tx: &'a YielderTx<T>,
}

impl<'a, T> Future for Yield<'a, T>
where
    T: Unpin,
{
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let Some(value) = self.value.take() else { return Poll::Ready(()) };

        let Some(data) = DATA.get() else { panic!("not inside tx scope") };
        assert_eq!(self.tx.id, data.id, "wrong scope");

        let cell = unsafe { &mut *(data.value as *mut Option<T>) };

        if cell.is_none() {
            *cell = Some(value);
        } else {
            self.value = Some(value);
        }

        // Wake-driven combinators must re-poll this send on the next stream poll.
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::{StreamExt, stream::FuturesUnordered};
    use tokio::time::timeout;

    #[tokio::test]
    async fn sends_resume_in_futures_unordered() {
        for count in [1, 3] {
            let stream = crate::generator(async |tx| {
                let mut pending =
                    (1..=count).map(|value| tx.send(value)).collect::<FuturesUnordered<_>>();
                while pending.next().await.is_some() {}

                tx.send(count + 1).await;
            });

            let mut values = timeout(Duration::from_secs(1), stream.collect::<Vec<_>>())
                .await
                .expect("sends stalled inside FuturesUnordered");

            assert_eq!(values.last(), Some(&(count + 1)));
            values.sort_unstable();
            assert_eq!(values, (1..=count + 1).collect::<Vec<_>>());
        }
    }
}
