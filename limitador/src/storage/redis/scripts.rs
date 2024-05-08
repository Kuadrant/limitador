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
