import { test } from "node:test";
import assert from "node:assert/strict";
import { exportHtml, exportMarkdown, fileName, transcriptTurns } from "./export.ts";

const messages = [
  { role: "user", content: [{ type: "text", text: "<script>alert('x')</script> & `a<b`" }, { type: "image_url" }] },
  { role: "assistant", content: "", tool_calls: [{ function: { name: "terminal" } }, {}] },
  { role: "assistant", content: 'Done:\n```sh\necho "</pre><img src=x onerror=1>"\n```' },
  { role: "tool", content: "ignored" },
  { role: "user", content: "second" },
];

test("messages become Turns like the chat view shows them", () => {
  assert.deepEqual(transcriptTurns(messages), [
    { user: "<script>alert('x')</script> & `a<b`\n[image]", tools: ["terminal", "tool"], reply: 'Done:\n```sh\necho "</pre><img src=x onerror=1>"\n```' },
    { user: "second", tools: [], reply: "" },
  ]);
  assert.deepEqual(transcriptTurns([{ role: "assistant", content: "hi" }]), [{ user: "", tools: [], reply: "hi" }]);
});

test("the HTML export escapes everything the Session said", () => {
  const html = exportHtml(`Title <b>"&'`, transcriptTurns(messages));
  assert.ok(html.includes("<title>Title &lt;b&gt;&quot;&amp;&#39;</title>"));
  assert.ok(html.includes("&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt; &amp; <code>a&lt;b</code>"));
  assert.ok(html.includes('<pre><code>echo &quot;&lt;/pre&gt;&lt;img src=x onerror=1&gt;&quot;\n</code></pre>'));
  assert.ok(!/<script|<img|<b>/.test(html), "no markup from the transcript survives");
  assert.ok(html.includes("Tools: terminal, tool"));
});

test("the Markdown export quotes the user and keeps replies as written", () => {
  const md = exportMarkdown("T", transcriptTurns([{ role: "user", content: "a\n\nb" }, { role: "assistant", content: "**ok**" }]));
  assert.equal(md, "# T\n\n**You**\n\n> a\n>\n> b\n\n**Hermes**\n\n**ok**\n");
});

test("file names drop characters no OS allows", () => {
  assert.equal(fileName('a/b: "c"?  d', "md"), "a b c d.md");
  assert.equal(fileName("///", "html"), "session.html");
});
