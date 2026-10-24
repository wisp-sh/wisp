// The host throws on jq errors and on denied WASI operations.
export function evaluate(filter: string, json: string): string[];
