const TRANSLATABLE_ATTRIBUTES = new Set([
  "aria-label",
  "aria-placeholder",
  "alt",
  "placeholder",
  "title",
]);

function visibleText(value) {
  return value.replace(/\s+/g, " ").trim();
}

function literalFromExpression(expression) {
  if (expression?.type === "StringLiteral" || expression?.type === "Literal") {
    return typeof expression.value === "string" ? expression.value : null;
  }
  if (expression?.type === "TemplateLiteral" && expression.expressions.length === 0) {
    return expression.quasis.map((quasi) => quasi.value.cooked ?? "").join("");
  }
  return null;
}

function containsVisibleLiteral(expression) {
  const literal = literalFromExpression(expression);
  if (literal !== null) return visibleText(literal) !== "";
  if (expression?.type === "TemplateLiteral") {
    return expression.quasis.some((quasi) => visibleText(quasi.value.cooked ?? "") !== "");
  }
  if (expression?.type === "BinaryExpression") {
    return containsVisibleLiteral(expression.left) || containsVisibleLiteral(expression.right);
  }
  if (expression?.type === "LogicalExpression") {
    return containsVisibleLiteral(expression.right);
  }
  if (expression?.type === "ConditionalExpression") {
    return (
      containsVisibleLiteral(expression.consequent) || containsVisibleLiteral(expression.alternate)
    );
  }
  if (expression?.type === "ArrayExpression") {
    return expression.elements.some(containsVisibleLiteral);
  }
  return false;
}

export const noUserVisibleLiterals = {
  meta: {
    type: "problem",
    docs: { description: "require catalog-backed user-visible strings in JSX" },
    schema: [],
    messages: {
      text: "User-visible JSX text must resolve through the localization catalog.",
      attribute: "The '{{name}}' attribute must resolve through the localization catalog.",
    },
  },
  create(context) {
    return {
      JSXText(node) {
        if (visibleText(node.value)) context.report({ node, messageId: "text" });
      },
      JSXAttribute(node) {
        const name = node.name?.name;
        if (typeof name !== "string" || !TRANSLATABLE_ATTRIBUTES.has(name)) return;
        if (node.value?.type === "StringLiteral" || node.value?.type === "Literal") {
          if (visibleText(String(node.value.value ?? ""))) {
            context.report({ node, messageId: "attribute", data: { name } });
          }
          return;
        }
        if (node.value?.type === "JSXExpressionContainer") {
          if (containsVisibleLiteral(node.value.expression)) {
            context.report({ node, messageId: "attribute", data: { name } });
          }
        }
      },
      JSXExpressionContainer(node) {
        if (containsVisibleLiteral(node.expression) && node.parent?.type !== "JSXAttribute") {
          context.report({ node, messageId: "text" });
        }
      },
    };
  },
};
