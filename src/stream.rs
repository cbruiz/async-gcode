
#[cfg(feature = "future-stream")]
pub use core::marker::Unpin as UnpinTrait;

#[cfg(feature = "future-stream")]
pub use futures::stream::Stream as ByteStream;

#[cfg(feature = "future-stream")]
pub use futures::stream::TryStream as TryByteStream;

#[cfg(not(feature = "future-stream"))]
pub trait UnpinTrait {}

#[cfg(not(feature = "future-stream"))]
#[allow(async_fn_in_trait)]
pub trait ByteStream  {
    /// Values yielded by the stream.
    type Item;
    async fn next(&mut self) -> Option<Self::Item>;

    // Used to notify inner stream when there is a reset due timeout or something
    async fn recovery_check(&mut self);
}

#[cfg(not(feature = "future-stream"))]
impl <S>UnpinTrait for S where S: ByteStream {}

#[cfg(not(feature = "future-stream"))]
#[allow(async_fn_in_trait)]
pub trait TryByteStream: ByteStream {
    /// The type of successful values yielded by this future
    type Ok;

    /// The type of failures yielded by this future
    type Error;
    async fn try_next(&mut self) -> Option<Result<Self::Ok, Self::Error>>;
}

#[cfg(not(feature = "future-stream"))]
impl<S, T, E> TryByteStream for S
where
    S: ?Sized + ByteStream<Item = Result<T, E>> + UnpinTrait,
{
    type Ok = T;
    type Error = E;

    async fn try_next(&mut self) -> Option<Result<Self::Ok, Self::Error>> {
        self.next().await
    }
}

pub(crate) trait MyTryStreamExt: TryByteStream {
    #[cfg(not(feature = "parse-checksum"))]
    fn push_backable(self) -> pushback::PushBack<Self>
    where
        Self: Sized,
    {
        pushback::PushBack::new(self)
    }

    #[cfg(feature = "parse-checksum")]
    fn xor_summed_push_backable(
        self,
        initial_sum: Self::Ok,
    ) -> xorsum_pushback::XorSumPushBack<Self>
    where
        Self: Sized,
        Self::Ok: Copy + core::ops::BitXorAssign,
    {
        xorsum_pushback::XorSumPushBack::new(self, initial_sum)
    }
}
impl<T: ?Sized> MyTryStreamExt for T where T: TryByteStream {}

pub(crate) trait PushBackable {
    type Item;
    fn push_back(&mut self, v: Self::Item) -> Option<Self::Item>;
}

#[cfg(not(feature = "parse-checksum"))]
pub(crate) mod pushback {

    use super::{ByteStream, TryByteStream};
    use super::PushBackable;
    #[cfg(feature = "future-stream")]
    use core::pin::Pin;
    #[cfg(feature = "future-stream")]
    use futures::task::Poll;
    #[cfg(feature = "future-stream")]
    use futures::task::Context;

    #[cfg(feature = "future-stream")]
    pin_project_lite::pin_project! {
        pub(crate) struct PushBack<S: TryByteStream> {
            #[pin]
            stream: S,
            val: Option<S::Ok>,
        }
    }


    #[cfg(not(feature = "future-stream"))]
    pub(crate) struct PushBack<S: TryByteStream> {
        stream: S,
        val: Option<S::Ok>,
    }

    impl<S: TryByteStream> PushBack<S> {
        pub fn new(stream: S) -> Self {
            Self { stream, val: None }
        }

        /// The wrapped stream. A pushed-back item, if any, is dropped.
        pub fn into_inner(self) -> S {
            self.stream
        }
    }
    impl<S> PushBackable for PushBack<S>
    where
        S: TryByteStream,
    {
        type Item = S::Ok;
        fn push_back(&mut self, v: S::Ok) -> Option<S::Ok> {
            self.val.replace(v)
        }
    }

    #[cfg(feature = "future-stream")]
    impl<S> ByteStream for PushBack<S>
    where
        S: TryByteStream,
    {
        type Item = Result<S::Ok, S::Error>;
        fn poll_next(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            let this = self.project();
            if let Some(v) = this.val.take() {
                Poll::Ready(Some(Ok(v)))
            } else {
                this.stream.try_poll_next(ctx)
            }
        }
    }

    #[cfg(not(feature = "future-stream"))]
    impl<S> ByteStream for PushBack<S>
    where
        S: TryByteStream,
    {
        type Item = Result<S::Ok, S::Error>;

        async fn next(&mut self) -> Option<Self::Item> {
            if let Some(v) = self.val.take() {
                Some(Ok(v))
            }
            else {
                self.stream.try_next().await
            }
        }

        async fn recovery_check(&mut self) {
            self.stream.recovery_check().await
        }
    }

    #[cfg(test)]
    mod test {
        use super::{PushBack, PushBackable};
        use crate::test_utils::{collect, stream_of, take};

        fn data() -> Vec<Result<u8, core::convert::Infallible>> {
            [1, 2, 4, 8, 16, 32, 64, 128].iter().copied().map(Ok).collect()
        }

        #[test]
        fn the_stream_works() {
            let data = data();
            let mut strm = PushBack::new(stream_of(data.iter().copied()));
            assert_eq!(collect(&mut strm), data);
        }

