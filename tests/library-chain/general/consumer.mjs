import assert from 'node:assert/strict';
import { catalog, formatting } from '@wisp/catalog';

function request(items) {
  return { items, label: 'order', policy: {gift: true, priority: true},
    metadata: [7, 'meta'], detail: {tag: 'numbers', val: [1, 2]} };
}
const output = catalog.process(request([
  {name: 'book', amount: 2, tags: ['a', 'b']}, {name: 'pen', amount: 5, tags: []},
]));
const expected = {summary: 'order:book', counts: new Uint32Array([2, 5]), first: {name: 'book', amount: 2, tags: ['a', 'b']},
  policy: {gift: true, priority: true}, metadata: [7, 'meta'], detail: {tag: 'numbers', val: new Uint32Array([1, 2])}};
assert.deepEqual(output, expected);
assert.deepEqual(formatting.echo(output), expected);
assert.throws(() => catalog.process(request([])), e => e.payload.tag === 'empty');
assert.throws(() => catalog.process(request([{name: 'bad', amount: 0, tags: []}])),
  e => e.payload.tag === 'invalid' && e.payload.val === 'bad');
assert.equal(formatting.greet('world'), 'hello world');
assert.equal(formatting.count(['a', 'b']), 2);
assert.equal(formatting.current(), 0);
assert.equal(catalog.increment(), 1);
assert.equal(formatting.current(), 1);
assert.equal(catalog.increment(), 2);
assert.equal(formatting.current(), 2);
console.log('nested-success\ntyped-empty\ntyped-invalid(bad)\nhello world\ncount(2)\nshared-state(2)');
