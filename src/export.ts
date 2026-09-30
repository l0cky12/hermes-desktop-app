/** A Session's transcript as Turns, and its Export as a self-contained HTML page or Markdown. Pure: no DOM. */

export type Message = { role: string; content: unknown; tool_calls?: { function?: { name?: string } }[] | null };
export type TranscriptTurn = { user: string; tools: string[]; reply: string };

export function contentText(content: unknown): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .map((part) => (part?.type === "text" ? String(part.text) : part?.type === "image_url" ? "[image]" : ""))
    .filter(Boolean)
    .join("\n");
}

/** Each user message starts a Turn; assistant messages (and their tool calls) fill its reply. */
export function transcriptTurns(messages: Message[]): TranscriptTurn[] {
  const turns: TranscriptTurn[] = [];
  let turn: TranscriptTurn | undefined;
  for (const m of messages) {
    const text = contentText(m.content);
    if (m.role === "user") turns.push((turn = { user: text, tools: [], reply: "" }));
    if (m.role !== "assistant") continue;
    if (!turn) turns.push((turn = { user: "", tools: [], reply: "" }));
    turn.tools.push(...(m.tool_calls ?? []).map((c) => c.function?.name ?? "tool"));
    if (text) turn.reply += (turn.reply ? "\n\n" : "") + text;
  }
  return turns;
}

export const FENCE = /```(?:[^\n`]*\n)?([\s\S]*?)```/;

/** Splits text the way the chat view shows it: fenced code, `inline code`, and plain text. */
export type TextPart = { kind: "pre" | "code" | "text"; text: string };

export function textParts(text: string): TextPart[] {
  return text.split(new RegExp(FENCE, "g")).flatMap((part, i): TextPart[] =>
    i % 2 ? [{ kind: "pre", text: part }] : part.split(/`([^`\n]+)`/).map((s, j) => ({ kind: j % 2 ? "code" : "text", text: s })),
  );
}

const escape = (s: string) =>
  s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);

const htmlText = (text: string) =>
  textParts(text)
    .map((p) => (p.kind === "pre" ? `<pre><code>${escape(p.text)}</code></pre>` : p.kind === "code" ? `<code>${escape(p.text)}</code>` : escape(p.text)))
    .join("");

const STYLE = `body{font:15px/1.5 system-ui,sans-serif;max-width:48rem;margin:2rem auto;padding:0 1rem;color:#1d1d24}
.user,.reply{white-space:pre-wrap;overflow-wrap:anywhere}.user{background:#f3ecd9;padding:.6rem .9rem;border-radius:.6rem}
.tools{color:#6b6b7b;font-size:.9rem;margin:.4rem 0}.turn{margin-bottom:1.5rem}
pre{background:#1d1d24;color:#f4f4f7;padding:.7rem;border-radius:.4rem;overflow-x:auto;white-space:pre}
code{font:.9em ui-monospace,monospace;background:rgb(0 0 0/.07);padding:0 .25rem;border-radius:.25rem}pre code{background:none;padding:0}`;

export function exportHtml(title: string, turns: TranscriptTurn[]): string {
  const body = turns
    .map((t) =>
      [
        '<article class="turn">',
        t.user && `<div class="user">${htmlText(t.user)}</div>`,
        t.tools.length && `<div class="tools">Tools: ${escape(t.tools.join(", "))}</div>`,
        t.reply && `<div class="reply">${htmlText(t.reply)}</div>`,
        "</article>",
      ].filter(Boolean).join("\n"),
    )
    .join("\n");
  return `<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>${escape(title)}</title><style>${STYLE}</style></head>
<body>
<h1>${escape(title)}</h1>
${body}
</body>
</html>
`;
}

export function exportMarkdown(title: string, turns: TranscriptTurn[]): string {
  const quote = (s: string) => s.split("\n").map((l) => (l ? `> ${l}` : ">")).join("\n");
  const blocks = turns.flatMap((t) => [
    ...(t.user ? [`**You**\n\n${quote(t.user)}`] : []),
    ...(t.tools.length ? [`_Tools: ${t.tools.join(", ")}_`] : []),
    ...(t.reply ? [`**Hermes**\n\n${t.reply}`] : []),
  ]);
  return `# ${title}\n\n${blocks.join("\n\n")}\n`;
}

/** A file name the save dialog can offer on every OS. */
export const fileName = (title: string, ext: string) =>
  `${title.replace(/[\\/:*?"<>|\x00-\x1f]+/g, " ").replace(/\s+/g, " ").trim().slice(0, 80) || "session"}.${ext}`;
