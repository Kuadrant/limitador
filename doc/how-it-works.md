## How it works

Limitador will increment counters for all `Limit`s that apply, if any of these counter is above its `Limit`'s
`max_value` the request will be considered to be _rate limited_. So think of it as if *the most restrictive limit
configuration will apply*.

Limitador will evaluate whether a `Limit` applies against its *namespace*, its conditions and whether all variables are
resolvable. The namespace for the descriptors is defined by the `domain` field from the [
`service.ratelimit.v3.RateLimitRequest`](https://www.envoyproxy.io/docs/envoy/latest/api-v3/service/ratelimit/v3/rls.proto#service-ratelimit-v3-ratelimitrequest).
For each matching `Limit`, its counter is increased and checked against the `Limit` `max_value`.

One example to illustrate:

Let's say we have one rate limit:

```yaml
conditions: [ "descriptors[0].KEY_A == 'VALUE_A'" ]
max_value: 1
seconds: 60
variables: []
namespace: example.org
```

Limitador Server receives a request with one descriptor with two entries:

```yaml
domain: example.org
descriptors:
  - entries:
    - KEY_A: VALUE_A
    - OTHER_KEY: OTHER_VALUE
```

The counter's condition all match. Then, the counter will be increased and the limit checked.
If the limit is exceeded, the request will be rejected with `429 Too Many Requests`,
otherwise accepted.

Note that the counter is being activated even though it does not match *all* the entries of the
descriptor. The same rule applies for the *variables* field.

Conditions are [CEL](https://cel.dev) expressions evaluating to a `bool` value.

The *variables* field is a list of keys.
The matching rule is defined just as the existence of the list of descriptor entries with the
same key values. If *variables* is `variables: ["descriptors[0].A", "descriptors[0].B", "descriptors[0].C]"`,
the limit will match if the first descriptor has *at least* three entries with the same A, B, C keys.

Few examples to illustrate.

Having the following descriptors:

```yaml
domain: example.org
descriptors:
  - entries:
    - KEY_A: VALUE_A
    - OTHER_KEY: OTHER_VALUE
```

the following counters would **not** be activated.

```yaml
conditions: [ "descriptors[0].KEY_B == 'VALUE_B'" ]
max_value: 1
seconds: 60
variables: []
namespace: example.org
```
Reason: conditions key does not exist

```yaml
conditions:
  - "descriptors[0].KEY_A == 'VALUE_A'"
  - "descriptors[0].OTHER_KEY == 'WRONG_VALUE'"
max_value: 1
seconds: 60
variables: []
namespace: example.org
```
Reason: not all the conditions match

```yaml
conditions: []
max_value: 1
seconds: 60
variables: [ "descriptors[0].MY_VAR" ]
namespace: example.org
```
Reason: the variable name does not exist

```yaml
conditions: [ "descriptors[0].KEY_B == 'VALUE_B'" ]
max_value: 1
seconds: 60
variables: [ "descriptors[0].MY_VAR" ]
namespace: example.org
```
Reason: Both variables and conditions must match. In this particular case, only conditions match
