# Generic external source fixture

`program.ts` is a product-neutral source fixture submitted through the
source-port boundary. It uses only the public frontend vocabulary, typed
fixture-local bindings, and ordinary source control flow.

The fixture is test input, not a second frontend or compiler path. The source
port captures its typed FrontendGraph, while Rust owns AIR lowering and
validation.
