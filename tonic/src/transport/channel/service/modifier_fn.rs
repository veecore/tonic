use crate::transport::channel::RawRequest;
use std::{
    future::{Future, Ready},
    task::{Context, Poll},
};
use tower_service::Service;

use super::{AsyncService, LayerFnOnce};

#[derive(Debug)]
pub(crate) struct ModifierFn<T, M> {
    inner: T,
    modifier_fn: M,
}

impl<T, M> ModifierFn<T, M> {
    pub(crate) fn new(inner: T, modifier_fn: M) -> Self {
        Self { inner, modifier_fn }
    }

    pub(crate) fn new_layer_once(modifier_fn: M) -> LayerFnOnce<impl FnOnce(T) -> Self> {
        LayerFnOnce::new(|inner| Self::new(inner, modifier_fn))
    }
}

pub(crate) fn default(
) -> impl FnOnce(RawRequest<crate::body::Body>) -> Ready<RawRequest<crate::body::Body>> + Clone {
    |r| std::future::ready(r)
}

impl<T, M, Body, MF> Service<RawRequest<Body>> for ModifierFn<T, M>
where
    T: AsyncService<RawRequest<Body>>,
    M: FnOnce(RawRequest<Body>) -> MF + Send + Clone + 'static,
    MF: Future<Output = RawRequest<Body>> + Send + 'static,
    Body: Send + 'static,
{
    type Response = T::Response;
    type Error = T::Error;
    type Future = T::Future;

    #[inline(always)]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: RawRequest<Body>) -> Self::Future {
        self.async_call(async move { req })
    }
}

impl<T, M, Body, MF> AsyncService<RawRequest<Body>> for ModifierFn<T, M>
where
    T: AsyncService<RawRequest<Body>>,
    M: FnOnce(RawRequest<Body>) -> MF + Send + Clone + 'static,
    MF: Future<Output = RawRequest<Body>> + Send + 'static,
    Body: Send + 'static,
{
    #[inline(always)]
    fn async_call(
        &mut self,
        input: impl Future<Output = RawRequest<Body>> + Send + 'static,
    ) -> Self::Future {
        // This part made us require FnOnce + Clone over FnMut... we might
        // bypass this by placing the custom modifier last(in layer) knowing the library
        // modifiers aren't really async... but this would cause issues once we
        // have an async one.... also, it seems more appropriate to have the custom
        // modifier first or before the timeout layer...
        // I don't like this but that's also what Axum does... Also the values captured
        // by all the library modifiers end-up getting cloned... same as my use case
        // For some peculiar case, arcing might be required.

        // sobs
        let modifier_fn = self.modifier_fn.clone();
        self.inner
            .async_call(async move { modifier_fn(input.await).await })
    }
}
