// Infinispan does not support locks in the REST API. This module adds support
// for them with some caveats.
//
// We use a standard Infinispan entry to simulate a lock. When we need to
// acquire a lock, we try to create an entry with a name that encodes the entry
// that we want to protect. If the call to create the entry fails because it
// already exists, it means that someone else already has the lock. To release
// the lock, we just need to delete the entry that represents it. To avoid
// holding the lock forever, the entry that represents the lock is created with
// a TTL.
//
// Notice that this approach has an important limitation: whatever we do within
// the lock critical section is racing against the TTL, and we cannot guarantee
// that the section will be finished within the limit of the TTL.

use crate::storage::StorageErr;
use infinispan::errors::InfinispanError;
use infinispan::request;
use infinispan::Infinispan;
use std::time::Duration;

const RETRIES_INTERVAL: Duration = Duration::from_millis(100);
const RETRIES: u64 = 20;
const LOCK_TTL: Duration = Duration::from_secs(10);

pub async fn lock(
    infinispan: &Infinispan,
    cache_name: impl Into<String>,
    entry_name: impl Into<String>,
) -> Result<(), StorageErr> {
    let cache_name = cache_name.into();
    let entry_name = entry_name.into();

    let mut retries = 0;

    loop {
        let resp = infinispan
            .run(
                &request::entries::create(cache_name.clone(), lock_name(&entry_name))
                    .with_ttl(LOCK_TTL),
            )
            .await?;

        let could_create = resp.status().is_success();

        if could_create {
            return Ok(());
        } else {
            if retries >= RETRIES {
                return Err(StorageErr {
                    msg: "can't acquire lock".into(),
                });
            }

            retries += 1;

            tokio::time::sleep(RETRIES_INTERVAL).await;
        }
    }
}

pub async fn release(
    infinispan: &Infinispan,
    cache_name: impl AsRef<str>,
    entry_name: impl AsRef<str>,
) -> Result<(), InfinispanError> {
    let _ = infinispan
        .run(&request::entries::delete(cache_name, lock_name(entry_name)))
        .await?;
    Ok(())
}

fn lock_name(entry_name: impl AsRef<str>) -> String {
    format!("lock:{}", entry_name.as_ref())
}
