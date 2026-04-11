import React from "react";

type Token = { type: string; text: string };

const PY_KEYWORDS = new Set([
  "def", "class", "return", "yield", "import", "from", "as", "if", "elif",
  "else", "for", "while", "try", "except", "finally", "with", "raise",
  "pass", "break", "continue", "and", "or", "not", "in", "is", "lambda",
  "global", "nonlocal", "assert", "del", "True", "False", "None", "async", "await",
]);

const PY_BUILTINS = new Set([
  "print", "len", "range", "int", "str", "float", "bool", "list", "dict",
  "set", "tuple", "type", "isinstance", "hasattr", "getattr", "setattr",
  "super", "self", "enumerate", "zip", "map", "filter", "sorted", "reversed",
  "any", "all", "min", "max", "sum", "abs", "open", "format",
]);

const MLIR_KEYWORDS = new Set([
  "module", "func", "return", "cf", "scf", "memref", "tensor", "vector",
  "arith", "index", "builtin",
]);

function tokenizePython(line: string): Token[] {
  const tokens: Token[] = [];
  let i = 0;
  while (i < line.length) {
    // Whitespace
    if (/\s/.test(line[i])) {
      let j = i;
      while (j < line.length && /\s/.test(line[j])) j++;
      tokens.push({ type: "ws", text: line.slice(i, j) });
      i = j;
      continue;
    }

    // Comment
    if (line[i] === "#") {
      tokens.push({ type: "comment", text: line.slice(i) });
      break;
    }

    // Decorator
    if (line[i] === "@") {
      let j = i + 1;
      while (j < line.length && /[\w.]/.test(line[j])) j++;
      tokens.push({ type: "decorator", text: line.slice(i, j) });
      i = j;
      continue;
    }

    // Strings (triple or single/double quoted)
    if (
      (line[i] === '"' || line[i] === "'") &&
      line.slice(i, i + 3) === line[i].repeat(3)
    ) {
      const q = line[i].repeat(3);
      const end = line.indexOf(q, i + 3);
      const j = end >= 0 ? end + 3 : line.length;
      tokens.push({ type: "string", text: line.slice(i, j) });
      i = j;
      continue;
    }
    if (line[i] === '"' || line[i] === "'") {
      const q = line[i];
      let j = i + 1;
      while (j < line.length && line[j] !== q) {
        if (line[j] === "\\") j++;
        j++;
      }
      if (j < line.length) j++;
      tokens.push({ type: "string", text: line.slice(i, j) });
      i = j;
      continue;
    }

    // f-string prefix
    if ((line[i] === "f" || line[i] === "F") && (line[i + 1] === '"' || line[i + 1] === "'")) {
      const q = line[i + 1];
      let j = i + 2;
      while (j < line.length && line[j] !== q) {
        if (line[j] === "\\") j++;
        j++;
      }
      if (j < line.length) j++;
      tokens.push({ type: "string", text: line.slice(i, j) });
      i = j;
      continue;
    }

    // Numbers
    if (/[0-9]/.test(line[i])) {
      let j = i;
      while (j < line.length && /[0-9._xXoObBeE]/.test(line[j])) j++;
      tokens.push({ type: "number", text: line.slice(i, j) });
      i = j;
      continue;
    }

    // Identifiers / keywords
    if (/[a-zA-Z_]/.test(line[i])) {
      let j = i;
      while (j < line.length && /[\w]/.test(line[j])) j++;
      const word = line.slice(i, j);
      if (PY_KEYWORDS.has(word)) {
        tokens.push({ type: "keyword", text: word });
      } else if (PY_BUILTINS.has(word)) {
        tokens.push({ type: "builtin", text: word });
      } else if (j < line.length && line[j] === "(") {
        tokens.push({ type: "function", text: word });
      } else {
        tokens.push({ type: "ident", text: word });
      }
      i = j;
      continue;
    }

    // Operators / punctuation
    tokens.push({ type: "punct", text: line[i] });
    i++;
  }
  return tokens;
}

