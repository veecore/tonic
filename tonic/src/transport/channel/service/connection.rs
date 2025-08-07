use super::{
    modifier_fn_default, AddOrigin, AsyncService, ModifierFn, Reconnect, SharedExec, UserAgent,
};
use crate::{
    body::Body,
    transport::{
        channel::{BoxFuture, RawRequest},
        service::GrpcTimeout,
        Endpoint,
    },
};
use http::{Request, Response, Uri};
use hyper::rt;
use hyper::{client::conn::http2::Builder, rt::Executor};
use hyper_util::rt::TokioTimer;
use std::{
    fmt, future::Future, sync::Arc, task::{Context, Poll}
};
use tower::load::Load;
use tower::{
    layer::Layer,
    limit::{concurrency::ConcurrencyLimitLayer, rate::RateLimitLayer},
    util::BoxService,
    ServiceBuilder, ServiceExt,
};
use tower_service::Service;

pub(crate) struct Connection {
    inner: BoxService<Request<Body>, Response<Body>, crate::BoxError>,
}

impl Connection {
    fn new<C>(connector: C, endpoint: Endpoint, is_lazy: bool) -> Self
    where
        C: Service<Uri> + Send + 'static,
        C::Error: Into<crate::BoxError> + Send,
        C::Future: Send,
        C::Response: rt::Read + rt::Write + Unpin + Send + 'static,
    {
        Self::new_with_modifier_fn(connector, endpoint, is_lazy, modifier_fn_default())
            .expect("Scheme and authority is set")
    }

    fn new_with_modifier_fn<C, M, MF>(
        connector: C,
        endpoint: Endpoint,
        is_lazy: bool,
        custom_modifier: M,
    ) -> Result<Self, crate::BoxError>
    where
        C: Service<Uri> + Send + 'static,
        C::Error: Into<crate::BoxError> + Send,
        C::Future: Send,
        C::Response: rt::Read + rt::Write + Unpin + Send + 'static,
        M: FnOnce(RawRequest<Body>) -> MF + Send + 'static + Clone,
        MF: Future<Output = RawRequest<Body>> + Send + 'static,
    {
        let mut settings: Builder<SharedExec> = Builder::new(endpoint.executor.clone())
            .initial_stream_window_size(endpoint.init_stream_window_size)
            .initial_connection_window_size(endpoint.init_connection_window_size)
            .keep_alive_interval(endpoint.http2_keep_alive_interval)
            .timer(TokioTimer::new())
            .clone();

        if let Some(val) = endpoint.http2_keep_alive_timeout {
            settings.keep_alive_timeout(val);
        }

        if let Some(val) = endpoint.http2_keep_alive_while_idle {
            settings.keep_alive_while_idle(val);
        }

        if let Some(val) = endpoint.http2_adaptive_window {
            settings.adaptive_window(val);
        }

        if let Some(val) = endpoint.http2_max_header_list_size {
            settings.max_header_list_size(val);
        }

        let endpoint_origin = endpoint.uri().clone();
        let add_origin = AddOrigin::new(endpoint.origin.unwrap_or(endpoint_origin.clone()))?;

        let stack = ServiceBuilder::new()
            .option_layer(endpoint.concurrency_limit.map(ConcurrencyLimitLayer::new))
            .option_layer(endpoint.rate_limit.map(|(l, d)| RateLimitLayer::new(l, d)))
            .layer(ModifierFn::new_layer_once(add_origin.to_request_fn()))
            .layer(ModifierFn::new_layer_once(
                UserAgent::new(endpoint.user_agent).to_request_fn(),
            ))
            .layer(ModifierFn::new_layer_once(custom_modifier))
            .layer_fn(|s| GrpcTimeout::new(s, endpoint.timeout))
            .into_inner();

        let make_service = MakeSendRequestService::new(connector, endpoint.executor, settings);

        let conn = Reconnect::new(make_service, endpoint_origin, is_lazy);

        Ok(Self {
            inner: BoxService::new(stack.layer(conn)),
        })
    }

