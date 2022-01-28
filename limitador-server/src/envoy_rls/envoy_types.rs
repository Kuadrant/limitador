#[path = "protobufs"]
pub mod envoy {
    #[path = "."]
    pub mod config {
        #[path = "."]
        pub mod core {
            #[path = "envoy.config.core.v3.rs"]
            // clippy will barf on protobuff generated code for enum variants in
            // v3::socket_option::SocketState, so allow this lint
            #[allow(clippy::enum_variant_names)]
            pub mod v3;
        }
    }

    #[path = "."]
    pub mod extensions {
        #[path = "."]
        pub mod common {
            #[path = "."]
            pub mod ratelimit {
                #[path = "envoy.extensions.common.ratelimit.v3.rs"]
                pub mod v3;
            }
        }
    }

    #[path = "."]
    pub mod r#type {
        #[path = "envoy.r#type.v3.rs"]
        pub mod v3;
    }

    #[path = "."]
    pub mod service {
        #[path = "."]
        pub mod ratelimit {
            #[path = "envoy.service.ratelimit.v3.rs"]
            pub mod v3;
        }
    }
}

#[path = "protobufs"]
pub mod xds {
    #[path = "."]
    pub mod core {
        #[path = "xds.core.v3.rs"]
        pub mod v3;
    }
}