function tokenizeMLIR(line: string): Token[] {
  const tokens: Token[] = [];
  let i = 0;
  while (i < line.length) {
    if (/\s/.test(line[i])) {
      let j = i;
      while (j < line.length && /\s/.test(line[j])) j++;
      tokens.push({ type: "ws", text: line.slice(i, j) });
      i = j;
      continue;
    }

    // Comment
    if (line.slice(i, i + 2) === "//") {
      tokens.push({ type: "comment", text: line.slice(i) });
      break;
    }

    // SSA values %name
    if (line[i] === "%") {
      let j = i + 1;
      while (j < line.length && /[\w]/.test(line[j])) j++;
      tokens.push({ type: "ssa", text: line.slice(i, j) });
      i = j;
      continue;
    }

    // Attributes with ais. prefix
    if (line.slice(i, i + 4) === "ais.") {
      let j = i + 4;
      while (j < line.length && /[\w]/.test(line[j])) j++;
      tokens.push({ type: "ais-op", text: line.slice(i, j) });
      i = j;
      continue;
    }

    // Strings
    if (line[i] === '"') {
      let j = i + 1;
      while (j < line.length && line[j] !== '"') {
        if (line[j] === "\\") j++;
        j++;
      }
      if (j < line.length) j++;
      tokens.push({ type: "string", text: line.slice(i, j) });
      i = j;
      continue;
    }

    // Types !ais.token etc
    if (line[i] === "!") {
      let j = i + 1;
      while (j < line.length && /[\w.]/.test(line[j])) j++;
      tokens.push({ type: "type", text: line.slice(i, j) });
      i = j;
      continue;
    }

    // @name symbols
    if (line[i] === "@") {
      let j = i + 1;
      while (j < line.length && /[\w]/.test(line[j])) j++;
      tokens.push({ type: "symbol", text: line.slice(i, j) });
      i = j;
      continue;
    }

    // Keywords
    if (/[a-zA-Z_]/.test(line[i])) {
      let j = i;
      while (j < line.length && /[\w.]/.test(line[j])) j++;
      const word = line.slice(i, j);
      const base = word.split(".")[0];
      if (MLIR_KEYWORDS.has(base) || word === "func.func" || word === "func.return") {
        tokens.push({ type: "keyword", text: word });
      } else {
        tokens.push({ type: "ident", text: word });
      }
      i = j;
      continue;
    }

    // Numbers
    if (/[0-9]/.test(line[i])) {
      let j = i;
      while (j < line.length && /[0-9.]/.test(line[j])) j++;
      tokens.push({ type: "number", text: line.slice(i, j) });
      i = j;
      continue;
    }

    tokens.push({ type: "punct", text: line[i] });
    i++;
  }
  return tokens;
}

const TOKEN_CLASSES: Record<string, string> = {
  keyword: "sh-keyword",
  builtin: "sh-builtin",
  decorator: "sh-decorator",
  string: "sh-string",
  comment: "sh-comment",
  number: "sh-number",
  function: "sh-function",
  ssa: "sh-ssa",
  "ais-op": "sh-ais-op",
  type: "sh-type",
  symbol: "sh-symbol",
  punct: "sh-punct",
};

function renderTokens(tokens: Token[]): React.ReactNode[] {
  return tokens.map((t, i) => {
    const cls = TOKEN_CLASSES[t.type];
    return cls ? (
      <span key={i} className={cls}>{t.text}</span>
    ) : (
      <React.Fragment key={i}>{t.text}</React.Fragment>
    );
  });
}

export function highlightLine(
  line: string,
  language: string,
): React.ReactNode {
  if (language === "python") {
    return <>{renderTokens(tokenizePython(line))}</>;
  }
  if (language === "mlir") {
    return <>{renderTokens(tokenizeMLIR(line))}</>;
  }
  return line || "\n";
}
