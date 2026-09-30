/** Token counts for a Turn or a Session; `input` counts every prompt token, cached or not. */
export type Usage = { input: number; output: number; cached: number };

/** A Session record's usage fields, as the API server lists them. */
export type SessionTotals = {
  input_tokens?: number | null;
  output_tokens?: number | null;
  cache_read_tokens?: number | null;
  cache_write_tokens?: number | null;
  estimated_cost_usd?: number | null;
};

const count = (n: unknown) => (typeof n === "number" && n > 0 ? n : 0);
const nonEmpty = (u: Usage) => (u.input || u.output ? u : null);

/** A `run.completed` event's `usage`. There `input_tokens` already includes cache reads and
 * writes (Hermes's `session_prompt_tokens`); an empty `{}` means the Run reported none. */
export function runUsage(usage: unknown): Usage | null {
  if (typeof usage !== "object" || usage === null) return null;
  const u = usage as Record<string, unknown>;
  return nonEmpty({ input: count(u.input_tokens), output: count(u.output_tokens), cached: count(u.cache_read_tokens) });
}

/** A Session record's running totals. Unlike a Run's, its `input_tokens` excludes cache reads and
 * writes (Hermes stores them in their own columns), so they are added back in. */
export function sessionUsage(s: SessionTotals): Usage | null {
  const cached = count(s.cache_read_tokens);
  return nonEmpty({ input: count(s.input_tokens) + cached + count(s.cache_write_tokens), output: count(s.output_tokens), cached });
}

export function addUsage(a: Usage | null, b: Usage): Usage {
  return a ? { input: a.input + b.input, output: a.output + b.output, cached: a.cached + b.cached } : b;
}

const COMPACT = new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 });
/** 950, 1.2k, 3.4M. */
export const compact = (n: number) => COMPACT.format(n).replace("K", "k");

/** "1.2k in · 350 out · 42% cached": the share of input tokens read from the prompt cache. */
export function describeUsage(u: Usage): string {
  const parts = [`${compact(u.input)} in`, `${compact(u.output)} out`];
  if (u.input) parts.push(`${Math.round((u.cached / u.input) * 100)}% cached`);
  return parts.join(" · ");
}

/** Dollars as Hermes estimates them from its price list. Hermes records a model it has no price for
 * as $0 (and never fills `actual_cost_usd`), so $0 shows as no cost at all. */
export function describeCost(usd: number | null | undefined): string | null {
  return typeof usd === "number" && usd > 0 ? `≈ $${usd.toFixed(usd >= 1 ? 2 : 4)}` : null;
}
