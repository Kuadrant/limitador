// [#protodoc-title: Common rate limit components]

/// A RateLimitDescriptor is a list of hierarchical entries that are used by the service to
/// determine the final rate limit key and overall allowed limit. Here are some examples of how
/// they might be used for the domain "envoy".
///
/// .. code-block:: cpp
///
///   ["authenticated": "false"], ["remote_address": "10.0.0.1"]
///
/// What it does: Limits all unauthenticated traffic for the IP address 10.0.0.1. The
/// configuration supplies a default limit for the *remote_address* key. If there is a desire to
/// raise the limit for 10.0.0.1 or block it entirely it can be specified directly in the
/// configuration.
///
/// .. code-block:: cpp
///
///   ["authenticated": "false"], ["path": "/foo/bar"]
///
/// What it does: Limits all unauthenticated traffic globally for a specific path (or prefix if
/// configured that way in the service).
///
/// .. code-block:: cpp
///
///   ["authenticated": "false"], ["path": "/foo/bar"], ["remote_address": "10.0.0.1"]
///
/// What it does: Limits unauthenticated traffic to a specific path for a specific IP address.
/// Like (1) we can raise/block specific IP addresses if we want with an override configuration.
///
/// .. code-block:: cpp
///
///   ["authenticated": "true"], ["client_id": "foo"]
///
/// What it does: Limits all traffic for an authenticated client "foo"
///
/// .. code-block:: cpp
///
///   ["authenticated": "true"], ["client_id": "foo"], ["path": "/foo/bar"]
///
/// What it does: Limits traffic to a specific path for an authenticated client "foo"
///
/// The idea behind the API is that (1)/(2)/(3) and (4)/(5) can be sent in 1 request if desired.
/// This enables building complex application scenarios with a generic backend.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct RateLimitDescriptor {
    /// Descriptor entries.
    #[prost(message, repeated, tag = "1")]
    pub entries: ::std::vec::Vec<rate_limit_descriptor::Entry>,
}
pub mod rate_limit_descriptor {
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Entry {
        /// Descriptor key.
        #[prost(string, tag = "1")]
        pub key: std::string::String,
        /// Descriptor value.
        #[prost(string, tag = "2")]
        pub value: std::string::String,
    }
}