    // pub(crate) async fn connect<C>(
    //     connector: C,
    //     endpoint: Endpoint,
    // ) -> Result<Self, crate::BoxError>
    // where
    //     C: Service<Uri> + Send + 'static,
    //     C::Error: Into<crate::BoxError> + Send,
    //     C::Future: Unpin + Send,
    //     C::Response: rt::Read + rt::Write + Unpin + Send + 'static,
    // {
    //     Self::new(connector, endpoint, false).ready_oneshot().await
    // }

    pub(crate) async fn connect<C, M , MF>(
        connector: C,
        endpoint: Endpoint,
        modifier_fn: M,
    ) -> Result<Self, crate::BoxError>
    where
        C: Service<Uri> + Send + 'static,
        C::Error: Into<crate::BoxError> + Send,
        C::Future: Unpin + Send,
        C::Response: rt::Read + rt::Write + Unpin + Send + 'static,
        M: FnOnce(RawRequest<Body>) -> MF + Send + 'static + Clone,
        MF: Future<Output = RawRequest<Body>> + Send + 'static,
    {
        Self::new_with_modifier_fn(connector, endpoint, false, modifier_fn)?
            .ready_oneshot()
            .await
    }

    pub(crate) fn lazy<C>(connector: C, endpoint: Endpoint) -> Self
    where
        C: Service<Uri> + Send + 'static,
        C::Error: Into<crate::BoxError> + Send,
        C::Future: Send,
        C::Response: rt::Read + rt::Write + Unpin + Send + 'static,
    {
        Self::new(connector, endpoint, true)
    }
}

impl Service<Request<Body>> for Connection {
    type Response = Response<Body>;
    type Error = crate::BoxError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Service::poll_ready(&mut self.inner, cx).map_err(Into::into)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        self.inner.call(req)
    }
}

impl Load for Connection {
    type Metric = usize;

    fn load(&self) -> Self::Metric {
        0
    }
}

impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connection").finish()
    }
}

struct SendRequest {
    inner: hyper::client::conn::http2::SendRequest<Body>,
}

impl From<hyper::client::conn::http2::SendRequest<Body>> for SendRequest {
    fn from(inner: hyper::client::conn::http2::SendRequest<Body>) -> Self {
        Self { inner }
    }
}

impl tower::Service<Request<Body>> for SendRequest {
    type Response = Response<Body>;
    type Error = crate::BoxError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        self.async_call(async move {req})
    }
}

impl AsyncService<Request<Body>> for SendRequest {
    #[inline(always)]
    fn async_call(&mut self, req: impl Future<Output = Request<Body>> + Send + 'static) -> Self::Future {
        // SendRequest is just 16bytes and clone is cheap since it's shared
        // Besides, we get rid of the many Box::pinning by doing this so it seems
        // justified to clone it. Of course, this is not solving the problem but
        // patching it.... we wouldn't have done this if it were any bigger.
        let mut send_request = self.inner.clone();

        Box::pin(async move {
            send_request
                .send_request(req.await)
                .await
                .map_err(Into::into)
                .map(|res| res.map(Body::new))
        })
    }
}

struct MakeSendRequestService<C> {
    connector: C,
    executor: SharedExec,
    settings: Arc<Builder<SharedExec>>,
}

impl<C> MakeSendRequestService<C> {
    fn new(connector: C, executor: SharedExec, settings: Builder<SharedExec>) -> Self {
        Self {
            connector,
            executor,
            settings: settings.into(),
        }
    }
}

impl<C> tower::Service<Uri> for MakeSendRequestService<C>
where
    C: Service<Uri> + Send + 'static,
    C::Error: Into<crate::BoxError> + Send,
    C::Future: Send,
    C::Response: rt::Read + rt::Write + Unpin + Send,
{
    type Response = SendRequest;
    type Error = crate::BoxError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.connector.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, req: Uri) -> Self::Future {
        let fut = self.connector.call(req);
        let builder = self.settings.clone();
        let executor = self.executor.clone();

        Box::pin(async move {
            let io = fut.await.map_err(Into::into)?;
            let (send_request, conn) = builder.handshake(io).await?;

            // FIXME: Do we have to Box pin even here?
            Executor::<BoxFuture<'static, ()>>::execute(
                &executor,
                Box::pin(async move {
                    if let Err(e) = conn.await {
                        tracing::debug!("connection task error: {:?}", e);
                    }
                }) as _,
            );

            Ok(SendRequest::from(send_request))
        })
    }
}
