import { current, increment, greeting } from './state';

type Features = { gift?: boolean; priority?: boolean };
type Item = { name: string; amount: number; tags: string[] };
type Choice = { tag: 'none' } | { tag: 'text'; val: string } | { tag: 'numbers'; val: number[] };
type Request = {
  items: Item[];
  label?: string;
  policy: Features;
  metadata: [number, string];
  detail: Choice;
};
type Response = {
  summary: string;
  counts: number[];
  first?: Item;
  policy: Features;
  metadata: [number, string];
  detail: Choice;
};

export const catalog = {
  process(value: Request): Response {
    if (value.items.length === 0) throw { tag: 'empty' };
    const invalid = value.items.find(item => item.amount === 0);
    if (invalid) throw { tag: 'invalid', val: invalid.name };
    return {
      summary: `${value.label ?? ''}:${value.items[0].name}`,
      counts: value.items.map(item => item.amount),
      first: value.items[0],
      policy: value.policy,
      metadata: value.metadata,
      detail: value.detail,
    };
  },
  increment,
};

export const formatting = {
  echo(value: Response): Response { return value; },
  greet(value: string): string { return greeting + value; },
  count(values: string[]): number { return values.length; },
  current,
};
