# apxm-designer

AI-first app builder. Describe an app -> all screens x all states in verified HTML -> any framework.

## Phases

| Phase | Status | What it does |
|-------|--------|--------------|
| 1: DISCOVER | ✅ | 4 specialists + debate -> 6 spec artifacts |
| 2: GENERATE | 🔜 | N×V HTML files, 2-stage verification |
| 3: CONVERT | 🔜 | HTML -> framework code |
| 4: TEST | 🔜 | Full test suite |

## Usage

```bash
dekk apxm execute examples/workflows/designer/phase1/discover.ais
```

## The Core Insight

Every other AI app builder (Replit, Firebase Studio, Bolt, v0) goes straight to code.
They skip the design phase. Result: missing empty states, inconsistent components, 
undocumented interactions. You find the gaps in QA.

apxm-designer enforces a structured design phase — ALL screens × ALL states in verified HTML
before a single line of implementation code. The HTML IS the design, the spec, and the 
conversion source.
