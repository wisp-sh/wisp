// A pinned JCO limitation probe. Run in its own process; abandoning the future
// leaves a guest writer alive, so process exit is part of the reproduction.
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
const { operations } = await import(pathToFileURL(process.argv[2]));
const future = await operations.delayed({label: 'unread', values: new Uint32Array([1])});
const methods = Object.getOwnPropertyNames(Object.getPrototypeOf(future));
assert.equal(typeof future.then, 'function');
assert.equal(typeof future.close, 'undefined');
assert.equal(typeof future.cancel, 'undefined');
assert.equal(typeof future[Symbol.dispose], 'undefined');
assert.equal(typeof future[Symbol.asyncDispose], 'undefined');
assert.equal(operations.active(), 1);
console.log(JSON.stringify({status: 'unsupported', reason: 'no public future cancellation', methods, activeGuestWriters: 1}));
process.exit(0);
