use crate::codec::EncodeBody;
use crate::codec::{CompressionEncoding, EnabledCompressionEncodings};
use crate::metadata::GRPC_CONTENT_TYPE;
use crate::{
    body::Body,
    client::GrpcService,
    codec::{Codec, Decoder, Streaming},
    request::SanitizeHeaders,
    Code, Request, Response, Status,
};
use http::{
    header::{HeaderValue, CONTENT_TYPE, TE},
    uri::{PathAndQuery, Uri},
};
use http_body::Body as HttpBody;
#[cfg(feature = "grpc_config")]
use std::sync::Arc;
use std::{fmt, future, pin::pin};
use tokio_stream::{Stream, StreamExt};

/// A gRPC client dispatcher.
///
/// This will wrap some inner [`GrpcService`] and will encode/decode
/// messages via the provided codec.
///
/// Each request method takes a [`Request`], a [`PathAndQuery`], and a
/// [`Codec`]. The request contains the message to send via the
/// [`Codec::encoder`]. The path determines the fully qualified path
/// that will be append to the outgoing uri. The path must follow
/// the conventions explained in the [gRPC protocol definition] under `Path →`. An
/// example of this path could look like `/greeter.Greeter/SayHello`.
///
/// [gRPC protocol definition]: https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md#requests
pub struct Grpc<T> {
    inner: T,
    #[cfg(feature = "grpc_config")]
    config: Arc<GrpcConfig>,
}

