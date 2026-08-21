// Redis Lua scripts. Ref: https://redis.io/commands/eval
//
// The calls made in some scripts could also be run in a MULTI/EXEC. However,
// that's not always the case. Redis guarantees that keys do not expire in the
// middle of a script, but they can expire in the middle of a MULTI/EXEC. This
// means that the update counter script would not work when run as a MULTI/EXEC
// because the counter key could expire between the "set" and the "incrby"
// calls.

// KEYS[1]: counter key
// KEYS[2]: key that contains the counters that belong to the limit
// ARGV[1]: counter TTL
// ARGV[2]: delta
pub const SCRIPT_UPDATE_COUNTER: &str = "
    local c = redis.call('incrby', KEYS[1], ARGV[2])
    if c == tonumber(ARGV[2]) then
      redis.call('expire', KEYS[1], ARGV[1])
      redis.call('sadd', KEYS[2], KEYS[1])
    end
    return c";

// KEY[i]: Counter key
// KEY[i+1]: Limit key
// ARGV[i]: TTLs
// ARGV[i+1]: Deltas
// This function returns a list with the values and TTLs for the updated counter_keys,
// the first position the counter value and the second the TTL
pub const BATCH_UPDATE_COUNTERS: &str = "
    local res = {}
    for i = 1, #KEYS, 2 do
        local counter_key = KEYS[i]
        local limit_key = KEYS[i+1]
        local ttl = ARGV[i]
        local delta = ARGV[i+1]

        local c = redis.call('incrby', counter_key, delta)
        table.insert(res, c)
        if c == tonumber(delta) then
            redis.call('expire', counter_key, ttl)
            redis.call('sadd', limit_key, counter_key)
        end
        table.insert(res, redis.call('pexpiretime', counter_key))
    end
    return res
";

// KEYS: the function returns the value and TTL (in ms) for these keys
// The first position of the list returned contains the value of KEYS[1], the
// second position contains its TTL. The third position contains the value of
// KEYS[2] and the fourth its TTL, and so on.
pub const VALUES_AND_TTLS: &str = "
    local res = {}
    for _, key in ipairs(KEYS) do
        table.insert(res, redis.call('get', key))
        table.insert(res, redis.call('pttl', key))
    end
    return res
";

// KEYS[1]: key that contains the counters that belong to the limit
// KEYS[i], i > 1: counter keys to read
// Returns the value and the TTL (in ms) of every counter key, in the same
// order and with the same layout as VALUES_AND_TTLS. The counters are passed
// as keys rather than as arguments so that Redis cluster can route the script.
//
// For a limit with no id they share the {namespace} hash tag with KEYS[1], so
// they all live on the same shard.
//
// A counter that no longer exists reports a nil value and is removed from the limit's counter set.
// Nothing else ever removes those members, so without this the set grows by one entry per counter
// ever created under the limit. The get and the srem must be one script. Otherwise an update
// recreating the counter in between finds the member still there, so its sadd no-ops and the srem
// then unindexes a live counter.
pub const GET_COUNTERS_AND_PRUNE: &str = "
    local res = {}
    for i = 2, #KEYS do
        local val = redis.call('get', KEYS[i])
        if val == false then
            redis.call('srem', KEYS[1], KEYS[i])
            table.insert(res, false)
            table.insert(res, -2)
        else
            table.insert(res, val)
            table.insert(res, redis.call('pttl', KEYS[i]))
        end
    end
    return res
";
