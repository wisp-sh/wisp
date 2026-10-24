// The compiler receives a frozen virtual filesystem, with no host or network APIs.
import base from "./base";

export let files: Record<string, string> = Object.create(null);
export function reset(inputs: Record<string, string>): void {
  files = Object.assign(Object.create(null), inputs, { "/bend2/base.bend": base });
}

export function normalize(path: string): string {
  const parts: string[] = [];
  for (const part of path.split("/")) {
    if (part === "..") parts.pop();
    else if (part !== "" && part !== ".") parts.push(part);
  }
  return "/" + parts.join("/");
}

export const fs = {
  realpathSync: normalize,
  readFileSync(path: string): string {
    const key = normalize(path);
    if (!Object.hasOwn(files, key)) throw new Error("missing compiler input: " + key);
    return files[key];
  },
};
