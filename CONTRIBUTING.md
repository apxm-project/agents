# Contributing

Work through Dekk so the repository environment and target directory remain
consistent.

```bash
dekk agents doctor
dekk agents check
dekk agents test-program-source
dekk agents test-python-frontend
dekk agents test-typescript-frontend
dekk agents test-frontend-examples
```

AIS definitions are owned by `crates/machine/ais`. After changing a TableGen
definition or generated C++ shim, run:

```bash
dekk agents build-dialect
dekk agents codegen
```

Keep product integrations, deployment recipes, credentials, benchmark output,
and generated evidence outside this repository. Do not commit `.apxm/`, local
build output, secrets, or generated artifacts.
