/// Main message for a rate limit request. The rate limit service is designed to be fully generic
/// in the sense that it can operate on arbitrary hierarchical key/value pairs. The loaded
/// configuration will parse the request and find the most specific limit to apply. In addition,
/// a RateLimitRequest can contain multiple "descriptors" to limit on. When multiple descriptors
/// are provided, the server will limit on *ALL* of them and return an OVER_LIMIT response if any
/// of them are over limit. This enables more complex application level rate limiting scenarios
/// if desired.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct RateLimitRequest {
    /// All rate limit requests must specify a domain. This enables the configuration to be per
    /// application without fear of overlap. E.g., "envoy".
    #[prost(string, tag = "1")]
    pub domain: ::prost::alloc::string::String,
    /// All rate limit requests must specify at least one RateLimitDescriptor. Each descriptor is
    /// processed by the service (see below). If any of the descriptors are over limit, the entire
    /// request is considered to be over limit.
    #[prost(message, repeated, tag = "2")]
    pub descriptors: ::prost::alloc::vec::Vec<
        super::super::super::extensions::common::ratelimit::v3::RateLimitDescriptor,
    >,
    /// Rate limit requests can optionally specify the number of hits a request adds to the matched
    /// limit. If the value is not set in the message, a request increases the matched limit by 1.
    #[prost(uint32, tag = "3")]
    pub hits_addend: u32,
}
/// A response from a ShouldRateLimit call.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct RateLimitResponse {
    /// The overall response code which takes into account all of the descriptors that were passed
    /// in the RateLimitRequest message.
    #[prost(enumeration = "rate_limit_response::Code", tag = "1")]
    pub overall_code: i32,
    /// A list of DescriptorStatus messages which matches the length of the descriptor list passed
    /// in the RateLimitRequest. This can be used by the caller to determine which individual
    /// descriptors failed and/or what the currently configured limits are for all of them.
    #[prost(message, repeated, tag = "2")]
    pub statuses: ::prost::alloc::vec::Vec<rate_limit_response::DescriptorStatus>,
    /// A list of headers to add to the response
    #[prost(message, repeated, tag = "3")]
    pub response_headers_to_add:
        ::prost::alloc::vec::Vec<super::super::super::config::core::v3::HeaderValue>,
    /// A list of headers to add to the request when forwarded
    #[prost(message, repeated, tag = "4")]
    pub request_headers_to_add:
        ::prost::alloc::vec::Vec<super::super::super::config::core::v3::HeaderValue>,
}
/// Nested message and enum types in `RateLimitResponse`.
pub mod rate_limit_response {
    /// Defines an actual rate limit in terms of requests per unit of time and the unit itself.
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct RateLimit {
        /// A name or description of this limit.
        #[prost(string, tag = "3")]
        pub name: ::prost::alloc::string::String,
        /// The number of requests per unit of time.
        #[prost(uint32, tag = "1")]
        pub requests_per_unit: u32,
        /// The unit of time.
        #[prost(enumeration = "rate_limit::Unit", tag = "2")]
        pub unit: i32,
    }
    /// Nested message and enum types in `RateLimit`.
    pub mod rate_limit {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
        #[repr(i32)]
        pub enum Unit {
            /// The time unit is not known.
            Unknown = 0,
            /// The time unit representing a second.
            Second = 1,
            /// The time unit representing a minute.
            Minute = 2,
            /// The time unit representing an hour.
            Hour = 3,
            /// The time unit representing a day.
            Day = 4,
        }
    }
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct DescriptorStatus {
        /// The response code for an individual descriptor.
        #[prost(enumeration = "Code", tag = "1")]
        pub code: i32,
        /// The current limit as configured by the server. Useful for debugging, etc.
        #[prost(message, optional, tag = "2")]
        pub current_limit: ::core::option::Option<RateLimit>,
        /// The limit remaining in the current time unit.
        #[prost(uint32, tag = "3")]
        pub limit_remaining: u32,
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
    #[repr(i32)]
    pub enum Code {
        /// The response code is not known.
        Unknown = 0,
        /// The response code to notify that the number of requests are under limit.
        Ok = 1,
        /// The response code to notify that the number of requests are over limit.
        OverLimit = 2,
    }
}
#[doc = r" Generated client implementations."]
pub mod rate_limit_service_client {
    #![allow(unused_variables, dead_code, missing_docs)]
    use tonic::codegen::*;
    pub struct RateLimitServiceClient<T> {
        inner: tonic::client::Grpc<T>,
    }
    impl RateLimitServiceClient<tonic::transport::Channel> {
        #[doc = r" Attempt to create a new client by connecting to a given endpoint."]
        pub async fn connect<D>(dst: D) -> Result<Self, tonic::transport::Error>
        where
            D: std::convert::TryInto<tonic::transport::Endpoint>,
            D::Error: Into<StdError>,
        {
            let conn = tonic::transport::Endpoint::new(dst)?.connect().await?;
            Ok(Self::new(conn))
        }
    }
    impl<T> RateLimitServiceClient<T>
    where
        T: tonic::client::GrpcService<tonic::body::BoxBody>,
        T::ResponseBody: Body + HttpBody + Send + 'static,
        T::Error: Into<StdError>,
        <T::ResponseBody as HttpBody>::Error: Into<StdError> + Send,
    {
        pub fn new(inner: T) -> Self {
            let inner = tonic::client::Grpc::new(inner);
            Self { inner }
        }
        pub fn with_interceptor(inner: T, interceptor: impl Into<tonic::Interceptor>) -> Self {
            let inner = tonic::client::Grpc::with_interceptor(inner, interceptor);
            Self { inner }
        }
        #[doc = " Determine whether rate limiting should take place."]
        pub async fn should_rate_limit(
            &mut self,
            request: impl tonic::IntoRequest<super::RateLimitRequest>,
        ) -> Result<tonic::Response<super::RateLimitResponse>, tonic::Status> {
            self.inner.ready().await.map_err(|e| {
                tonic::Status::new(
                    tonic::Code::Unknown,
                    format!("Service was not ready: {}", e.into()),
                )
            })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/envoy.service.ratelimit.v3.RateLimitService/ShouldRateLimit",
            );
            self.inner.unary(request.into_request(), path, codec).await
        }
    }
    impl<T: Clone> Clone for RateLimitServiceClient<T> {
        fn clone(&self) -> Self {
            Self {
                inner: self.inner.clone(),
            }
        }
    }
    impl<T> std::fmt::Debug for RateLimitServiceClient<T> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "RateLimitServiceClient {{ ... }}")
        }
    }
}
#[doc = r" Generated server implementations."]
pub mod rate_limit_service_server {
    #![allow(unused_variables, dead_code, missing_docs)]
    use tonic::codegen::*;
    #[doc = "Generated trait containing gRPC methods that should be implemented for use with RateLimitServiceServer."]
    #[async_trait]
    pub trait RateLimitService: Send + Sync + 'static {
        #[doc = " Determine whether rate limiting should take place."]
        async fn should_rate_limit(
            &self,
            request: tonic::Request<super::RateLimitRequest>,
        ) -> Result<tonic::Response<super::RateLimitResponse>, tonic::Status>;
    }
    #[derive(Debug)]
    pub struct RateLimitServiceServer<T: RateLimitService> {
        inner: _Inner<T>,
    }
    struct _Inner<T>(Arc<T>, Option<tonic::Interceptor>);
    impl<T: RateLimitService> RateLimitServiceServer<T> {
        pub fn new(inner: T) -> Self {
            let inner = Arc::new(inner);
            let inner = _Inner(inner, None);
            Self { inner }
        }
        pub fn with_interceptor(inner: T, interceptor: impl Into<tonic::Interceptor>) -> Self {
            let inner = Arc::new(inner);
            let inner = _Inner(inner, Some(interceptor.into()));
            Self { inner }
        }
    }
    impl<T, B> Service<http::Request<B>> for RateLimitServiceServer<T>
    where
        T: RateLimitService,
        B: HttpBody + Send + Sync + 'static,
        B::Error: Into<StdError> + Send + 'static,
    {
        type Response = http::Response<tonic::body::BoxBody>;
        type Error = Never;
        type Future = BoxFuture<Self::Response, Self::Error>;
        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
        fn call(&mut self, req: http::Request<B>) -> Self::Future {
            let inner = self.inner.clone();
            match req.uri().path() {
                "/envoy.service.ratelimit.v3.RateLimitService/ShouldRateLimit" => {
                    #[allow(non_camel_case_types)]
                    struct ShouldRateLimitSvc<T: RateLimitService>(pub Arc<T>);
                    impl<T: RateLimitService> tonic::server::UnaryService<super::RateLimitRequest>
                        for ShouldRateLimitSvc<T>
                    {
                        type Response = super::RateLimitResponse;
                        type Future = BoxFuture<tonic::Response<Self::Response>, tonic::Status>;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::RateLimitRequest>,
                        ) -> Self::Future {
                            let inner = self.0.clone();
                            let fut = async move { (*inner).should_rate_limit(request).await };
                            Box::pin(fut)
                        }
                    }
                    let inner = self.inner.clone();
                    let fut = async move {
                        let interceptor = inner.1.clone();
                        let inner = inner.0;
                        let method = ShouldRateLimitSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = if let Some(interceptor) = interceptor {
                            tonic::server::Grpc::with_interceptor(codec, interceptor)
                        } else {
                            tonic::server::Grpc::new(codec)
                        };
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                _ => Box::pin(async move {
                    Ok(http::Response::builder()
                        .status(200)
                        .header("grpc-status", "12")
                        .header("content-type", "application/grpc")
                        .body(tonic::body::BoxBody::empty())
                        .unwrap())
                }),
            }
        }
    }
    impl<T: RateLimitService> Clone for RateLimitServiceServer<T> {
        fn clone(&self) -> Self {
            let inner = self.inner.clone();
            Self { inner }
        }
    }
    impl<T: RateLimitService> Clone for _Inner<T> {
        fn clone(&self) -> Self {
            Self(self.0.clone(), self.1.clone())
        }
    }
    impl<T: std::fmt::Debug> std::fmt::Debug for _Inner<T> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{:?}", self.0)
        }
    }
    impl<T: RateLimitService> tonic::transport::NamedService for RateLimitServiceServer<T> {
        const NAME: &'static str = "envoy.service.ratelimit.v3.RateLimitService";
    }
}
