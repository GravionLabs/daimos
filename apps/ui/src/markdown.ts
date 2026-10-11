/** A small, safe Markdown subset for the detail pane: headings, paragraphs, lists, code, emphasis. */
export interface Inline {
  text: string;
  bold?: boolean;
  italic?: boolean;
  code?: boolean;
}

export type Block =
  | { type: "heading"; level: number; inlines: Inline[] }
  | { type: "paragraph"; inlines: Inline[] }
  | { type: "list"; items: Inline[][] }
  | { type: "code"; text: string };

const INLINE = /(\*\*[^*]+\*\*|`[^`]+`|\*[^*\s][^*]*\*|_[^_\s][^_]*_|\[[^\]]+\]\([^)]*\))/;

/** Emphasis, code and links (the link text only: the pane never navigates). */
export function parseInline(source: string): Inline[] {
  return source
    .split(INLINE)
    .filter((part) => part !== "")
    .map((part): Inline => {
      if (part.startsWith("**") && part.endsWith("**") && part.length > 4) {
        return { text: part.slice(2, -2), bold: true };
      }
      if (part.startsWith("`") && part.endsWith("`") && part.length > 2) {
        return { text: part.slice(1, -1), code: true };
      }
      if (/^[*_][^*_\s].*[*_]$/.test(part)) return { text: part.slice(1, -1), italic: true };
      const link = /^\[([^\]]+)\]\([^)]*\)$/.exec(part);
      if (link?.[1]) return { text: link[1] };
      return { text: part };
    });
}

export function parseMarkdown(source: string): Block[] {
  const blocks: Block[] = [];
  const lines = source.replace(/\r\n/g, "\n").split("\n");
  let paragraph: string[] = [];
  let list: Inline[][] | null = null;

  const flush = () => {
    if (paragraph.length > 0) {
      blocks.push({ type: "paragraph", inlines: parseInline(paragraph.join(" ")) });
      paragraph = [];
    }
    if (list) {
      blocks.push({ type: "list", items: list });
      list = null;
    }
  };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i] ?? "";
    if (line.trimStart().startsWith("```")) {
      flush();
      const code: string[] = [];
      i += 1;
      while (i < lines.length && !(lines[i] ?? "").trimStart().startsWith("```")) {
        code.push(lines[i] ?? "");
        i += 1;
      }
      blocks.push({ type: "code", text: code.join("\n") });
      continue;
    }
    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    const item = /^\s*[-*]\s+(.*)$/.exec(line);
    if (heading?.[1]) {
      flush();
      blocks.push({
        type: "heading",
        level: heading[1].length,
        inlines: parseInline(heading[2] ?? ""),
      });
    } else if (item) {
      if (paragraph.length > 0) flush();
      list = [...(list ?? []), parseInline(item[1] ?? "")];
    } else if (line.trim() === "") {
      flush();
    } else {
      if (list) flush();
      paragraph.push(line.trim());
    }
  }
  flush();
  return blocks;
}
