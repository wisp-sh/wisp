import { pathToFileURL } from 'node:url';
import assert from 'node:assert/strict';
const { operations } = await import(process.argv[2] ? pathToFileURL(process.argv[2]) : '@wisp/async-values');
const future = await operations.delayed({label: 'nested', values: new Uint32Array([3, 5])});

const value = await future;
assert.equal(value.label, 'nested'); assert.deepEqual([...value.values], [3, 5]);
const stream = await operations.words(['hello', 'βeta']);

const values = [];
for await (const value of stream) values.push(value);
assert.deepEqual(values, ['hello', 'βeta']);
const cancelled = await operations.words(['one', 'two']);
await cancelled.return();
for (let attempt = 0; operations.active() && attempt < 100; attempt++) await new Promise(resolve => setTimeout(resolve, 1));
assert.equal(operations.active(), 0);
console.log('future record + stream strings + stream cancellation passed');
