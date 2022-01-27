// [#protodoc-title: Percent]

/// Identifies a percentage, in the range [0.0, 100.0].
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Percent {
    #[prost(double, tag = "1")]
    pub value: f64,
}
/// A fractional percentage is used in cases in which for performance reasons performing floating
/// point to integer conversions during randomness calculations is undesirable. The message includes
/// both a numerator and denominator that together determine the final fractional value.
///
/// * **Example**: 1/100 = 1%.
/// * **Example**: 3/10000 = 0.03%.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FractionalPercent {
    /// Specifies the numerator. Defaults to 0.
    #[prost(uint32, tag = "1")]
    pub numerator: u32,
    /// Specifies the denominator. If the denominator specified is less than the numerator, the final
    /// fractional percentage is capped at 1 (100%).
    #[prost(enumeration = "fractional_percent::DenominatorType", tag = "2")]
    pub denominator: i32,
}
/// Nested message and enum types in `FractionalPercent`.
pub mod fractional_percent {
    /// Fraction percentages support several fixed denominator values.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
    #[repr(i32)]
    pub enum DenominatorType {
        /// 100.
        ///
        /// **Example**: 1/100 = 1%.
        Hundred = 0,
        /// 10,000.
        ///
        /// **Example**: 1/10000 = 0.01%.
        TenThousand = 1,
        /// 1,000,000.
        ///
        /// **Example**: 1/1000000 = 0.0001%.
        Million = 2,
    }
}
// [#protodoc-title: Semantic Version]

/// Envoy uses SemVer (<https://semver.org/>). Major/minor versions indicate
/// expected behaviors and APIs, the patch version field is used only
/// for security fixes and can be generally ignored.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SemanticVersion {
    #[prost(uint32, tag = "1")]
    pub major_number: u32,
    #[prost(uint32, tag = "2")]
    pub minor_number: u32,
    #[prost(uint32, tag = "3")]
    pub patch: u32,
}
// [#protodoc-title: Ratelimit Time Unit]

/// Identifies the unit of of time for rate limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum RateLimitUnit {
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
// [#protodoc-title: Token bucket]

/// Configures a token bucket, typically used for rate limiting.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct TokenBucket {
    /// The maximum tokens that the bucket can hold. This is also the number of tokens that the bucket
    /// initially contains.
    #[prost(uint32, tag = "1")]
    pub max_tokens: u32,
    /// The number of tokens added to the bucket during each fill interval. If not specified, defaults
    /// to a single token.
    #[prost(message, optional, tag = "2")]
    pub tokens_per_fill: ::core::option::Option<u32>,
    /// The fill interval that tokens are added to the bucket. During each fill interval
    /// `tokens_per_fill` are added to the bucket. The bucket will never contain more than
    /// `max_tokens` tokens.
    #[prost(message, optional, tag = "3")]
    pub fill_interval: ::core::option::Option<::prost_types::Duration>,
}
