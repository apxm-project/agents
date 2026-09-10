// Projects finite JSON input contracts from the TypeScript checker's resolved types.
import ts from "typescript";
import type { EntrypointInputSchema } from "./generated/frontend-records.js";
import {
  INPUT_SCHEMA_TYPE_ARRAY,
  INPUT_SCHEMA_TYPE_BOOLEAN,
  INPUT_SCHEMA_TYPE_NULL,
  INPUT_SCHEMA_TYPE_NUMBER,
  INPUT_SCHEMA_TYPE_OBJECT,
  INPUT_SCHEMA_TYPE_STRING,
} from "./generated/frontend-graph.js";

// Mirror of crates/machine/program/src/input_schema.rs.
const MAX_SCHEMA_DEPTH = 32;
const MAX_SCHEMA_NODES = 512;

/** A missing schema leaves unsupported or unresolved input types ineligible for schema-bound admission. */
export function checkedInputSchema(checker: ts.TypeChecker, location: ts.Node, input: ts.Type): EntrypointInputSchema | undefined {
  const ancestors = new Set<ts.Type>();
  let nodes = 0;
  const project = (type: ts.Type, depth: number, optional = false): EntrypointInputSchema | undefined => {
    nodes += 1;
    if (depth > MAX_SCHEMA_DEPTH || nodes > MAX_SCHEMA_NODES || ancestors.has(type)) return undefined;
    if (optional && type.isUnion()) {
      const present = type.types.filter((part) => (part.flags & ts.TypeFlags.Undefined) === 0);
      if (present.length !== type.types.length) {
        if (present.length === 1) return project(present[0], depth);
        if (present.length === 2 && present.every((part) => (part.flags & ts.TypeFlags.BooleanLiteral) !== 0)) return { type: INPUT_SCHEMA_TYPE_BOOLEAN };
        return undefined;
      }
    }
    if (type.flags & ts.TypeFlags.String) return { type: INPUT_SCHEMA_TYPE_STRING };
    if (type.flags & ts.TypeFlags.Number) return { type: INPUT_SCHEMA_TYPE_NUMBER };
    if (type.flags & ts.TypeFlags.Boolean) return { type: INPUT_SCHEMA_TYPE_BOOLEAN };
    if (type.flags & ts.TypeFlags.Null) return { type: INPUT_SCHEMA_TYPE_NULL };
    if (!(type.flags & ts.TypeFlags.Object) || type.isUnionOrIntersection() || checker.isTupleType(type)) return undefined;
    ancestors.add(type);
    try {
      if (checker.isArrayType(type)) {
        const element = checker.getTypeArguments(type as ts.TypeReference)[0];
        const items = element && project(element, depth + 1);
        return items ? { type: INPUT_SCHEMA_TYPE_ARRAY, items } : undefined;
      }
      if (
        (type.getSymbol()?.flags ?? 0) & ts.SymbolFlags.Class ||
        checker.getSignaturesOfType(type, ts.SignatureKind.Call).length ||
        checker.getSignaturesOfType(type, ts.SignatureKind.Construct).length ||
        checker.getIndexInfosOfType(type).length
      ) return undefined;
      const properties: Record<string, EntrypointInputSchema> = Object.create(null);
      const required: string[] = [];
      for (const property of checker.getPropertiesOfType(type)) {
        if (property.getName().startsWith("__@")) return undefined;
        const isOptional = (property.flags & ts.SymbolFlags.Optional) !== 0;
        const value = project(checker.getTypeOfSymbolAtLocation(property, location), depth + 1, isOptional);
        if (!value) return undefined;
        properties[property.getName()] = value;
        if (!isOptional) required.push(property.getName());
      }
      return { type: INPUT_SCHEMA_TYPE_OBJECT, properties, required, additionalProperties: false };
    } finally {
      ancestors.delete(type);
    }
  };
  return project(input, 0);
}
