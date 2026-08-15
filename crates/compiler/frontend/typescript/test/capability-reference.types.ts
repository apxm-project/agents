// The Capability-reference property, proved where it is decided: the compiler.
//
// `Tool` and `Capability` accept a builtin id the generated catalogue mints or
// the object a handler declaration hands back, and nothing else. That is a
// property of the type, not of a check some pass runs, so the proof is a file
// that must compile — and whose `@ts-expect-error` lines must keep erroring.
// A widened `CapabilityReference` makes the invented references below legal,
// which turns each directive into an unused one and fails `tsc` just as loudly
// as an accepted reference should.
//
// This file is typechecked by `tsconfig.json` here and deliberately runs
// nothing: vitest collects `*.test.ts`, so there is no suite to keep in step
// with a fact the compiler already settles.

import { Capability, Tool } from "../src/index.ts";
import { READ } from "../src/capabilities.ts";

type Input = { query: string };
type Output = { answer: string };

// A builtin id, written as the literal the catalogue mints and as the symbol
// naming it. `examples/agents/coder` binds `read` this way: it is a builtin, so
// no package ships a handler for it and there is no declaration to name.
export const BuiltinLiteral = Tool<Input, Output>("read");
export const BuiltinSymbol = Tool<Input, Output>(READ);

// The other arm: what a handler declaration returns, carrying the id it
// implements. `@apxm/agent-packaging`'s `Tool.define` returns this shape.
const proposeEdit = { capabilityId: "edit" };
export const ShippedHandler = Tool<Input, Output>(proposeEdit);
export const ShippedCapability = Capability<Input, Output>(proposeEdit);

// @ts-expect-error an invented bare string is neither a catalogue id nor a handler
export const InventedTool = Tool<Input, Output>("cap.search");
// @ts-expect-error `edit` is an id a package ships; only the declaration names it
export const BareShippedTool = Tool<Input, Output>("edit");
// @ts-expect-error the same closed set governs `Capability`
export const InventedCapability = Capability<Input, Output>("cap.search");
