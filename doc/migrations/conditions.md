# New condition syntax 

With `limitador-server` version `1.0.0` (and the `limitador` crate version `0.3.0`), the syntax for `condition`s within 
`limit` definitions has changed.

# Note! This synthax has been deprecated as of version `2.0.0`

## Changes when working with Limitador Server versions `1.x`

### The new syntax

The new syntax formalizes what part of an expression is the identifier and which is the value to test against. 
Identifiers are simple string value, while string literals are to be demarcated by single quotes (`'`) or double quotes
(`"`) so that `foo == " bar"` now makes it explicit that the value is to be prefixed with a space character.

A few remarks:
 - Only `string` values are supported, as that's what they really are
 - There is no escape character sequence supported in string literals
 - A new operator has been added, `!=`

### The issue with the deprecated syntax

The previous syntax wouldn't differentiate between values and the identifier, so that `foo == bar` was valid. In this
case `foo` was the identifier of the variable, while `bar` was the value to evaluate it against. Whitespaces before and
after the operator `==` would be equally important. SO that `foo  ==  bar` would test for a `foo ` variable being equal
to ` bar` where the trailing whitespace after the identifier, and the one prefixing the value, would have been
evaluated.
