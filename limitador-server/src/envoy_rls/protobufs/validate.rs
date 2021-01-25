/// FieldRules encapsulates the rules for each type of field. Depending on the
/// field, the correct set should be used to ensure proper validations.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FieldRules {
    #[prost(message, optional, tag = "17")]
    pub message: ::std::option::Option<MessageRules>,
    #[prost(
        oneof = "field_rules::Type",
        tags = "1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 19, 20, 21, 22"
    )]
    pub r#type: ::std::option::Option<field_rules::Type>,
}
pub mod field_rules {
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum Type {
        /// Scalar Field Types
        #[prost(message, tag = "1")]
        Float(super::FloatRules),
        #[prost(message, tag = "2")]
        Double(super::DoubleRules),
        #[prost(message, tag = "3")]
        Int32(super::Int32Rules),
        #[prost(message, tag = "4")]
        Int64(super::Int64Rules),
        #[prost(message, tag = "5")]
        Uint32(super::UInt32Rules),
        #[prost(message, tag = "6")]
        Uint64(super::UInt64Rules),
        #[prost(message, tag = "7")]
        Sint32(super::SInt32Rules),
        #[prost(message, tag = "8")]
        Sint64(super::SInt64Rules),
        #[prost(message, tag = "9")]
        Fixed32(super::Fixed32Rules),
        #[prost(message, tag = "10")]
        Fixed64(super::Fixed64Rules),
        #[prost(message, tag = "11")]
        Sfixed32(super::SFixed32Rules),
        #[prost(message, tag = "12")]
        Sfixed64(super::SFixed64Rules),
        #[prost(message, tag = "13")]
        Bool(super::BoolRules),
        #[prost(message, tag = "14")]
        String(super::StringRules),
        #[prost(message, tag = "15")]
        Bytes(super::BytesRules),
        /// Complex Field Types
        #[prost(message, tag = "16")]
        Enum(super::EnumRules),
        #[prost(message, tag = "18")]
        Repeated(Box<super::RepeatedRules>),
        #[prost(message, tag = "19")]
        Map(Box<super::MapRules>),
        /// Well-Known Field Types
        #[prost(message, tag = "20")]
        Any(super::AnyRules),
        #[prost(message, tag = "21")]
        Duration(super::DurationRules),
        #[prost(message, tag = "22")]
        Timestamp(super::TimestampRules),
    }
}
/// FloatRules describes the constraints applied to `float` values
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FloatRules {
    /// Const specifies that this field must be exactly the specified value
    #[prost(float, optional, tag = "1")]
    pub r#const: ::std::option::Option<f32>,
    /// Lt specifies that this field must be less than the specified value,
    /// exclusive
    #[prost(float, optional, tag = "2")]
    pub lt: ::std::option::Option<f32>,
    /// Lte specifies that this field must be less than or equal to the
    /// specified value, inclusive
    #[prost(float, optional, tag = "3")]
    pub lte: ::std::option::Option<f32>,
    /// Gt specifies that this field must be greater than the specified value,
    /// exclusive. If the value of Gt is larger than a specified Lt or Lte, the
    /// range is reversed.
    #[prost(float, optional, tag = "4")]
    pub gt: ::std::option::Option<f32>,
    /// Gte specifies that this field must be greater than or equal to the
    /// specified value, inclusive. If the value of Gte is larger than a
    /// specified Lt or Lte, the range is reversed.
    #[prost(float, optional, tag = "5")]
    pub gte: ::std::option::Option<f32>,
    /// In specifies that this field must be equal to one of the specified
    /// values
    #[prost(float, repeated, packed = "false", tag = "6")]
    pub r#in: ::std::vec::Vec<f32>,
    /// NotIn specifies that this field cannot be equal to one of the specified
    /// values
    #[prost(float, repeated, packed = "false", tag = "7")]
    pub not_in: ::std::vec::Vec<f32>,
}
/// DoubleRules describes the constraints applied to `double` values
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct DoubleRules {
    /// Const specifies that this field must be exactly the specified value
    #[prost(double, optional, tag = "1")]
    pub r#const: ::std::option::Option<f64>,
    /// Lt specifies that this field must be less than the specified value,
    /// exclusive
    #[prost(double, optional, tag = "2")]
    pub lt: ::std::option::Option<f64>,
    /// Lte specifies that this field must be less than or equal to the
    /// specified value, inclusive
    #[prost(double, optional, tag = "3")]
    pub lte: ::std::option::Option<f64>,
    /// Gt specifies that this field must be greater than the specified value,
    /// exclusive. If the value of Gt is larger than a specified Lt or Lte, the
    /// range is reversed.
    #[prost(double, optional, tag = "4")]
    pub gt: ::std::option::Option<f64>,
    /// Gte specifies that this field must be greater than or equal to the
    /// specified value, inclusive. If the value of Gte is larger than a
    /// specified Lt or Lte, the range is reversed.
    #[prost(double, optional, tag = "5")]
    pub gte: ::std::option::Option<f64>,
    /// In specifies that this field must be equal to one of the specified
    /// values
    #[prost(double, repeated, packed = "false", tag = "6")]
    pub r#in: ::std::vec::Vec<f64>,
    /// NotIn specifies that this field cannot be equal to one of the specified
    /// values
    #[prost(double, repeated, packed = "false", tag = "7")]
    pub not_in: ::std::vec::Vec<f64>,
}
/// Int32Rules describes the constraints applied to `int32` values
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Int32Rules {
    /// Const specifies that this field must be exactly the specified value
    #[prost(int32, optional, tag = "1")]
    pub r#const: ::std::option::Option<i32>,
    /// Lt specifies that this field must be less than the specified value,
    /// exclusive
    #[prost(int32, optional, tag = "2")]
    pub lt: ::std::option::Option<i32>,
    /// Lte specifies that this field must be less than or equal to the
    /// specified value, inclusive
    #[prost(int32, optional, tag = "3")]
    pub lte: ::std::option::Option<i32>,
    /// Gt specifies that this field must be greater than the specified value,
    /// exclusive. If the value of Gt is larger than a specified Lt or Lte, the
    /// range is reversed.
    #[prost(int32, optional, tag = "4")]
    pub gt: ::std::option::Option<i32>,
    /// Gte specifies that this field must be greater than or equal to the
    /// specified value, inclusive. If the value of Gte is larger than a
    /// specified Lt or Lte, the range is reversed.
    #[prost(int32, optional, tag = "5")]
    pub gte: ::std::option::Option<i32>,
    /// In specifies that this field must be equal to one of the specified
    /// values
    #[prost(int32, repeated, packed = "false", tag = "6")]
    pub r#in: ::std::vec::Vec<i32>,
    /// NotIn specifies that this field cannot be equal to one of the specified
    /// values
    #[prost(int32, repeated, packed = "false", tag = "7")]
    pub not_in: ::std::vec::Vec<i32>,
}
/// Int64Rules describes the constraints applied to `int64` values
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Int64Rules {
    /// Const specifies that this field must be exactly the specified value
    #[prost(int64, optional, tag = "1")]
    pub r#const: ::std::option::Option<i64>,
    /// Lt specifies that this field must be less than the specified value,
    /// exclusive
    #[prost(int64, optional, tag = "2")]
    pub lt: ::std::option::Option<i64>,
    /// Lte specifies that this field must be less than or equal to the
    /// specified value, inclusive
    #[prost(int64, optional, tag = "3")]
    pub lte: ::std::option::Option<i64>,
    /// Gt specifies that this field must be greater than the specified value,
    /// exclusive. If the value of Gt is larger than a specified Lt or Lte, the
    /// range is reversed.
    #[prost(int64, optional, tag = "4")]
    pub gt: ::std::option::Option<i64>,
    /// Gte specifies that this field must be greater than or equal to the
    /// specified value, inclusive. If the value of Gte is larger than a
    /// specified Lt or Lte, the range is reversed.
    #[prost(int64, optional, tag = "5")]
    pub gte: ::std::option::Option<i64>,
    /// In specifies that this field must be equal to one of the specified
    /// values
    #[prost(int64, repeated, packed = "false", tag = "6")]
    pub r#in: ::std::vec::Vec<i64>,
    /// NotIn specifies that this field cannot be equal to one of the specified
    /// values
    #[prost(int64, repeated, packed = "false", tag = "7")]
    pub not_in: ::std::vec::Vec<i64>,
}
/// UInt32Rules describes the constraints applied to `uint32` values
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct UInt32Rules {
    /// Const specifies that this field must be exactly the specified value
    #[prost(uint32, optional, tag = "1")]
    pub r#const: ::std::option::Option<u32>,
    /// Lt specifies that this field must be less than the specified value,
    /// exclusive
    #[prost(uint32, optional, tag = "2")]
    pub lt: ::std::option::Option<u32>,
    /// Lte specifies that this field must be less than or equal to the
    /// specified value, inclusive
    #[prost(uint32, optional, tag = "3")]
    pub lte: ::std::option::Option<u32>,
    /// Gt specifies that this field must be greater than the specified value,
    /// exclusive. If the value of Gt is larger than a specified Lt or Lte, the
    /// range is reversed.
    #[prost(uint32, optional, tag = "4")]
    pub gt: ::std::option::Option<u32>,
    /// Gte specifies that this field must be greater than or equal to the
    /// specified value, inclusive. If the value of Gte is larger than a
    /// specified Lt or Lte, the range is reversed.
    #[prost(uint32, optional, tag = "5")]
    pub gte: ::std::option::Option<u32>,
    /// In specifies that this field must be equal to one of the specified
    /// values
    #[prost(uint32, repeated, packed = "false", tag = "6")]
    pub r#in: ::std::vec::Vec<u32>,
    /// NotIn specifies that this field cannot be equal to one of the specified
    /// values
    #[prost(uint32, repeated, packed = "false", tag = "7")]
    pub not_in: ::std::vec::Vec<u32>,
}
/// UInt64Rules describes the constraints applied to `uint64` values
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct UInt64Rules {
    /// Const specifies that this field must be exactly the specified value
    #[prost(uint64, optional, tag = "1")]
    pub r#const: ::std::option::Option<u64>,
    /// Lt specifies that this field must be less than the specified value,
    /// exclusive
    #[prost(uint64, optional, tag = "2")]
    pub lt: ::std::option::Option<u64>,
    /// Lte specifies that this field must be less than or equal to the
    /// specified value, inclusive
    #[prost(uint64, optional, tag = "3")]
    pub lte: ::std::option::Option<u64>,
    /// Gt specifies that this field must be greater than the specified value,
    /// exclusive. If the value of Gt is larger than a specified Lt or Lte, the
    /// range is reversed.
    #[prost(uint64, optional, tag = "4")]
    pub gt: ::std::option::Option<u64>,
    /// Gte specifies that this field must be greater than or equal to the
    /// specified value, inclusive. If the value of Gte is larger than a
    /// specified Lt or Lte, the range is reversed.
    #[prost(uint64, optional, tag = "5")]
    pub gte: ::std::option::Option<u64>,
    /// In specifies that this field must be equal to one of the specified
    /// values
    #[prost(uint64, repeated, packed = "false", tag = "6")]
    pub r#in: ::std::vec::Vec<u64>,
    /// NotIn specifies that this field cannot be equal to one of the specified
    /// values
    #[prost(uint64, repeated, packed = "false", tag = "7")]
    pub not_in: ::std::vec::Vec<u64>,
}
/// SInt32Rules describes the constraints applied to `sint32` values
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SInt32Rules {
    /// Const specifies that this field must be exactly the specified value
    #[prost(sint32, optional, tag = "1")]
    pub r#const: ::std::option::Option<i32>,
    /// Lt specifies that this field must be less than the specified value,
    /// exclusive
    #[prost(sint32, optional, tag = "2")]
    pub lt: ::std::option::Option<i32>,
    /// Lte specifies that this field must be less than or equal to the
    /// specified value, inclusive
    #[prost(sint32, optional, tag = "3")]
    pub lte: ::std::option::Option<i32>,
    /// Gt specifies that this field must be greater than the specified value,
    /// exclusive. If the value of Gt is larger than a specified Lt or Lte, the
    /// range is reversed.
    #[prost(sint32, optional, tag = "4")]
    pub gt: ::std::option::Option<i32>,
    /// Gte specifies that this field must be greater than or equal to the
    /// specified value, inclusive. If the value of Gte is larger than a
    /// specified Lt or Lte, the range is reversed.
    #[prost(sint32, optional, tag = "5")]
    pub gte: ::std::option::Option<i32>,
    /// In specifies that this field must be equal to one of the specified
    /// values
    #[prost(sint32, repeated, packed = "false", tag = "6")]
    pub r#in: ::std::vec::Vec<i32>,
    /// NotIn specifies that this field cannot be equal to one of the specified
    /// values
    #[prost(sint32, repeated, packed = "false", tag = "7")]
    pub not_in: ::std::vec::Vec<i32>,
}
/// SInt64Rules describes the constraints applied to `sint64` values
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SInt64Rules {
    /// Const specifies that this field must be exactly the specified value
    #[prost(sint64, optional, tag = "1")]
    pub r#const: ::std::option::Option<i64>,
    /// Lt specifies that this field must be less than the specified value,
    /// exclusive
    #[prost(sint64, optional, tag = "2")]
    pub lt: ::std::option::Option<i64>,
    /// Lte specifies that this field must be less than or equal to the
    /// specified value, inclusive
    #[prost(sint64, optional, tag = "3")]
    pub lte: ::std::option::Option<i64>,
    /// Gt specifies that this field must be greater than the specified value,
    /// exclusive. If the value of Gt is larger than a specified Lt or Lte, the
    /// range is reversed.
    #[prost(sint64, optional, tag = "4")]
    pub gt: ::std::option::Option<i64>,
    /// Gte specifies that this field must be greater than or equal to the
    /// specified value, inclusive. If the value of Gte is larger than a
    /// specified Lt or Lte, the range is reversed.
    #[prost(sint64, optional, tag = "5")]
    pub gte: ::std::option::Option<i64>,
    /// In specifies that this field must be equal to one of the specified
    /// values
    #[prost(sint64, repeated, packed = "false", tag = "6")]
    pub r#in: ::std::vec::Vec<i64>,
    /// NotIn specifies that this field cannot be equal to one of the specified
    /// values
    #[prost(sint64, repeated, packed = "false", tag = "7")]
    pub not_in: ::std::vec::Vec<i64>,
}
/// Fixed32Rules describes the constraints applied to `fixed32` values
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Fixed32Rules {
    /// Const specifies that this field must be exactly the specified value
    #[prost(fixed32, optional, tag = "1")]
    pub r#const: ::std::option::Option<u32>,
    /// Lt specifies that this field must be less than the specified value,
    /// exclusive
    #[prost(fixed32, optional, tag = "2")]
    pub lt: ::std::option::Option<u32>,
    /// Lte specifies that this field must be less than or equal to the
    /// specified value, inclusive
    #[prost(fixed32, optional, tag = "3")]
    pub lte: ::std::option::Option<u32>,
    /// Gt specifies that this field must be greater than the specified value,
    /// exclusive. If the value of Gt is larger than a specified Lt or Lte, the
    /// range is reversed.
    #[prost(fixed32, optional, tag = "4")]
    pub gt: ::std::option::Option<u32>,
    /// Gte specifies that this field must be greater than or equal to the
    /// specified value, inclusive. If the value of Gte is larger than a
    /// specified Lt or Lte, the range is reversed.
    #[prost(fixed32, optional, tag = "5")]
    pub gte: ::std::option::Option<u32>,
    /// In specifies that this field must be equal to one of the specified
    /// values
    #[prost(fixed32, repeated, packed = "false", tag = "6")]
    pub r#in: ::std::vec::Vec<u32>,
    /// NotIn specifies that this field cannot be equal to one of the specified
    /// values
    #[prost(fixed32, repeated, packed = "false", tag = "7")]
    pub not_in: ::std::vec::Vec<u32>,
}
/// Fixed64Rules describes the constraints applied to `fixed64` values
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Fixed64Rules {
    /// Const specifies that this field must be exactly the specified value
    #[prost(fixed64, optional, tag = "1")]
    pub r#const: ::std::option::Option<u64>,
    /// Lt specifies that this field must be less than the specified value,
    /// exclusive
    #[prost(fixed64, optional, tag = "2")]
    pub lt: ::std::option::Option<u64>,
    /// Lte specifies that this field must be less than or equal to the
    /// specified value, inclusive
    #[prost(fixed64, optional, tag = "3")]
    pub lte: ::std::option::Option<u64>,
    /// Gt specifies that this field must be greater than the specified value,
    /// exclusive. If the value of Gt is larger than a specified Lt or Lte, the
    /// range is reversed.
    #[prost(fixed64, optional, tag = "4")]
    pub gt: ::std::option::Option<u64>,
    /// Gte specifies that this field must be greater than or equal to the
    /// specified value, inclusive. If the value of Gte is larger than a
    /// specified Lt or Lte, the range is reversed.
    #[prost(fixed64, optional, tag = "5")]
    pub gte: ::std::option::Option<u64>,
    /// In specifies that this field must be equal to one of the specified
    /// values
    #[prost(fixed64, repeated, packed = "false", tag = "6")]
    pub r#in: ::std::vec::Vec<u64>,
    /// NotIn specifies that this field cannot be equal to one of the specified
    /// values
    #[prost(fixed64, repeated, packed = "false", tag = "7")]
    pub not_in: ::std::vec::Vec<u64>,
}
/// SFixed32Rules describes the constraints applied to `sfixed32` values
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SFixed32Rules {
    /// Const specifies that this field must be exactly the specified value
    #[prost(sfixed32, optional, tag = "1")]
    pub r#const: ::std::option::Option<i32>,
    /// Lt specifies that this field must be less than the specified value,
    /// exclusive
    #[prost(sfixed32, optional, tag = "2")]
    pub lt: ::std::option::Option<i32>,
    /// Lte specifies that this field must be less than or equal to the
    /// specified value, inclusive
    #[prost(sfixed32, optional, tag = "3")]
    pub lte: ::std::option::Option<i32>,
    /// Gt specifies that this field must be greater than the specified value,
    /// exclusive. If the value of Gt is larger than a specified Lt or Lte, the
    /// range is reversed.
    #[prost(sfixed32, optional, tag = "4")]
    pub gt: ::std::option::Option<i32>,
    /// Gte specifies that this field must be greater than or equal to the
    /// specified value, inclusive. If the value of Gte is larger than a
    /// specified Lt or Lte, the range is reversed.
    #[prost(sfixed32, optional, tag = "5")]
    pub gte: ::std::option::Option<i32>,
    /// In specifies that this field must be equal to one of the specified
    /// values
    #[prost(sfixed32, repeated, packed = "false", tag = "6")]
    pub r#in: ::std::vec::Vec<i32>,
    /// NotIn specifies that this field cannot be equal to one of the specified
    /// values
    #[prost(sfixed32, repeated, packed = "false", tag = "7")]
    pub not_in: ::std::vec::Vec<i32>,
}
/// SFixed64Rules describes the constraints applied to `sfixed64` values
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SFixed64Rules {
    /// Const specifies that this field must be exactly the specified value
    #[prost(sfixed64, optional, tag = "1")]
    pub r#const: ::std::option::Option<i64>,
    /// Lt specifies that this field must be less than the specified value,
    /// exclusive
    #[prost(sfixed64, optional, tag = "2")]
    pub lt: ::std::option::Option<i64>,
    /// Lte specifies that this field must be less than or equal to the
    /// specified value, inclusive
    #[prost(sfixed64, optional, tag = "3")]
    pub lte: ::std::option::Option<i64>,
    /// Gt specifies that this field must be greater than the specified value,
    /// exclusive. If the value of Gt is larger than a specified Lt or Lte, the
    /// range is reversed.
    #[prost(sfixed64, optional, tag = "4")]
    pub gt: ::std::option::Option<i64>,
    /// Gte specifies that this field must be greater than or equal to the
    /// specified value, inclusive. If the value of Gte is larger than a
    /// specified Lt or Lte, the range is reversed.
    #[prost(sfixed64, optional, tag = "5")]
    pub gte: ::std::option::Option<i64>,
    /// In specifies that this field must be equal to one of the specified
    /// values
    #[prost(sfixed64, repeated, packed = "false", tag = "6")]
    pub r#in: ::std::vec::Vec<i64>,
    /// NotIn specifies that this field cannot be equal to one of the specified
    /// values
    #[prost(sfixed64, repeated, packed = "false", tag = "7")]
    pub not_in: ::std::vec::Vec<i64>,
}
/// BoolRules describes the constraints applied to `bool` values
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct BoolRules {
    /// Const specifies that this field must be exactly the specified value
    #[prost(bool, optional, tag = "1")]
    pub r#const: ::std::option::Option<bool>,
}
/// StringRules describe the constraints applied to `string` values
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct StringRules {
    /// Const specifies that this field must be exactly the specified value
    #[prost(string, optional, tag = "1")]
    pub r#const: ::std::option::Option<std::string::String>,
    /// Len specifies that this field must be the specified number of
    /// characters (Unicode code points). Note that the number of
    /// characters may differ from the number of bytes in the string.
    #[prost(uint64, optional, tag = "19")]
    pub len: ::std::option::Option<u64>,
    /// MinLen specifies that this field must be the specified number of
    /// characters (Unicode code points) at a minimum. Note that the number of
    /// characters may differ from the number of bytes in the string.
    #[prost(uint64, optional, tag = "2")]
    pub min_len: ::std::option::Option<u64>,
    /// MaxLen specifies that this field must be the specified number of
    /// characters (Unicode code points) at a maximum. Note that the number of
    /// characters may differ from the number of bytes in the string.
    #[prost(uint64, optional, tag = "3")]
    pub max_len: ::std::option::Option<u64>,
    /// LenBytes specifies that this field must be the specified number of bytes
    /// at a minimum
    #[prost(uint64, optional, tag = "20")]
    pub len_bytes: ::std::option::Option<u64>,
    /// MinBytes specifies that this field must be the specified number of bytes
    /// at a minimum
    #[prost(uint64, optional, tag = "4")]
    pub min_bytes: ::std::option::Option<u64>,
    /// MaxBytes specifies that this field must be the specified number of bytes
    /// at a maximum
    #[prost(uint64, optional, tag = "5")]
    pub max_bytes: ::std::option::Option<u64>,
    /// Pattern specifes that this field must match against the specified
    /// regular expression (RE2 syntax). The included expression should elide
    /// any delimiters.
    #[prost(string, optional, tag = "6")]
    pub pattern: ::std::option::Option<std::string::String>,
    /// Prefix specifies that this field must have the specified substring at
    /// the beginning of the string.
    #[prost(string, optional, tag = "7")]
    pub prefix: ::std::option::Option<std::string::String>,
    /// Suffix specifies that this field must have the specified substring at
    /// the end of the string.
    #[prost(string, optional, tag = "8")]
    pub suffix: ::std::option::Option<std::string::String>,
    /// Contains specifies that this field must have the specified substring
    /// anywhere in the string.
    #[prost(string, optional, tag = "9")]
    pub contains: ::std::option::Option<std::string::String>,
    /// NotContains specifies that this field cannot have the specified substring
    /// anywhere in the string.
    #[prost(string, optional, tag = "23")]
    pub not_contains: ::std::option::Option<std::string::String>,
    /// In specifies that this field must be equal to one of the specified
    /// values
    #[prost(string, repeated, tag = "10")]
    pub r#in: ::std::vec::Vec<std::string::String>,
    /// NotIn specifies that this field cannot be equal to one of the specified
    /// values
    #[prost(string, repeated, tag = "11")]
    pub not_in: ::std::vec::Vec<std::string::String>,
    /// This applies to regexes HTTP_HEADER_NAME and HTTP_HEADER_VALUE to enable
    /// strict header validation.
    /// By default, this is true, and HTTP header validations are RFC-compliant.
    /// Setting to false will enable a looser validations that only disallows
    /// \r\n\0 characters, which can be used to bypass header matching rules.
    #[prost(bool, optional, tag = "25", default = "true")]
    pub strict: ::std::option::Option<bool>,
    /// WellKnown rules provide advanced constraints against common string
    /// patterns
    #[prost(
        oneof = "string_rules::WellKnown",
        tags = "12, 13, 14, 15, 16, 17, 18, 21, 22, 24"
    )]
    pub well_known: ::std::option::Option<string_rules::WellKnown>,
}
pub mod string_rules {
    /// WellKnown rules provide advanced constraints against common string
    /// patterns
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum WellKnown {
        /// Email specifies that the field must be a valid email address as
        /// defined by RFC 5322
        #[prost(bool, tag = "12")]
        Email(bool),
        /// Hostname specifies that the field must be a valid hostname as
        /// defined by RFC 1034. This constraint does not support
        /// internationalized domain names (IDNs).
        #[prost(bool, tag = "13")]
        Hostname(bool),
        /// Ip specifies that the field must be a valid IP (v4 or v6) address.
        /// Valid IPv6 addresses should not include surrounding square brackets.
        #[prost(bool, tag = "14")]
        Ip(bool),
        /// Ipv4 specifies that the field must be a valid IPv4 address.
        #[prost(bool, tag = "15")]
        Ipv4(bool),
        /// Ipv6 specifies that the field must be a valid IPv6 address. Valid
        /// IPv6 addresses should not include surrounding square brackets.
        #[prost(bool, tag = "16")]
        Ipv6(bool),
        /// Uri specifies that the field must be a valid, absolute URI as defined
        /// by RFC 3986
        #[prost(bool, tag = "17")]
        Uri(bool),
        /// UriRef specifies that the field must be a valid URI as defined by RFC
        /// 3986 and may be relative or absolute.
        #[prost(bool, tag = "18")]
        UriRef(bool),
        /// Address specifies that the field must be either a valid hostname as
        /// defined by RFC 1034 (which does not support internationalized domain
        /// names or IDNs), or it can be a valid IP (v4 or v6).
        #[prost(bool, tag = "21")]
        Address(bool),
        /// Uuid specifies that the field must be a valid UUID as defined by
        /// RFC 4122
        #[prost(bool, tag = "22")]
        Uuid(bool),
        /// WellKnownRegex specifies a common well known pattern defined as a regex.
        #[prost(enumeration = "super::KnownRegex", tag = "24")]
        WellKnownRegex(i32),
    }
}
/// BytesRules describe the constraints applied to `bytes` values
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct BytesRules {
    /// Const specifies that this field must be exactly the specified value
    #[prost(bytes, optional, tag = "1")]
    pub r#const: ::std::option::Option<std::vec::Vec<u8>>,
    /// Len specifies that this field must be the specified number of bytes
    #[prost(uint64, optional, tag = "13")]
    pub len: ::std::option::Option<u64>,
    /// MinLen specifies that this field must be the specified number of bytes
    /// at a minimum
    #[prost(uint64, optional, tag = "2")]
    pub min_len: ::std::option::Option<u64>,
    /// MaxLen specifies that this field must be the specified number of bytes
    /// at a maximum
    #[prost(uint64, optional, tag = "3")]
    pub max_len: ::std::option::Option<u64>,
    /// Pattern specifes that this field must match against the specified
    /// regular expression (RE2 syntax). The included expression should elide
    /// any delimiters.
    #[prost(string, optional, tag = "4")]
    pub pattern: ::std::option::Option<std::string::String>,
    /// Prefix specifies that this field must have the specified bytes at the
    /// beginning of the string.
    #[prost(bytes, optional, tag = "5")]
    pub prefix: ::std::option::Option<std::vec::Vec<u8>>,
    /// Suffix specifies that this field must have the specified bytes at the
    /// end of the string.
    #[prost(bytes, optional, tag = "6")]
    pub suffix: ::std::option::Option<std::vec::Vec<u8>>,
    /// Contains specifies that this field must have the specified bytes
    /// anywhere in the string.
    #[prost(bytes, optional, tag = "7")]
    pub contains: ::std::option::Option<std::vec::Vec<u8>>,
    /// In specifies that this field must be equal to one of the specified
    /// values
    #[prost(bytes, repeated, tag = "8")]
    pub r#in: ::std::vec::Vec<std::vec::Vec<u8>>,
    /// NotIn specifies that this field cannot be equal to one of the specified
    /// values
    #[prost(bytes, repeated, tag = "9")]
    pub not_in: ::std::vec::Vec<std::vec::Vec<u8>>,
    /// WellKnown rules provide advanced constraints against common byte
    /// patterns
    #[prost(oneof = "bytes_rules::WellKnown", tags = "10, 11, 12")]
    pub well_known: ::std::option::Option<bytes_rules::WellKnown>,
}
pub mod bytes_rules {
    /// WellKnown rules provide advanced constraints against common byte
    /// patterns
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum WellKnown {
        /// Ip specifies that the field must be a valid IP (v4 or v6) address in
        /// byte format
        #[prost(bool, tag = "10")]
        Ip(bool),
        /// Ipv4 specifies that the field must be a valid IPv4 address in byte
        /// format
        #[prost(bool, tag = "11")]
        Ipv4(bool),
        /// Ipv6 specifies that the field must be a valid IPv6 address in byte
        /// format
        #[prost(bool, tag = "12")]
        Ipv6(bool),
    }
}
/// EnumRules describe the constraints applied to enum values
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct EnumRules {
    /// Const specifies that this field must be exactly the specified value
    #[prost(int32, optional, tag = "1")]
    pub r#const: ::std::option::Option<i32>,
    /// DefinedOnly specifies that this field must be only one of the defined
    /// values for this enum, failing on any undefined value.
    #[prost(bool, optional, tag = "2")]
    pub defined_only: ::std::option::Option<bool>,
    /// In specifies that this field must be equal to one of the specified
    /// values
    #[prost(int32, repeated, packed = "false", tag = "3")]
    pub r#in: ::std::vec::Vec<i32>,
    /// NotIn specifies that this field cannot be equal to one of the specified
    /// values
    #[prost(int32, repeated, packed = "false", tag = "4")]
    pub not_in: ::std::vec::Vec<i32>,
}
/// MessageRules describe the constraints applied to embedded message values.
/// For message-type fields, validation is performed recursively.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct MessageRules {
    /// Skip specifies that the validation rules of this field should not be
    /// evaluated
    #[prost(bool, optional, tag = "1")]
    pub skip: ::std::option::Option<bool>,
    /// Required specifies that this field must be set
    #[prost(bool, optional, tag = "2")]
    pub required: ::std::option::Option<bool>,
}
/// RepeatedRules describe the constraints applied to `repeated` values
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct RepeatedRules {
    /// MinItems specifies that this field must have the specified number of
    /// items at a minimum
    #[prost(uint64, optional, tag = "1")]
    pub min_items: ::std::option::Option<u64>,
    /// MaxItems specifies that this field must have the specified number of
    /// items at a maximum
    #[prost(uint64, optional, tag = "2")]
    pub max_items: ::std::option::Option<u64>,
    /// Unique specifies that all elements in this field must be unique. This
    /// contraint is only applicable to scalar and enum types (messages are not
    /// supported).
    #[prost(bool, optional, tag = "3")]
    pub unique: ::std::option::Option<bool>,
    /// Items specifies the contraints to be applied to each item in the field.
    /// Repeated message fields will still execute validation against each item
    /// unless skip is specified here.
    #[prost(message, optional, boxed, tag = "4")]
    pub items: ::std::option::Option<::std::boxed::Box<FieldRules>>,
}
/// MapRules describe the constraints applied to `map` values
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct MapRules {
    /// MinPairs specifies that this field must have the specified number of
    /// KVs at a minimum
    #[prost(uint64, optional, tag = "1")]
    pub min_pairs: ::std::option::Option<u64>,
    /// MaxPairs specifies that this field must have the specified number of
    /// KVs at a maximum
    #[prost(uint64, optional, tag = "2")]
    pub max_pairs: ::std::option::Option<u64>,
    /// NoSparse specifies values in this field cannot be unset. This only
    /// applies to map's with message value types.
    #[prost(bool, optional, tag = "3")]
    pub no_sparse: ::std::option::Option<bool>,
    /// Keys specifies the constraints to be applied to each key in the field.
    #[prost(message, optional, boxed, tag = "4")]
    pub keys: ::std::option::Option<::std::boxed::Box<FieldRules>>,
    /// Values specifies the constraints to be applied to the value of each key
    /// in the field. Message values will still have their validations evaluated
    /// unless skip is specified here.
    #[prost(message, optional, boxed, tag = "5")]
    pub values: ::std::option::Option<::std::boxed::Box<FieldRules>>,
}
/// AnyRules describe constraints applied exclusively to the
/// `google.protobuf.Any` well-known type
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct AnyRules {
    /// Required specifies that this field must be set
    #[prost(bool, optional, tag = "1")]
    pub required: ::std::option::Option<bool>,
    /// In specifies that this field's `type_url` must be equal to one of the
    /// specified values.
    #[prost(string, repeated, tag = "2")]
    pub r#in: ::std::vec::Vec<std::string::String>,
    /// NotIn specifies that this field's `type_url` must not be equal to any of
    /// the specified values.
    #[prost(string, repeated, tag = "3")]
    pub not_in: ::std::vec::Vec<std::string::String>,
}
/// DurationRules describe the constraints applied exclusively to the
/// `google.protobuf.Duration` well-known type
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct DurationRules {
    /// Required specifies that this field must be set
    #[prost(bool, optional, tag = "1")]
    pub required: ::std::option::Option<bool>,
    /// Const specifies that this field must be exactly the specified value
    #[prost(message, optional, tag = "2")]
    pub r#const: ::std::option::Option<::prost_types::Duration>,
    /// Lt specifies that this field must be less than the specified value,
    /// exclusive
    #[prost(message, optional, tag = "3")]
    pub lt: ::std::option::Option<::prost_types::Duration>,
    /// Lt specifies that this field must be less than the specified value,
    /// inclusive
    #[prost(message, optional, tag = "4")]
    pub lte: ::std::option::Option<::prost_types::Duration>,
    /// Gt specifies that this field must be greater than the specified value,
    /// exclusive
    #[prost(message, optional, tag = "5")]
    pub gt: ::std::option::Option<::prost_types::Duration>,
    /// Gte specifies that this field must be greater than the specified value,
    /// inclusive
    #[prost(message, optional, tag = "6")]
    pub gte: ::std::option::Option<::prost_types::Duration>,
    /// In specifies that this field must be equal to one of the specified
    /// values
    #[prost(message, repeated, tag = "7")]
    pub r#in: ::std::vec::Vec<::prost_types::Duration>,
    /// NotIn specifies that this field cannot be equal to one of the specified
    /// values
    #[prost(message, repeated, tag = "8")]
    pub not_in: ::std::vec::Vec<::prost_types::Duration>,
}
/// TimestampRules describe the constraints applied exclusively to the
/// `google.protobuf.Timestamp` well-known type
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct TimestampRules {
    /// Required specifies that this field must be set
    #[prost(bool, optional, tag = "1")]
    pub required: ::std::option::Option<bool>,
    /// Const specifies that this field must be exactly the specified value
    #[prost(message, optional, tag = "2")]
    pub r#const: ::std::option::Option<::prost_types::Timestamp>,
    /// Lt specifies that this field must be less than the specified value,
    /// exclusive
    #[prost(message, optional, tag = "3")]
    pub lt: ::std::option::Option<::prost_types::Timestamp>,
    /// Lte specifies that this field must be less than the specified value,
    /// inclusive
    #[prost(message, optional, tag = "4")]
    pub lte: ::std::option::Option<::prost_types::Timestamp>,
    /// Gt specifies that this field must be greater than the specified value,
    /// exclusive
    #[prost(message, optional, tag = "5")]
    pub gt: ::std::option::Option<::prost_types::Timestamp>,
    /// Gte specifies that this field must be greater than the specified value,
    /// inclusive
    #[prost(message, optional, tag = "6")]
    pub gte: ::std::option::Option<::prost_types::Timestamp>,
    /// LtNow specifies that this must be less than the current time. LtNow
    /// can only be used with the Within rule.
    #[prost(bool, optional, tag = "7")]
    pub lt_now: ::std::option::Option<bool>,
    /// GtNow specifies that this must be greater than the current time. GtNow
    /// can only be used with the Within rule.
    #[prost(bool, optional, tag = "8")]
    pub gt_now: ::std::option::Option<bool>,
    /// Within specifies that this field must be within this duration of the
    /// current time. This constraint can be used alone or with the LtNow and
    /// GtNow rules.
    #[prost(message, optional, tag = "9")]
    pub within: ::std::option::Option<::prost_types::Duration>,
}
/// WellKnownRegex contain some well-known patterns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum KnownRegex {
    Unknown = 0,
    /// HTTP header name as defined by RFC 7230.
    HttpHeaderName = 1,
    /// HTTP header value as defined by RFC 7230.
    HttpHeaderValue = 2,
}
