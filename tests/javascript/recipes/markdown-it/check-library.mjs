import assert from 'node:assert/strict';
import { render, renderInline } from './library/node/index.js';

assert.equal(render('# Hello\n'), '<h1>Hello</h1>\n');
assert.equal(renderInline('Hello **世界** 🦀'), 'Hello <strong>世界</strong> 🦀');
assert.equal(renderInline('<script>'), '&lt;script&gt;');
for (let i = 0; i < 100; i++) {
  assert.equal(renderInline(`**世界-${i}**`), `<strong>世界-${i}</strong>`);
}
console.log('markdown-it Node package: Unicode, escaping, and 100 repeated calls passed');
