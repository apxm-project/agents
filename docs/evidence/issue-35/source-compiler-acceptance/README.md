# Issue 35 source/compiler acceptance evidence

The nine-gate bundle was run at source revision
`8c371296045481737e0c099c293118a544c5a9a3` on 2026-08-05. Seven gates
passed and two failed. The structured report is `report.json` with SHA-256
`355d19aad11ad41dc428a473edaa36c312a980d7ab7eca8e3bfd21a9a225fa9c`; each
gate log is in `logs/` and its digest is recorded in the report.

The two failures are the real macOS arm64 PyO3 link failure in
`apxm-frontend-python`, with undefined Python symbols including
`_PyBytes_AsString`, `_PyErr_Fetch`, and `__Py_NoneStruct`. It affects
`check-frontend-parity` and `compile-service-canonical`.

No external-source fixture claim is made. The nine-gate bundle uses the
repository-owned TypeScript frontend package gate instead. The `apxm_vllm`
package is present at `tools/apxm_vllm` and `dekk agents test-canonical-only`
passes with the declared Dekk `PYTHONPATH`; a direct invocation without that
configured path is a local invocation error, not a missing-package blocker.
