use std::{cell::RefCell, future::Future};

use tower::{Layer, Service};

mod modifier_fn;
pub(crate) use self::modifier_fn::default as modifier_fn_default;
use self::modifier_fn::ModifierFn;

mod request_modifiers;
use self::request_modifiers::*;

mod reconnect;
use self::reconnect::Reconnect;

mod connection;
pub(super) use self::connection::Connection;

mod discover;
pub use self::discover::Change;
pub(super) use self::discover::DynamicServiceStream;

mod io;
use self::io::BoxedIo;

mod connector;
pub(crate) use self::connector::Connector;

mod executor;
pub(super) use self::executor::{Executor, SharedExec};

#[cfg(feature = "_tls-any")]
mod tls;
#[cfg(feature = "_tls-any")]
pub(super) use self::tls::TlsConnector;

/// A [`tower::Layer`] implementation that wraps a `FnOnce` service constructor.
///
/// This type allows you to provide a function that consumes its input service
/// exactly **once** and returns a transformed service.
///
/// # ⚠️ Safety Warning
///
/// `LayerFnOnce` uses **unsafe memory tricks** to move the closure out of a
/// shared reference (`&self`) exactly once. After the first call to `layer()`,
/// the internal function is considered dropped — accessing it again is **undefined behavior**
/// and will likely result in a **segmentation fault** or worse.
///
/// This is *intentional* to allow Tower's `Layer` trait to work ergonomically with `FnOnce`.
///
/// ## Do not:
///
/// - Call `layer()` more than once.
/// - Clone or reuse the `LayerFnOnce` after it's been used.
/// - Wrap shared state in `LayerFnOnce` without ensuring one-time use.
///
/// ## Do:
///
/// - Use `LayerFnOnce::new` to build middleware inline without needing a full struct.
/// - Use it when you are 100% certain `.layer(...)` is called **once**, such as:
///     - During server startup
///     - In integration tests
///
/// # Example
///
/// ```rust
/// # use tower::{Layer, Service};
/// # use std::task::{Poll, Context};
/// # use std::pin::Pin;
///
/// struct MySvc;
/// impl<S> Service<S> for MySvc {
///     type Response = ();
///     type Error = ();
///     type Future = std::future::Ready<Result<(), ()>>;
///
///     fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
///         Poll::Ready(Ok(()))
///     }
///
///     fn call(&mut self, _: S) -> Self::Future {
///         std::future::ready(Ok(()))
///     }
/// }
///
/// let layer = LayerFnOnce::new(|svc: MySvc| {
///     // Transform the service here
///     svc
/// });
///
/// let svc = layer.layer(MySvc); // ✅ Safe if called exactly once
/// ```
#[derive(Clone, Debug)]
pub(crate) struct LayerFnOnce<F> {
    f: RefCell<Option<F>>,
}

impl<F> LayerFnOnce<F> {
    pub(crate) fn new(f: F) -> Self {
        Self {
            f: RefCell::new(Some(f)),
        }
    }
}

impl<F, S, Out> Layer<S> for LayerFnOnce<F>
where
    F: FnOnce(S) -> Out,
{
    type Service = Out;

    fn layer(&self, inner: S) -> Self::Service {
        let f = self
            .f
            .borrow_mut()
            .take()
            .expect("LayerFnOnce used more than once");
        f(inner)
    }
}

/*Musn't necessarily be Servie but to reduce typing*/
// More like ServicePassAlong than AsyncService
// This exists to reduce heap-allocation of futures...
// we do this... then at the root, we box one giant future instead of intermediate
// light ones. Boxing of these intermediate light ones was necessary since impl Trait
// is unstable. So in essence, this trait helps bypass impl Trait restriction.
//
// This bypasses none of tower's life restrictions (even though I wish it did)
// It will fail to solve many cases... e.g. when input's output(the future's output)
// is needed to build self's future
pub(crate) trait AsyncService<T>: Service<T> {
    fn async_call(&mut self, input: impl Future<Output = T> + Send + 'static) -> Self::Future;
}
