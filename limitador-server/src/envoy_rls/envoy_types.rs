#[path = "protobufs"]
pub mod envoy {
    #[path = "."]
    pub mod config {
        #[path = "."]
        pub mod core {
            #[path = "envoy.config.core.v3.rs"]
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
