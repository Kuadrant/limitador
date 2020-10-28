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
// ARGV[1]: counter max val
// ARGV[2]: counter TTL
// ARGV[3]: delta
pub const SCRIPT_UPDATE_COUNTER: &str = "
            local set_res = redis.call('set', KEYS[1], ARGV[1], 'EX', ARGV[2], 'NX')
            redis.call('incrby', KEYS[1], - ARGV[3])
            if set_res then
                redis.call('sadd', KEYS[2], KEYS[1])
            end";

// KEYS[1]: key with limits of namespace
// KEYS[2]: key with set of all namespaces
// ARGV[1]: limit
// ARGV[2]: namespace of the limit
pub const SCRIPT_DELETE_LIMIT: &str = "
            redis.call('srem', KEYS[1], ARGV[1])
            if redis.call('scard', KEYS[1]) == 0 then
                redis.call('srem', KEYS[2], ARGV[2])
            end";
