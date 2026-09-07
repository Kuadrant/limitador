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

// Atomically checks and, if every counter admits it, holds `amount` of estimated
// capacity against all of them. All counters are admitted, or none are: nothing is
// written unless every counter would stay within its limit once outstanding, live
// reservations are accounted for.
//
// KEYS come in triplets: [counter_key, reservation_key, limit_key, ...]. `limit_key` is
// the same per-limit set `SCRIPT_UPDATE_COUNTER` maintains (the one `get_counters`/the
// `/counters` endpoint enumerate) - a freshly-initialized counter is registered into it
// here too, so a counter touched only by Reserve is still visible, exactly as if it had
// been touched by Report/CheckRateLimit.
// ARGV holds one (window_seconds, max_value) pair per counter, in the same order as
// KEYS, followed by four trailing scalars:
//   ARGV[2i-1] = window (seconds) for counter i, used to lazily start its window if
//                the counter key doesn't exist yet
//   ARGV[2i]   = max_value for counter i
//   ARGV[2n+1] = amount requested
//   ARGV[2n+2] = reservation ttl (ms), already clamped by the caller
//   ARGV[2n+3] = reservation id
//   ARGV[2n+4] = now (ms)
//
// Returns a flat list, three values per counter - [value, outstanding, window_ttl_ms] -
// followed by a trailing 1 (admitted) or 0 (limited).
pub const SCRIPT_RESERVE: &str = "
    local n = #KEYS / 3
    local amount = tonumber(ARGV[2 * n + 1])
    local ttl_ms = tonumber(ARGV[2 * n + 2])
    local reservation_id = ARGV[2 * n + 3]
    local now_ms = tonumber(ARGV[2 * n + 4])

    local values = {}
    local outstanding = {}
    local window_ttls = {}
    local admitted = true

    for i = 1, n do
        local counter_key = KEYS[3 * i - 2]
        local reservation_key = KEYS[3 * i - 1]
        local limit_key = KEYS[3 * i]
        local window_secs = tonumber(ARGV[2 * i - 1])
        local max_value = tonumber(ARGV[2 * i])

        if redis.call('exists', counter_key) == 0 then
            redis.call('set', counter_key, 0, 'EX', window_secs)
            redis.call('sadd', limit_key, counter_key)
        end
        local value = tonumber(redis.call('get', counter_key)) or 0
        local window_ttl_ms = redis.call('pttl', counter_key)
        if window_ttl_ms < 0 then
            window_ttl_ms = window_secs * 1000
        end

        local held = 0
        local fields = redis.call('hgetall', reservation_key)
        for j = 1, #fields, 2 do
            local field = fields[j]
            local packed = fields[j + 1]
            local sep = string.find(packed, ':')
            local field_amount = tonumber(string.sub(packed, 1, sep - 1))
            local field_expires_at = tonumber(string.sub(packed, sep + 1))
            if field_expires_at > now_ms then
                held = held + field_amount
            else
                redis.call('hdel', reservation_key, field)
            end
        end

        values[i] = value
        outstanding[i] = held
        window_ttls[i] = window_ttl_ms

        if value + held + amount > max_value then
            admitted = false
        end
    end

    if admitted then
        for i = 1, n do
            local reservation_key = KEYS[3 * i - 1]
            local expires_at_ms = now_ms + ttl_ms
            if expires_at_ms > now_ms + window_ttls[i] then
                expires_at_ms = now_ms + window_ttls[i]
            end
            redis.call('hset', reservation_key, reservation_id, amount .. ':' .. expires_at_ms)
            redis.call('pexpire', reservation_key, window_ttls[i])
        end
    end

    local res = {}
    for i = 1, n do
        table.insert(res, values[i])
        table.insert(res, outstanding[i])
        table.insert(res, window_ttls[i])
    end
    table.insert(res, admitted and 1 or 0)
    return res
";
