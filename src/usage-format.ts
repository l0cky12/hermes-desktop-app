/** Numbers in the Usage view. */

const CENTS = new Intl.NumberFormat("en-US", { style: "currency", currency: "USD" });
const SMALL = new Intl.NumberFormat("en-US", { style: "currency", currency: "USD", maximumFractionDigits: 4 });
/** $1,234.50; under a dollar, up to four places, so a few cents of tokens don't read as $0.00. */
export const dollars = (usd: number) => (usd < 1 ? SMALL : CENTS).format(usd);

const COMPACT = new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 1 });
/** 950, 1.2k, 3.4M. */
export const tokens = (n: number) => COMPACT.format(n).replace("K", "k");

/** Whole percent of a limit window still left, from `hermes usage`'s `used_percent`. */
export const percentLeft = (usedPercent: number) => Math.round(Math.min(100, Math.max(0, 100 - usedPercent)));
