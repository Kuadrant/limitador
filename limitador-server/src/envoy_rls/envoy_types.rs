pub mod envoy {
    pub mod config {
        pub mod core {
            pub mod v3 {
                include!("protobufs/envoy.config.core.v3.rs");
            }
        }
    }

    pub mod extensions {
        pub mod common {
            pub mod ratelimit {
                pub mod v3 {
                    include!("protobufs/envoy.extensions.common.ratelimit.v3.rs");
                }
            }
        }
    }

    pub mod r#type {
        include!("protobufs/envoy.r#type.rs");

        pub mod v3 {
            include!("protobufs/envoy.r#type.v3.rs");
        }
    }

    pub mod service {
        pub mod ratelimit {
            pub mod v3 {
                include!("protobufs/envoy.service.ratelimit.v3.rs");
            }
        }
    }
}
