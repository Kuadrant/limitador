#[allow(unknown_lints)]
pub mod envoy {
    pub mod config {
        pub mod core {
            // clippy will barf on protobuff generated code for enum variants in
            // v3::socket_option::SocketState, so allow this lint
            #[allow(
                clippy::enum_variant_names,
                clippy::derive_partial_eq_without_eq,
                clippy::doc_lazy_continuation
            )]
            pub mod v3 {
                tonic::include_proto!("envoy.config.core.v3");
            }
        }
    }

    pub mod extensions {
        pub mod common {
            pub mod ratelimit {
                #[allow(clippy::derive_partial_eq_without_eq)]
                pub mod v3 {
                    tonic::include_proto!("envoy.extensions.common.ratelimit.v3");
                }
            }
        }
    }

    pub mod r#type {
        #[allow(clippy::derive_partial_eq_without_eq)]
        pub mod v3 {
            tonic::include_proto!("envoy.r#type.v3");
        }
    }

    pub mod service {
        pub mod ratelimit {
            #[allow(clippy::derive_partial_eq_without_eq, clippy::doc_lazy_continuation)]
            pub mod v3 {
                tonic::include_proto!("envoy.service.ratelimit.v3");
            }
        }
    }
}

pub mod xds {
    pub mod core {
        #[allow(clippy::derive_partial_eq_without_eq)]
        pub mod v3 {
            tonic::include_proto!("xds.core.v3");
        }
    }
}
