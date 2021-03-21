#[path = "protobufs"]
pub mod envoy {
    #[path = "."]
    pub mod config {
        #[path = "."]
        pub mod core {
            pub mod v3 {
                include!(concat!(env!("OUT_DIR"), "/envoy.config.core.v3.rs"));
            }
        }
    }

    #[path = "."]
    pub mod extensions {
        #[path = "."]
        pub mod common {
            #[path = "."]
            pub mod ratelimit {
                pub mod v3 {
                    include!(concat!(
                        env!("OUT_DIR"),
                        "/envoy.extensions.common.ratelimit.v3.rs"
                    ));
                }
            }
        }
    }

    #[path = "."]
    pub mod r#type {
        pub mod v3 {
            include!(concat!(env!("OUT_DIR"), "/envoy.r#type.v3.rs"));
        }
    }

    #[path = "."]
    pub mod service {
        #[path = "."]
        pub mod ratelimit {
            pub mod v3 {
                include!(concat!(env!("OUT_DIR"), "/envoy.service.ratelimit.v3.rs"));
            }
        }
    }
}
