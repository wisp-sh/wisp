import * as Bend from "./bend";
import * as Comp from "./comp";
import { files, fs, normalize, reset } from "./host";

// Load local imports synchronously. The upstream loader also fetches hub packages;
// builds here use only the captured source tree and the pinned Base library.
function load(book: Bend.Book, file: string, ns: string,
              seen: Map<string, boolean>, top: string): void {
  file = normalize(file);
  if (seen.has(file)) {
    if (!seen.get(file)) throw new Error("Bend import cycle: " + file);
    return;
  }
  seen.set(file, false);
  const dir = file.slice(0, file.lastIndexOf("/") + 1);
  const lines = fs.readFileSync(file).split("\n");
  const aliases: Record<string, string> = Object.create(null);
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (line === "" || line.startsWith("#")) continue;
    if (!/^import(?:\s|$)/.test(line)) break;
    const match = /^import\s+(\S+)(?:\s+as\s+([A-Za-z_]\w*))?\s*(?:#.*)?$/.exec(line);
    if (!match) throw new Error("invalid Bend import: " + line);
    lines[i] = "";
    if (match[1] === "Base" && !match[2]) {
      load(book, "/bend2/base.bend", "", seen, "/bend2/");
      continue;
    }
    if (!match[2] || !/^(?:\.\/|(?:\.\.\/)*)?(?:[A-Za-z_][\w-]*\/)*[A-Za-z_][\w-]*\.bend$/.test(match[1])) {
      throw new Error("use local .bend imports with an alias; hub imports are unsupported: " + line);
    }
    if (Object.hasOwn(aliases, match[2])) throw new Error("duplicate import alias: " + match[2]);
    const path = normalize(dir + match[1]);
    if (!path.startsWith(top)) throw new Error("Bend import escapes project: " + match[1]);
    const sub = path.slice(top.length, -5);
    aliases[match[2]] = sub;
    load(book, path, sub, seen, top);
  }
  const start = book.order.length;
  Bend.parse_book(book, dir, lines.join("\n"), ns, aliases);
  if (file === "/bend2/base.bend") {
    for (const name of book.order.slice(start)) book.tlds[name].b = true;
  }
  seen.set(file, true);
}

export function compile(request: string): string {
  try {
    const input = JSON.parse(request);
    reset(input.files);
    const entry = normalize("/project/" + input.entry);
    if (!Object.hasOwn(files, entry)) throw new Error("missing Bend entrypoint");
    const book = Bend.book_nil();
    load(book, entry, "", new Map(), "/project/");
    Bend.book_valid(book);
    if (book.hols !== 0) throw new Error("Bend contains unproved laws or holes");
    const exports: string[] = [];
    for (const name of book.order) {
      const def = book.tlds[name];
      if (def.$ !== "Def" || def.b || def.v === null || def.x !== 0) continue;
      const { doms, ret } = Bend.tele_unbind(book, def.T);
      const scalar = (type: Bend.HTerm): boolean => {
        const t = Bend.term_wnf(book, type);
        return t.$ === "ADT" && (t.k === "U32" || t.k === "F32");
      };
      if (/^[a-z][a-z0-9]*$/.test(name) && doms.length <= 16 &&
          doms.every(([q, , type]) => q.$ !== "None" && scalar(type)) && scalar(ret)) {
        exports.push(name);
      }
    }
    if (exports.length === 0) throw new Error("Bend needs a U32 or F32 function to export");
    const library = Comp.js_lib(book, true, exports).replace("export default {", "const bend = {");
    const wrappers = exports.map(name =>
      `export function ${name}(...args) { return bend[${JSON.stringify(name)}](...args); }`).join("\n");
    return JSON.stringify({ javascript: library + "\n" + wrappers + "\n", exports });
  } catch (error) {
    let message: string;
    try { message = Bend.err_show(error as Bend.Err); }
    catch { message = String(error); }
    return JSON.stringify({ error: message });
  }
}
