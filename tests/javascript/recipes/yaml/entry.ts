import { parse, parseAllDocuments, stringify } from './upstream/src/index';
// JSON envelopes preserve upstream errors without trapping the Component.
function attempt(fn: () => unknown): string {
  try { return JSON.stringify({ ok: true, value: fn() }); }
  catch (error) { return JSON.stringify({ ok: false, error: String(error) }); }
}
export function parseYaml(text: string): string {
  return attempt(() => parse(text, { maxAliasCount: 100 }));
}
export function parseDocuments(text: string): string {
  return attempt(() => parseAllDocuments(text).map(doc => {
    if (doc.errors.length) throw doc.errors[0];
    return doc.toJS({ maxAliasCount: 100 });
  }));
}
export function stringifyJson(json: string): string {
  return attempt(() => stringify(JSON.parse(json)));
}
