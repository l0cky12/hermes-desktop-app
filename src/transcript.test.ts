import { test } from "node:test";
import assert from "node:assert/strict";
import { transcriptMarkdown } from "./transcript.ts";

test("a Session transcript labels each part and skips empty ones", () => {
  const md = transcriptMarkdown([
    { text: "  Fix the build\n", attachments: ["log.txt", "Pasted image 1.png"], tools: [], reply: "" },
    { text: "", attachments: [], tools: ["terminal", "read_file"], reply: "\nDone:\n```sh\nnpm run build\n```" },
    { text: "Thanks", attachments: [], tools: [], reply: "You're welcome." },
  ]);
  assert.equal(md, [
    "## User", "Fix the build", "Attachments: log.txt, Pasted image 1.png",
    "## Hermes", "Tools: terminal, read_file", "Done:\n```sh\nnpm run build\n```",
    "## User", "Thanks",
    "## Hermes", "You're welcome.",
  ].join("\n\n") + "\n");
});