        #[test]
        fn pushbacked_value_come_out_first() {
            let data = data();
            let mut strm = PushBack::new(stream_of(data.iter().copied()));
            assert_eq!(take(&mut strm, 4), &data[..=3]);

            strm.push_back(0xCC);

            assert_eq!(take(&mut strm, 4), [Ok(0xCC), Ok(16), Ok(32), Ok(64)]);
        }
    }
}

#[cfg(feature = "parse-checksum")]
pub(crate) mod xorsum_pushback {

    use super::PushBackable;
    use super::{ByteStream, TryByteStream};

    #[cfg(feature = "future-stream")]
    use core::pin::Pin;
    #[cfg(feature = "future-stream")]
    use core::task::{Context, Poll};

    #[cfg(feature = "future-stream")]
    pin_project_lite::pin_project! {
        pub(crate) struct XorSumPushBack<S: TryByteStream> {
            #[pin]
            stream: S,
            head: Option<S::Ok>,
            sum: S::Ok
        }
    }

    #[cfg(not(feature = "future-stream"))]
    pub(crate) struct XorSumPushBack<S: TryByteStream> {
        stream: S,
        head: Option<S::Ok>,
        sum: S::Ok
    }


    impl<S> XorSumPushBack<S>
    where
        S: TryByteStream,
        S::Ok: core::ops::BitXorAssign + Copy,
    {
        pub fn new(stream: S, initial_sum: S::Ok) -> Self {
            Self {
                stream,
                head: None,
                sum: initial_sum,
            }
        }

        /// The wrapped stream. A pushed-back item, if any, is dropped.
        pub fn into_inner(self) -> S {
            self.stream
        }

        pub fn reset_sum(&mut self, initial_sum: S::Ok) {
            self.sum = initial_sum;
        }

        pub fn sum(&self) -> S::Ok {
            self.sum
        }
    }

    impl<S> PushBackable for XorSumPushBack<S>
    where
        S: TryByteStream,
        S::Ok: core::ops::BitXorAssign + Copy,
    {
        type Item = S::Ok;
        fn push_back(&mut self, head: S::Ok) -> Option<S::Ok> {
            self.sum ^= head;
            self.head.replace(head)
        }
    }

    #[cfg(feature = "future-stream")]
    impl<S> ByteStream for XorSumPushBack<S>
    where
        S: TryByteStream,
        S::Ok: core::ops::BitXorAssign + Copy,
    {
        type Item = Result<S::Ok, S::Error>;

        fn poll_next(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            let this = self.project();
            let item = if let Some(item) = this.head.take() {
                item
            } else {
                match this.stream.try_poll_next(ctx) {
                    Poll::Ready(Some(Ok(item))) => item,
                    other => return other,
                }
            };
            *this.sum ^= item;
            Poll::Ready(Some(Ok(item)))
        }
    }


    #[cfg(not(feature = "future-stream"))]
    impl<S> ByteStream for XorSumPushBack<S>
    where
        S: TryByteStream,
        S::Ok: core::ops::BitXorAssign + Copy,
    {
        type Item = Result<S::Ok, S::Error>;

        async fn next(&mut self) -> Option<Self::Item> {
            let item = if let Some(item) = self.head.take() {
                item
            } else {
                match self.stream.try_next().await {
                    Some(Ok(item)) => item,
                    other => return other,
                }
            };
            self.sum ^= item;
            Some(Ok(item))
        }

        #[cfg(not(feature = "future-stream"))]
        async fn recovery_check(&mut self) {
            self.stream.recovery_check().await;
        }
    }

    #[cfg(test)]
    mod test {
        use super::{PushBackable, XorSumPushBack};
        use crate::test_utils::{collect, stream_of, take};

        fn data() -> Vec<Result<u8, core::convert::Infallible>> {
            [1, 2, 4, 8, 16, 32, 64, 128].iter().copied().map(Ok).collect()
        }

        #[test]
        fn the_stream_works_and_the_xorsum_is_computed() {
            let data = data();
            let mut strm = XorSumPushBack::new(stream_of(data.iter().copied()), 0);
            assert_eq!(collect(&mut strm), data);
            assert_eq!(strm.sum(), 0xFF);
        }

        #[test]
        fn the_xorsum_can_be_reset() {
            let data = data();
            let mut strm = XorSumPushBack::new(stream_of(data.iter().copied()), 0);
            assert_eq!(take(&mut strm, 4), &data[..=3]);
            assert_eq!(strm.sum(), 0x0F);

            strm.reset_sum(0x30);

            assert_eq!(collect(&mut strm), &data[4..]);
            assert_eq!(strm.sum(), 0xC0);
        }

        #[test]
        fn pushing_back_updates_the_xorsum() {
            let data = data();
            let mut strm = XorSumPushBack::new(stream_of(data.iter().copied()), 0);
            assert_eq!(take(&mut strm, 4), &data[..=3]);
            assert_eq!(strm.sum(), 0x0F);

            strm.push_back(0xCC);

            assert_eq!(strm.sum(), 0xC3);
            assert_eq!(take(&mut strm, 4), [Ok(0xCC), Ok(16), Ok(32), Ok(64)]);
        }
    }
}
