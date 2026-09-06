import { createContext } from "react";
import type { LocalizationService } from "./service";

export const LocalizationContext = createContext<LocalizationService | null>(null);
