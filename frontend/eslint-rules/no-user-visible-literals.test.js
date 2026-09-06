import { RuleTester } from "eslint";
import babelParser from "@babel/eslint-parser";
import { noUserVisibleLiterals } from "./no-user-visible-literals.js";

const tester = new RuleTester({
  languageOptions: {
    parser: babelParser,
    parserOptions: {
      requireConfigFile: false,
      babelOptions: { presets: ["@babel/preset-react", "@babel/preset-typescript"] },
    },
  },
});

tester.run("no-user-visible-literals", noUserVisibleLiterals, {
  valid: [
    { code: "const View = () => <p>{intl.formatMessage(messages.ready)}</p>;" },
    { code: "const View = () => <input aria-label={intl.formatMessage(messages.search)} />;" },
    { code: "const View = () => <code>{path}</code>;" },
  ],
  invalid: [
    { code: "const View = () => <p>Ready</p>;", errors: [{ messageId: "text" }] },
    {
      code: 'const View = () => <input placeholder="Search" />;',
      errors: [{ messageId: "attribute" }],
    },
    {
      code: "const View = () => <button aria-label={'Close'} />;",
      errors: [{ messageId: "attribute" }],
    },
    { code: "const View = ({name}) => <p>{`Hello ${name}`}</p>;", errors: [{ messageId: "text" }] },
    {
      code: "const View = ({ready}) => <p>{ready ? 'Ready' : 'Waiting'}</p>;",
      errors: [{ messageId: "text" }],
    },
  ],
});
