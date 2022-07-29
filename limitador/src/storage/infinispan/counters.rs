// Infinispan does not support counters with TTL. This module provides some
// functions on top of the Infinispan client to be able to work with counters
// with TTLs.
//
// Counters in Infinispan don't have a TTL. So to implement a counter, apart
// from the actual counter, we need a helper key with a TTL that simply tells us
// whether the counter is still valid.
//
// This is how it works in more detail: When a counter is created, apart from
// the actual Infinispan counter, this module creates a regular entry with the
// same name as the counter and with a TTL. The TTL is set to the duration of
// the limit. So, for a limit of 10 requests per minute, the TTL would be a
// minute. While the key has not expired, we know that the value of the counter
// is valid. When the key has expired, the value of the counter no longer
// applies.

use crate::storage::infinispan::response::response_to_string;
use infinispan::errors::InfinispanError;
use infinispan::{request, Infinispan};
use std::convert::TryFrom;
use std::fmt::{Display, Formatter};
use std::time::Duration;
use thiserror::Error;

#[derive(Copy, Clone)]
pub enum Consistency {
    Weak,
    Strong,
}

impl Display for Consistency {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Consistency::Weak => write!(f, "Weak"),
            Consistency::Strong => write!(f, "Strong"),
        }
    }
}

#[derive(Error, Debug)]
#[error("invalid consistency mode: {mode}")]
pub struct InvalidConsistencyErr {
    mode: String,
}

impl TryFrom<String> for Consistency {
    type Error = InvalidConsistencyErr;

    fn try_from(mut value: String) -> Result<Self, Self::Error> {
        value.make_ascii_lowercase();

        match value.as_str() {
            "weak" => Ok(Self::Weak),
            "strong" => Ok(Self::Strong),
            _ => Err(InvalidConsistencyErr { mode: value }),
        }
    }
}

pub struct CounterOpts {
    initial_value: i64,
    ttl: Duration,
    consistency: Consistency,
}

impl CounterOpts {
    pub fn new(initial_value: i64, ttl: Duration, consistency: Consistency) -> Self {
        Self {
            initial_value,
            ttl,
            consistency,
        }
    }
}

pub async fn get_value(
    infinispan: &Infinispan,
    cache_name: impl AsRef<str>,
    counter_key: impl AsRef<str>,
) -> Result<Option<i64>, InfinispanError> {
    let response = infinispan
        .run(&request::entries::get(cache_name, &counter_key))
        .await?;

    if response.status() == 404 {
        return Ok(None);
    }

    let response = infinispan
        .run(&request::counters::get(&counter_key))
        .await?;

    // Note: as these operations are not atomic, the counter might have been
    // deleted at this point. In that case, the parsing will fail.
    return match response_to_string(response).await.parse::<i64>() {
        Ok(val) => Ok(Some(val)),
        Err(_) => Ok(None),
    };
}

// Returns a bool that indicates whether the counter was created.
//
// The param "create_counter_opts" specifies the options used to create a
// counter when it's needed (it has expired or was never used).
pub async fn decrement_by(
    infinispan: &Infinispan,
    cache_name: impl Into<String>,
    counter_key: impl Into<String>,
    delta: i64,
    create_counter_opts: &CounterOpts,
) -> Result<bool, InfinispanError> {
    let cache_name = cache_name.into();
    let counter_key = counter_key.into();

    let response = infinispan
        .run(&request::entries::get(&cache_name, &counter_key))
        .await?;

    if response.status() == 404 {
        let reset_resp = infinispan
            .run(&request::counters::reset(&counter_key))
            .await?;

        if reset_resp.status() == 404 {
            let create_req = match &create_counter_opts.consistency {
                Consistency::Weak => request::counters::create_weak(&counter_key),
                Consistency::Strong => request::counters::create_strong(&counter_key),
            };

            let _ = infinispan
                .run(&create_req.with_value(create_counter_opts.initial_value - delta))
                .await?;
        }

        let _ = infinispan
            .run(
                &request::entries::create(&cache_name, &counter_key)
                    .with_ttl(create_counter_opts.ttl),
            )
            .await?;

        Ok(true)
    } else {
        // TODO: check other errors
        let _ = infinispan
            .run(&request::counters::increment(&counter_key).by(-delta))
            .await?;

        Ok(false)
    }
}

pub async fn delete(
    infinispan: &Infinispan,
    cache_name: impl AsRef<str>,
    counter_key: impl AsRef<str>,
) -> Result<(), InfinispanError> {
    let _ = infinispan
        .run(&request::entries::delete(cache_name, &counter_key))
        .await?;

    let _ = infinispan
        .run(&request::counters::delete(&counter_key))
        .await?;

    Ok(())
}
