// Infinispan does not support sets. This module provides some functions on top
// of the Infinispan client to be able to work with sets.

use crate::storage::infinispan::dist_lock;
use crate::storage::infinispan::response::response_to_string;
use crate::storage::StorageErr;
use infinispan::errors::InfinispanError;
use infinispan::{request, Infinispan};
use serde_json::json;
use std::collections::HashSet;
use std::iter::FromIterator;

pub async fn get(
    infinispan: &Infinispan,
    cache_name: impl AsRef<str>,
    set_key: impl AsRef<str>,
) -> Result<HashSet<String>, InfinispanError> {
    let get_entry_response = infinispan
        .run(&request::entries::get(cache_name, &set_key))
        .await?;

    let set = if get_entry_response.status() == 404 {
        HashSet::new()
    } else {
        // TODO: handle other errors
        serde_json::from_str(&response_to_string(get_entry_response).await).unwrap()
    };

    Ok(set)
}

// Creates the set if it does not exist.
pub async fn add(
    infinispan: &Infinispan,
    cache_name: impl Into<String>,
    set_key: impl Into<String>,
    element: impl Into<String>,
) -> Result<(), StorageErr> {
    let cache_name = cache_name.into();
    let set_key = set_key.into();
    let element = element.into();

    dist_lock::lock(infinispan, &cache_name, &set_key).await?;

    let res = add_set(infinispan, cache_name.clone(), set_key.clone(), element).await;

    dist_lock::release(infinispan, &cache_name, &set_key).await?;

    res
}

pub async fn delete(
    infinispan: &Infinispan,
    cache_name: impl Into<String>,
    set_key: impl Into<String>,
    element: impl Into<String>,
) -> Result<HashSet<String>, StorageErr> {
    let cache_name = cache_name.into();
    let set_key = set_key.into();
    let element = element.into();

    dist_lock::lock(infinispan, &cache_name, &set_key).await?;

    let res = delete_from_set(infinispan, cache_name.clone(), set_key.clone(), element).await;

    dist_lock::release(infinispan, &cache_name, &set_key).await?;

    res
}

async fn add_set(
    infinispan: &Infinispan,
    cache_name: String,
    set_key: String,
    element: String,
) -> Result<(), StorageErr> {
    let get_entry_response = infinispan
        .run(&request::entries::get(&cache_name, &set_key))
        .await?;

    if get_entry_response.status() == 404 {
        let limits_set: HashSet<String> = HashSet::from_iter(vec![element]);

        let _ = infinispan
            .run(
                &request::entries::create(&cache_name, &set_key)
                    .with_value(json!(limits_set).to_string()),
            )
            .await?;
    } else {
        // TODO: handle other errors
        let mut limits_set: HashSet<String> =
            serde_json::from_str(&response_to_string(get_entry_response).await).unwrap();
        limits_set.insert(element);

        let _ = infinispan
            .run(&request::entries::update(
                &cache_name,
                &set_key,
                &json!(limits_set).to_string(),
            ))
            .await?;
    }

    Ok(())
}

async fn delete_from_set(
    infinispan: &Infinispan,
    cache_name: String,
    set_key: String,
    element: String,
) -> Result<HashSet<String>, StorageErr> {
    let response = infinispan
        .run(&request::entries::get(&cache_name, &set_key))
        .await?;

    let mut set: HashSet<String> =
        serde_json::from_str(&response_to_string(response).await).unwrap();

    set.remove(&element);

    if set.is_empty() {
        let _ = infinispan
            .run(&request::entries::delete(&cache_name, &set_key))
            .await?;
    } else {
        let _ = infinispan
            .run(&request::entries::update(
                &cache_name,
                &set_key,
                &json!(set).to_string(),
            ))
            .await?;
    }

    return Ok(set);
}
