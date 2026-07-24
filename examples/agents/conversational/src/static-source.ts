// Node build-harness source token for the conversational compiler example.

import { readFileSync } from "node:fs";
import { relative } from "node:path";
import { fileURLToPath } from "node:url";

export function staticSource(url: string): { fileName: string; text: string } {
  const absoluteFileName = fileURLToPath(url);
  return {
    fileName: relative(process.cwd(), absoluteFileName),
    text: readFileSync(absoluteFileName, "utf8"),
  };
}
