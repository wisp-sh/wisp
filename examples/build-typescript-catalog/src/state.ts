let calls = 0;
export function increment(): number { return ++calls; }
export function current(): number { return calls; }
export const greeting = 'hello ';
