## How it works

Limitador ensures that *the most restrictive limit configuration will apply*.

Limitador will try to match each incoming descriptor with the same *namespaced*
counter's conditions and variables.
The namespace for the descriptors is defined by the `domain` field
whereas for the rate limit configuration the `namespace` field is being used.
For each matching counter, the counter is increased and the limits checked.

One example to illustrate:

Let's say we have 1 rate limit configuration (one counter per config):

```yaml
conditions: ["KEY_A == 'VALUE_A'"]
max_value: 1
seconds: 60
variables: []
namespace: example.org
```

Limitador receives one descriptor with two entries:

```yaml
domain: example.org
descriptors:
  - entries:
    - KEY_A: VALUE_A
    - OTHER_KEY: OTHER_VALUE
```

The counter's condition will match. Then, the counter will be increased and the limit checked.
If the limit is exceeded, the request will be rejected with `429 Too Many Requests`,
otherwise accepted.

Note that the counter is being activated eventhough it does not match *all* the entries of the
descriptor. The same rule applies for the *variables* field.

Currently *condition* implementation only allows *equal* operator.
More operators can be implemented if there are use cases.

The *variables* field is a list of keys.
The matching rule is defined just as the existence of the list of descriptor entries with the
same key values. If *variables* is `variables: [A, B, C]`,
one descriptor matches if it has *at least* three entries with the same A, B, C keys.

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
conditions: ["KEY_B == 'VALUE_B'"]
max_value: 1
seconds: 60
variables: []
namespace: example.org
```
Reason: conditions key does not exist

```yaml
conditions:
  - "KEY_A == 'VALUE_A'"
  - "OTHER_KEY = 'WRONG_VALUE'"
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
variables: ["MY_VAR"]
namespace: example.org
```
Reason: the variable name does not exist

```yaml
conditions: ["KEY_B == 'VALUE_B'"]
max_value: 1
seconds: 60
variables: ["MY_VAR"]
namespace: example.org
```
Reason: Both variables and conditions must match. In this particular case, only conditions match
