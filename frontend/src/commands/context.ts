export type CommandContextValue = boolean | string | number | null | undefined;
export type CommandContext = Readonly<Record<string, CommandContextValue>>;
type Operator = "!" | "&&" | "||" | "==" | "!=";

type Token =
  | { kind: "identifier"; value: string }
  | { kind: "literal"; value: CommandContextValue }
  | { kind: "operator"; value: Operator }
  | { kind: "paren"; value: "(" | ")" };

function tokenize(expression: string): Token[] {
  const tokens: Token[] = [];
  let offset = 0;
  while (offset < expression.length) {
    const rest = expression.slice(offset);
    const whitespace = /^\s+/.exec(rest);
    if (whitespace !== null) {
      offset += whitespace[0].length;
      continue;
    }
    const operator = /^(&&|\|\||==|!=|!)/.exec(rest);
    if (operator !== null) {
      tokens.push({ kind: "operator", value: operator[1] as Operator });
      offset += operator[0].length;
      continue;
    }
    const paren = /^[()]/.exec(rest);
    if (paren !== null) {
      tokens.push({ kind: "paren", value: paren[0] as "(" | ")" });
      offset += 1;
      continue;
    }
    const quoted = /^(?:"([^"\\]*(?:\\.[^"\\]*)*)"|'([^'\\]*(?:\\.[^'\\]*)*)')/.exec(rest);
    if (quoted !== null) {
      tokens.push({
        kind: "literal",
        value: (quoted[1] ?? quoted[2] ?? "").replace(/\\(['"\\])/g, "$1"),
      });
      offset += quoted[0].length;
      continue;
    }
    const number = /^-?\d+(?:\.\d+)?/.exec(rest);
    if (number !== null) {
      tokens.push({ kind: "literal", value: Number(number[0]) });
      offset += number[0].length;
      continue;
    }
    const identifier = /^[A-Za-z_][A-Za-z0-9_.-]*/.exec(rest);
    if (identifier !== null) {
      const value = identifier[0];
      if (value === "true" || value === "false") {
        tokens.push({ kind: "literal", value: value === "true" });
      } else if (value === "null") {
        tokens.push({ kind: "literal", value: null });
      } else {
        tokens.push({ kind: "identifier", value });
      }
      offset += value.length;
      continue;
    }
    throw new Error(`Unexpected token near '${rest.slice(0, 12)}'.`);
  }
  return tokens;
}

class Parser {
  private offset = 0;
  private readonly tokens: readonly Token[];
  private readonly context: CommandContext;

  constructor(tokens: readonly Token[], context: CommandContext) {
    this.tokens = tokens;
    this.context = context;
  }

  parse(): boolean {
    const result = this.parseOr();
    if (this.offset !== this.tokens.length) throw new Error("Unexpected trailing expression.");
    return Boolean(result);
  }

  private parseOr(): CommandContextValue {
    let value = this.parseAnd();
    while (this.consumeOperator("||")) {
      const right = this.parseAnd();
      value = Boolean(value) || Boolean(right);
    }
    return value;
  }

  private parseAnd(): CommandContextValue {
    let value = this.parseComparison();
    while (this.consumeOperator("&&")) {
      const right = this.parseComparison();
      value = Boolean(value) && Boolean(right);
    }
    return value;
  }

  private parseComparison(): CommandContextValue {
    const left = this.parseUnary();
    if (this.consumeOperator("==")) return left === this.parseComparisonValue();
    if (this.consumeOperator("!=")) return left !== this.parseComparisonValue();
    return left;
  }

  private parseComparisonValue(): CommandContextValue {
    const token = this.tokens[this.offset];
    if (token?.kind === "identifier") {
      this.offset += 1;
      return token.value;
    }
    return this.parseUnary();
  }

  private parseUnary(): CommandContextValue {
    if (this.consumeOperator("!")) return !this.parseUnary();
    const token = this.tokens[this.offset++];
    if (token === undefined) throw new Error("Expected a context value.");
    if (token.kind === "literal") return token.value;
    if (token.kind === "identifier") return this.context[token.value];
    if (token.kind === "paren" && token.value === "(") {
      const value = this.parseOr();
      const closing = this.tokens[this.offset++];
      if (closing?.kind !== "paren" || closing.value !== ")") throw new Error("Expected ')'.");
      return value;
    }
    throw new Error("Expected a context value.");
  }

  private consumeOperator(value: Operator): boolean {
    const token = this.tokens[this.offset];
    if (token?.kind !== "operator" || token.value !== value) return false;
    this.offset += 1;
    return true;
  }
}

export function evaluateEnablement(
  expression: string | null | undefined,
  context: CommandContext,
): boolean {
  if (expression === null || expression === undefined || expression.trim() === "") return true;
  try {
    return new Parser(tokenize(expression), context).parse();
  } catch {
    return false;
  }
}

export function validateWhenClause(expression: string | undefined): boolean {
  if (!expression?.trim()) return true;
  try {
    new Parser(tokenize(expression), {}).parse();
    return true;
  } catch {
    return false;
  }
}
