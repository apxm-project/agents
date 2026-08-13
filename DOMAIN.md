# agents — domain overview

This repository owns the APXM abstract machine and runtime contracts:

- closed AIS operations and structural IR;
- Python and TypeScript source-first frontends;
- FrontendGraph, AIR, artifact, and source-map contracts;
- exact capability, model, event, and execution Port bindings;
- generic execution, commit, confinement, and evidence semantics.

It does not own a product control plane, Studio, Server, OS host, Auth system,
Telegram/webhook integration, deployment fleet, model zoo, or evaluation
program. Those are downstream composition roots.
