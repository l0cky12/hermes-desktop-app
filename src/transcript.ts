/** What the chat view shows for one Turn, reduced to text. */
export type TurnText = { text: string; attachments: string[]; tools: string[]; reply: string };

/** A whole Session as Markdown: each Turn's user part and Hermes's reply under their own heading, tools by name only. */
export function transcriptMarkdown(turns: TurnText[]): string {
  return turns
    .flatMap((t) => {
      const user = [t.text.trim(), t.attachments.length ? `Attachments: ${t.attachments.join(", ")}` : ""].filter(Boolean);
      const hermes = [t.tools.length ? `Tools: ${t.tools.join(", ")}` : "", t.reply.trim()].filter(Boolean);
      return [...(user.length ? ["## User", ...user] : []), ...(hermes.length ? ["## Hermes", ...hermes] : [])];
    })
    .join("\n\n") + "\n";
}
