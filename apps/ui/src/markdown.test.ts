import { parseInline, parseMarkdown } from "./markdown";

describe("parseInline", () => {
  it("recognises bold, italic, code and link text", () => {
    expect(parseInline("a **b** *c* _d_ `e` [f](https://x.test)")).toEqual([
      { text: "a " },
      { text: "b", bold: true },
      { text: " " },
      { text: "c", italic: true },
      { text: " " },
      { text: "d", italic: true },
      { text: " " },
      { text: "e", code: true },
      { text: " " },
      { text: "f" },
    ]);
  });

  it("leaves HTML and stray markers as plain text", () => {
    expect(parseInline("<b>x</b> 2 * 3")).toEqual([{ text: "<b>x</b> 2 * 3" }]);
  });
});

describe("parseMarkdown", () => {
  it("parses headings, paragraphs, lists and code blocks", () => {
    const blocks = parseMarkdown(
      "# Title\n\nSome **text**\nacross lines.\n\n- one\n- two\n\n```\nlet a = 1;\n```\n",
    );
    expect(blocks).toEqual([
      { type: "heading", level: 1, inlines: [{ text: "Title" }] },
      {
        type: "paragraph",
        inlines: [{ text: "Some " }, { text: "text", bold: true }, { text: " across lines." }],
      },
      { type: "list", items: [[{ text: "one" }], [{ text: "two" }]] },
      { type: "code", text: "let a = 1;" },
    ]);
  });

  it("handles CRLF, an unterminated code block and empty input", () => {
    expect(parseMarkdown("a\r\nb")).toEqual([{ type: "paragraph", inlines: [{ text: "a b" }] }]);
    expect(parseMarkdown("```\ncode")).toEqual([{ type: "code", text: "code" }]);
    expect(parseMarkdown("")).toEqual([]);
  });

  it("starts a new block when a list follows a paragraph and the other way round", () => {
    expect(parseMarkdown("intro\n- a\nafter")).toEqual([
      { type: "paragraph", inlines: [{ text: "intro" }] },
      { type: "list", items: [[{ text: "a" }]] },
      { type: "paragraph", inlines: [{ text: "after" }] },
    ]);
  });
});
