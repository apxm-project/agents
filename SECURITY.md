# Security policy

Report vulnerabilities privately to the repository maintainers. Include the
affected commit, component, reproduction, and impact. Do not include API keys,
OAuth tokens, model credentials, or other secrets in an issue or patch.

Security-sensitive boundaries in this repository are:

- exact Invocation Admission and capability grants;
- Port binding and target identity verification;
- confinement and resource ceilings;
- request identity, idempotency, and outcome-unknown handling;
- append-only runtime evidence.

Product transports, credential stores, deployment infrastructure, and external
provider services are separate owners and should be reported to their
respective maintainers.
