use crate::counter::Counter;
use std::fmt::{Display, Formatter};
use std::time::{Duration, SystemTime};
use uuid::Uuid;

/// Opaque handle identifying a single reservation created by `RateLimiter::reserve`
/// (or its async twin) and later resolved by `RateLimiter::commit_reservation`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReservationId(String);

impl ReservationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ReservationId {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for ReservationId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for ReservationId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl AsRef<str> for ReservationId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// A single outstanding hold of estimated capacity against one counter.
///
/// The same `reservation_id` and `amount` are stored under every counter a
/// `RateLimiter::reserve` call touched; `expires_at` is computed independently
/// per counter, since it is clamped to that counter's own window boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReservationEntry {
    reservation_id: ReservationId,
    amount: u64,
    expires_at: SystemTime,
}

impl ReservationEntry {
    pub fn new(reservation_id: ReservationId, amount: u64, expires_at: SystemTime) -> Self {
        Self {
            reservation_id,
            amount,
            expires_at,
        }
    }

    pub fn reservation_id(&self) -> &ReservationId {
        &self.reservation_id
    }

    pub fn amount(&self) -> u64 {
        self.amount
    }

    pub fn expires_at(&self) -> SystemTime {
        self.expires_at
    }

    pub fn is_live_at(&self, when: SystemTime) -> bool {
        self.expires_at > when
    }
}

/// Result of `RateLimiter::reserve`/`AsyncRateLimiter::reserve`.
///
/// `reservation_id` is `Some` only when `limited` is `false`: admission was granted
/// and `amount` is now held against every matching counter until it expires or is
/// resolved by `RateLimiter::commit_reservation`.
///
/// `amount` is the amount actually held, uniformly across every matching counter - it
/// can be less than what was requested if `ReservationLimits::max_fraction` clamped it
/// down, and it's `0` when `limited` is `true`. It's purely informational:
/// `commit_reservation`'s `actual_amount` is independent of it and always applied as
/// given.
pub struct ReserveResult {
    pub limited: bool,
    pub reservation_id: Option<ReservationId>,
    pub amount: u64,
    pub counters: Vec<Counter>,
    pub limit_name: Option<String>,
}

/// Result of `RateLimiter::commit_reservation`/`AsyncRateLimiter::commit_reservation`.
///
/// `reservation_released` reflects whether a live reservation matching the given id
/// was found and removed; `actual_amount` is applied to the counters unconditionally
/// either way, so a missing/expired reservation degrades gracefully to `Report`-like
/// behavior.
pub struct CommitResult {
    pub reservation_released: bool,
}

/// Policy limits applied by `RateLimiter::reserve`/`AsyncRateLimiter::reserve`, set once via
/// `RateLimiterBuilder::reservation_limits`/`AsyncRateLimiterBuilder::reservation_limits`
/// rather than per call - the fraction clamp needs each matching counter's own `max_value`,
/// which `reserve()` already resolves internally, so there's no reason for callers to supply
/// it (or re-resolve counters themselves) on every call.
#[derive(Debug, Clone, Copy)]
pub struct ReservationLimits {
    /// Caps the amount actually held by an admitted `reserve()` call to
    /// `counter.max_value() * max_fraction`, for every matching counter, the most
    /// restrictive one wins. This never affects admission itself (that's always decided on
    /// the raw requested amount); it only limits how much of a counter a single reservation
    /// can claim, so it can't starve concurrent reservations. Defaults to `1.0`, i.e. no
    /// clamp beyond each counter's own `max_value`.
    ///
    /// This is a trade-off knob, not a free safety win: a lower `max_fraction` admits more
    /// *concurrent* reservations (better for fairness), but each holds a smaller, less
    /// accurate share of the real amount it may end up committing - so the more of them are
    /// in flight at once, the more the counter's real value can overshoot `max_value` once
    /// they all commit. A higher `max_fraction` accepts more monopolization risk (one
    /// reservation can claim more of the counter, or all of it at `1.0`) in exchange for a
    /// tighter bound on aggregate overshoot.
    pub max_fraction: f64,

    /// Hard ceiling on a `reserve()` call's requested ttl. Defaults to 60s (RFC 0021's
    /// recommendation) - unlike `max_fraction`, there's no safe "no-op" `Duration` to default
    /// to instead, since an extreme sentinel risks overflowing `SystemTime::now() + ttl`
    /// inside storage.
    pub max_ttl: Duration,
}

impl Default for ReservationLimits {
    fn default() -> Self {
        Self {
            max_fraction: 1.0,
            max_ttl: Duration::from_secs(60),
        }
    }
}
