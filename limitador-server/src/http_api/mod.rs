// Deriving ApiV2Schema raises this Clippy warning. We can delete this once we
// upgrade to a version of paperclip that contains the fix
// https://github.com/wafflespeanut/paperclip/pull/275
#[allow(clippy::field_reassign_with_default)]
mod request_types;

pub use request_types::Limit as LimitVO;

pub mod server;
