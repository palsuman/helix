import "@testing-library/jest-dom/vitest";
import * as axeMatchers from "vitest-axe/matchers.js";
import { expect } from "vitest";

expect.extend(axeMatchers);
