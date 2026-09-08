import { EditorSurface, type EditorSurfaceProps } from "./EditorSurface";
import { defaultMonacoLoader } from "./monacoLoader";

type EditorEntryProps = Omit<EditorSurfaceProps, "monacoLoader">;

export function EditorEntry(props: EditorEntryProps) {
  return <EditorSurface {...props} monacoLoader={defaultMonacoLoader} />;
}