/** Dependency kind carried by a graph edge — mirrors apxm.constants.DependencyType. */
export type DependencyType = "Data" | "Control" | "Effect";

const DEPENDENCY_TYPES: ReadonlySet<DependencyType> = new Set([
  "Data",
  "Control",
  "Effect",
]);

/** Normalize a case-insensitive dependency string to its canonical form. */
export function normalizeDependencyType(value: DependencyType | string): DependencyType {
  const found = Array.from(DEPENDENCY_TYPES).find(
    (candidate) => candidate.toLowerCase() === value.toLowerCase(),
  );
  if (!found) {
    const allowed = Array.from(DEPENDENCY_TYPES).sort().join(", ");
    throw new Error(`invalid dependency '${value}', expected one of: ${allowed}`);
  }
  return found;
}

/** Parameter type names accepted by the APXM runtime (mirrors VALID_PARAM_TYPES). */
export type ParamType = "str" | "int" | "float" | "bool" | "json";

export const VALID_PARAM_TYPES: ReadonlySet<ParamType> = new Set([
  "str",
  "int",
  "float",
  "bool",
  "json",
]);
