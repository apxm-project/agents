# Issue 35 source/compiler acceptance evidence

The nine-gate bundle was run at source revision
`eb99c0fd4f3f84d4320d7d81bae287386ad51945` on 2026-08-05. Seven gates
passed and two failed. The structured report is `report.json` with SHA-256
`31d9352e4b183219355a74e6114105975b8ae3ff40bcc1ede785ce11b3b26d4e`; each
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
