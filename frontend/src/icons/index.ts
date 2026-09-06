export { Icon, type IconProps } from "./Icon";
export { injectIconStyles } from "./styles";
export type { IconId, IconSize } from "../generated/icons.gen";
export {
  BUILTIN_FILE_ICON_THEMES,
  DEFAULT_FILE_ICON_THEME,
  resolveFileIcon,
  resolveFolderIcon,
  type FileIconTheme,
} from "./fileIcons";
export {
  BUILTIN_PRODUCT_ICON_THEMES,
  DEFAULT_PRODUCT_ICON_THEME,
  type ProductIconTheme,
} from "./productIcons";