macro_rules! config_getters {
    {
        $(#[$attr:meta])*
        struct GrpcConfig {
            $(
                $(#[$doc:meta])*
                // This is to keep tests compiling and to avoid adding paste as dependency
                $field:ident = $getter:ident: $ty:ty,
            )+
        }
    } => {
        $(#[$attr])*
        struct GrpcConfig {
            $(
                $(#[$doc])*
                $field: $ty,
            )+
        }

        impl<T> Grpc<T> {
            $(
                #[inline(always)]
                fn $getter(&self) -> $ty {
                    #[cfg(feature = "grpc_config")]
                    {
                        self.config.$field.clone()
                    }

                    #[cfg(not(feature = "grpc_config"))]
                    {
                        <$ty>::default()
                    }
                }
            )*
        }
    };
}

config_getters! {
    #[allow(dead_code)]
    #[derive(Default, Clone, Debug)]
    struct GrpcConfig {
        origin = origin: Uri,
        /// Which compression encodings does the client accept?
        accept_compression_encodings = get_accept_compression_encodings: EnabledCompressionEncodings,
        /// The compression encoding that will be applied to requests.
        send_compression_encodings = get_send_compression_encodings: Option<CompressionEncoding>,
        /// Limits the maximum size of a decoded message.
        max_decoding_message_size = get_max_decoding_message_size: Option<usize>,
        /// Limits the maximum size of an encoded message.
        max_encoding_message_size = get_max_encoding_message_size: Option<usize>,
    }
}

impl<T> Grpc<T> {
    /// Creates a new gRPC client with the provided [`GrpcService`].
    pub fn new(inner: T) -> Self {
        #[cfg(feature = "grpc_config")]
        {
            Self::builder().build(inner)
        }

        #[cfg(not(feature = "grpc_config"))]
        {
            Self { inner }
        }
    }

    /// Creates a builder for configuring a [`Grpc`] client.
    ///
    /// This method is only available when the `grpc_config` feature is enabled.  
    /// It allows customization of options such as compression, message size limits, and the request origin URI.
    ///
    /// # Example
    ///
    /// ```rust
    /// use tonic::transport::Channel;
    /// use tonic::codec::CompressionEncoding;
    /// use tonic::client::Grpc;
    ///
    /// # async {
    /// let channel = Channel::builder("http://[::1]:50051".parse().unwrap())
    ///     .connect()
    ///     .await
    ///     .unwrap();
    ///
    /// let client = Grpc::<Channel>::builder()
    ///     .origin("http://example.com".parse().unwrap())
    ///     .send_compressed(CompressionEncoding::Gzip)
    ///     .accept_compressed(CompressionEncoding::Gzip)
    ///     .max_decoding_message_size(2 * 1024 * 1024)
    ///     .max_encoding_message_size(2 * 1024 * 1024)
    ///     .build(channel);
    /// # };
    /// ```
    #[cfg(feature = "grpc_config")]
    pub fn builder() -> GrpcBuilder {
        GrpcBuilder::default()
    }

    /// Creates a new gRPC client with the provided [`GrpcService`] and `Uri`.
    ///
    /// The provided Uri will use only the scheme and authority parts as the
    /// path_and_query portion will be set for each method.
    #[allow(unused_variables)]
    pub fn with_origin(inner: T, origin: Uri) -> Self {
        #[cfg(feature = "grpc_config")]
        {
            Self::builder().origin(origin).build(inner)
        }

        #[cfg(not(feature = "grpc_config"))]
        {
            Self::new(inner)
        }
    }

    /// Compress requests with the provided encoding.
    ///
    /// This clones the current config to avoid modifying shared state.
    /// Prefer using `GrpcBuilder::send_compressed` during configuration.
    ///
    /// # Note
    ///
    /// Using this method bypasses the shared configuration mechanism and
    /// allocates a new `GrpcConfig` internally.
    #[allow(unused_variables)]
    #[allow(unused_mut)]
    pub fn send_compressed(mut self, encoding: CompressionEncoding) -> Self {
        // FIXME: Removeall
        // This method and its kind shouldn't exist at all but it's harder to
        // refractor uses than to just have a no-op
        #[cfg(feature = "grpc_config")]
        {
            let mut config = (*self.config).clone();
            config.send_compression_encodings = Some(encoding);
            self.config = Arc::new(config);
        }
        self
    }

    /// Enable accepting compressed responses.
    ///
    /// This clones the current config to avoid modifying shared state.
    /// Prefer using `GrpcBuilder::accept_compressed` during configuration.
    ///
    /// # Note
    ///
    /// Using this method bypasses the shared configuration mechanism and
    /// allocates a new `GrpcConfig` internally.
    #[allow(unused_variables)]
    #[allow(unused_mut)]
    pub fn accept_compressed(mut self, encoding: CompressionEncoding) -> Self {
        #[cfg(feature = "grpc_config")]
        {
            let mut config = (*self.config).clone();
            config.accept_compression_encodings.enable(encoding);
            self.config = Arc::new(config);
        }
        self
    }

    /// Limits the maximum size of a decoded message.
    ///
    /// This clones the current config to avoid modifying shared state.
    /// Prefer using `GrpcBuilder::max_decoding_message_size` during configuration.
    #[allow(unused_variables)]
    #[allow(unused_mut)]
    pub fn max_decoding_message_size(mut self, limit: usize) -> Self {
        #[cfg(feature = "grpc_config")]
        {
            let mut config = (*self.config).clone();
            config.max_decoding_message_size = Some(limit);
            self.config = Arc::new(config);
        }
        self
    }

    /// Limits the maximum size of an encoded message.
    ///
    /// This clones the current config to avoid modifying shared state.
    /// Prefer using `GrpcBuilder::max_encoding_message_size` during configuration.
    #[allow(unused_variables)]
    #[allow(unused_mut)]
    pub fn max_encoding_message_size(mut self, limit: usize) -> Self {
        #[cfg(feature = "grpc_config")]
        {
            let mut config = (*self.config).clone();
            config.max_encoding_message_size = Some(limit);
            self.config = Arc::new(config);
        }
        self
    }

    /// Check if the inner [`GrpcService`] is able to accept a  new request.
    ///
    /// This will call [`GrpcService::poll_ready`] until it returns ready or
    /// an error. If this returns ready the inner [`GrpcService`] is ready to
    /// accept one more request.
    pub async fn ready(&mut self) -> Result<(), T::Error>
    where
        T: GrpcService<Body>,
    {
        future::poll_fn(|cx| self.inner.poll_ready(cx)).await
    }

    /// Send a single unary gRPC request.
    pub async fn unary<M1, M2, C>(
        &mut self,
        request: Request<M1>,
        path: PathAndQuery,
        codec: C,
    ) -> Result<Response<M2>, Status>
    where
        T: GrpcService<Body>,
        T::ResponseBody: HttpBody + Send + 'static,
        <T::ResponseBody as HttpBody>::Error: Into<crate::BoxError>,
        C: Codec<Encode = M1, Decode = M2>,
        M1: Send + Sync + 'static,
        M2: Send + Sync + 'static,
    {
        let request = request.map(|m| tokio_stream::once(m));
        self.client_streaming(request, path, codec).await
    }

    /// Send a client side streaming gRPC request.
    pub async fn client_streaming<S, M1, M2, C>(
        &mut self,
        request: Request<S>,
        path: PathAndQuery,
        codec: C,
    ) -> Result<Response<M2>, Status>
    where
        T: GrpcService<Body>,
        T::ResponseBody: HttpBody + Send + 'static,
        <T::ResponseBody as HttpBody>::Error: Into<crate::BoxError>,
        S: Stream<Item = M1> + Send + 'static,
        C: Codec<Encode = M1, Decode = M2>,
        M1: Send + Sync + 'static,
        M2: Send + Sync + 'static,
    {
        let (mut parts, body, extensions) =
            self.streaming(request, path, codec).await?.into_parts();

        let mut body = pin!(body);

        let message = body
            .try_next()
            .await
            .map_err(|mut status| {
                status.metadata_mut().merge(parts.clone());
                status
            })?
            .ok_or_else(|| Status::internal("Missing response message."))?;

        if let Some(trailers) = body.trailers().await? {
            parts.merge(trailers);
        }

        Ok(Response::from_parts(parts, message, extensions))
    }

    /// Send a server side streaming gRPC request.
    pub async fn server_streaming<M1, M2, C>(
        &mut self,
        request: Request<M1>,
        path: PathAndQuery,
        codec: C,
    ) -> Result<Response<Streaming<M2>>, Status>
    where
        T: GrpcService<Body>,
        T::ResponseBody: HttpBody + Send + 'static,
        <T::ResponseBody as HttpBody>::Error: Into<crate::BoxError>,
        C: Codec<Encode = M1, Decode = M2>,
        M1: Send + Sync + 'static,
        M2: Send + Sync + 'static,
    {
        let request = request.map(|m| tokio_stream::once(m));
        self.streaming(request, path, codec).await
    }

    /// Send a bi-directional streaming gRPC request.
    pub async fn streaming<S, M1, M2, C>(
        &mut self,
        request: Request<S>,
        path: PathAndQuery,
        mut codec: C,
    ) -> Result<Response<Streaming<M2>>, Status>
    where
        T: GrpcService<Body>,
        T::ResponseBody: HttpBody + Send + 'static,
        <T::ResponseBody as HttpBody>::Error: Into<crate::BoxError>,
        S: Stream<Item = M1> + Send + 'static,
        C: Codec<Encode = M1, Decode = M2>,
        M1: Send + Sync + 'static,
        M2: Send + Sync + 'static,
    {
        let request = request
            .map(|s| {
                EncodeBody::new_client(
                    codec.encoder(),
                    s.map(Ok),
                    self.get_send_compression_encodings(),
                    self.get_max_encoding_message_size(),
                )
            })
            .map(Body::new);

        let request = self.prepare_request(request, path);

        let response = self
            .inner
            .call(request)
            .await
            .map_err(Status::from_error_generic)?;

        let decoder = codec.decoder();

        self.create_response(decoder, response)
    }

    // Keeping this code in a separate function from Self::streaming lets functions that return the
    // same output share the generated binary code
    fn create_response<M2>(
        &self,
        decoder: impl Decoder<Item = M2, Error = Status> + Send + 'static,
        response: http::Response<T::ResponseBody>,
    ) -> Result<Response<Streaming<M2>>, Status>
    where
        T: GrpcService<Body>,
        T::ResponseBody: HttpBody + Send + 'static,
        <T::ResponseBody as HttpBody>::Error: Into<crate::BoxError>,
    {
        let encoding = CompressionEncoding::from_encoding_header(
            response.headers(),
            self.get_accept_compression_encodings(),
        )?;

        let status_code = response.status();
        let trailers_only_status = Status::from_header_map(response.headers());

        // We do not need to check for trailers if the `grpc-status` header is present
        // with a valid code.
        let expect_additional_trailers = if let Some(status) = trailers_only_status {
            if status.code() != Code::Ok {
                return Err(status);
            }

            false
        } else {
            true
        };

        let response = response.map(|body| {
            if expect_additional_trailers {
                Streaming::new_response(
                    decoder,
                    body,
                    status_code,
                    encoding,
                    self.get_max_decoding_message_size(),
                )
            } else {
                Streaming::new_empty(decoder, body)
            }
        });

        Ok(Response::from_http(response))
    }

    fn prepare_request(&self, request: Request<Body>, path: PathAndQuery) -> http::Request<Body> {
        let mut parts = self.origin().into_parts();

        match &parts.path_and_query {
            Some(pnq) if pnq != "/" => {
                parts.path_and_query = Some(
                    format!("{}{}", pnq.path(), path)
                        .parse()
                        .expect("must form valid path_and_query"),
                )
            }
            _ => {
                parts.path_and_query = Some(path);
            }
        }

        let uri = Uri::from_parts(parts).expect("path_and_query only is valid Uri");

        let mut request = request.into_http(
            uri,
            http::Method::POST,
            http::Version::HTTP_2,
            SanitizeHeaders::Yes,
        );

        // Add the gRPC related HTTP headers
        request
            .headers_mut()
            .insert(TE, HeaderValue::from_static("trailers"));

        // Set the content type
        request
            .headers_mut()
            .insert(CONTENT_TYPE, GRPC_CONTENT_TYPE);

        #[cfg(any(feature = "gzip", feature = "deflate", feature = "zstd"))]
        if let Some(encoding) = self.get_send_compression_encodings() {
            request.headers_mut().insert(
                crate::codec::compression::ENCODING_HEADER,
                encoding.into_header_value(),
            );
        }

        if let Some(header_value) = self
            .get_accept_compression_encodings()
            .into_accept_encoding_header_value()
        {
            request.headers_mut().insert(
                crate::codec::compression::ACCEPT_ENCODING_HEADER,
                header_value,
            );
        }

        request
    }
}

impl<T: Clone> Clone for Grpc<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            #[cfg(feature = "grpc_config")]
            config: self.config.clone(),
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for Grpc<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        #[cfg(feature = "grpc_config")]
        {
            f.debug_struct("Grpc")
                .field("inner", &self.inner)
                .field("origin", &self.config.origin)
                .field(
                    "compression_encoding",
                    &self.config.send_compression_encodings,
                )
                .field(
                    "accept_compression_encodings",
                    &self.config.accept_compression_encodings,
                )
                .field(
                    "max_decoding_message_size",
                    &self.config.max_decoding_message_size,
                )
                .field(
                    "max_encoding_message_size",
                    &self.config.max_encoding_message_size,
                )
                .finish()
        }

        #[cfg(not(feature = "grpc_config"))]
        {
            f.debug_struct("Grpc").field("inner", &self.inner).finish()
        }
    }
}

#[cfg(feature = "grpc_config")]
#[derive(Default, Debug)]
pub struct GrpcBuilder {
    config: GrpcConfig,
}

#[cfg(feature = "grpc_config")]
impl GrpcBuilder {
    /// Optionally specify `Uri`.
    ///
    /// The provided Uri will use only the scheme and authority parts as the
    /// path_and_query portion will be set for each method.
    pub fn origin(mut self, origin: Uri) -> Self {
        self.config.origin = origin;
        self
    }

    /// Compress requests with the provided encoding.
    ///
    /// Requires the server to accept the specified encoding, otherwise it might return an error.
    ///
    /// # Example
    ///
    /// The most common way of using this is through a client generated by tonic-build:
    ///
    /// ```rust
    /// use tonic::transport::Channel;
    /// # enum CompressionEncoding { Gzip }
    /// # struct TestClient<T>(T);
    /// # impl<T> TestClient<T> {
    /// #     fn new(channel: T) -> Self { Self(channel) }
    /// #     fn send_compressed(self, _: CompressionEncoding) -> Self { self }
    /// # }
    ///
    /// # async {
    /// let channel = Channel::builder("127.0.0.1:3000".parse().unwrap())
    ///     .connect()
    ///     .await
    ///     .unwrap();
    ///
    /// let client = TestClient::new(channel).send_compressed(CompressionEncoding::Gzip);
    /// # };
    /// ```
    pub fn send_compressed(mut self, encoding: CompressionEncoding) -> Self {
        self.config.send_compression_encodings = Some(encoding);
        self
    }

    /// Enable accepting compressed responses.
    ///
    /// Requires the server to also support sending compressed responses.
    ///
    /// # Example
    ///
    /// The most common way of using this is through a client generated by tonic-build:
    ///
    /// ```rust
    /// use tonic::transport::Channel;
    /// # enum CompressionEncoding { Gzip }
    /// # struct TestClient<T>(T);
    /// # impl<T> TestClient<T> {
    /// #     fn new(channel: T) -> Self { Self(channel) }
    /// #     fn accept_compressed(self, _: CompressionEncoding) -> Self { self }
    /// # }
    ///
    /// # async {
    /// let channel = Channel::builder("127.0.0.1:3000".parse().unwrap())
    ///     .connect()
    ///     .await
    ///     .unwrap();
    ///
    /// let client = TestClient::new(channel).accept_compressed(CompressionEncoding::Gzip);
    /// # };
    /// ```
    pub fn accept_compressed(mut self, encoding: CompressionEncoding) -> Self {
        self.config.accept_compression_encodings.enable(encoding);
        self
    }

    /// Limits the maximum size of a decoded message.
    ///
    /// # Example
    ///
    /// The most common way of using this is through a client generated by tonic-build:
    ///
    /// ```rust
    /// use tonic::transport::Channel;
    /// # struct TestClient<T>(T);
    /// # impl<T> TestClient<T> {
    /// #     fn new(channel: T) -> Self { Self(channel) }
    /// #     fn max_decoding_message_size(self, _: usize) -> Self { self }
    /// # }
    ///
    /// # async {
    /// let channel = Channel::builder("127.0.0.1:3000".parse().unwrap())
    ///     .connect()
    ///     .await
    ///     .unwrap();
    ///
    /// // Set the limit to 2MB, Defaults to 4MB.
    /// let limit = 2 * 1024 * 1024;
    /// let client = TestClient::new(channel).max_decoding_message_size(limit);
    /// # };
    /// ```
    pub fn max_decoding_message_size(mut self, limit: usize) -> Self {
        self.config.max_decoding_message_size = Some(limit);
        self
    }

    /// Limits the maximum size of an encoded message.
    ///
    /// # Example
    ///
    /// The most common way of using this is through a client generated by tonic-build:
    ///
    /// ```rust
    /// use tonic::transport::Channel;
    /// # struct TestClient<T>(T);
    /// # impl<T> TestClient<T> {
    /// #     fn new(channel: T) -> Self { Self(channel) }
    /// #     fn max_encoding_message_size(self, _: usize) -> Self { self }
    /// # }
    ///
    /// # async {
    /// let channel = Channel::builder("127.0.0.1:3000".parse().unwrap())
    ///     .connect()
    ///     .await
    ///     .unwrap();
    ///
    /// // Set the limit to 2MB, Defaults to `usize::MAX`.
    /// let limit = 2 * 1024 * 1024;
    /// let client = TestClient::new(channel).max_encoding_message_size(limit);
    /// # };
    /// ```
    pub fn max_encoding_message_size(mut self, limit: usize) -> Self {
        self.config.max_encoding_message_size = Some(limit);
        self
    }

    /// Creates a new gRPC client with the provided [`GrpcService`]
    pub fn build<T>(self, inner: T) -> Grpc<T> {
        Grpc {
            inner,
            config: self.config.into(),
        }
    }
}
