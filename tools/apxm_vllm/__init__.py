"""Operator-side APXM/vLLM contract shared by the `tools/scripts` entrypoints.

`contract` owns the operational names (env vars, routes, dataclasses,
`build_layout()`); `data_config` resolves the `.apxm/` data buckets. This
package is operator tooling only: canonical Agent Program authoring is
`apxm_program` under the Python authoring frontend and never imports this.
"""
