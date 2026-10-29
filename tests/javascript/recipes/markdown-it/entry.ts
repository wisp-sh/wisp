import MarkdownIt from 'markdown-it';

const markdown = new MarkdownIt({ html: false, linkify: true });

export function render(source: string): string {
  return markdown.render(source);
}

export function renderInline(source: string): string {
  return markdown.renderInline(source);
}
