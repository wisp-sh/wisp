"""Schema-3 browser parity for the unchanged jq Component and installed npm archive."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
JQ = ROOT / "fixtures/jq/component.wasm"
EXPECTED_JQ = "3f4f95f882d2f7e574e1a3dcd01f690a8f72e928fe8c7d151892daf6433b173e"


def run(*args, cwd=None):
    result = subprocess.run([str(a) for a in args], cwd=cwd, text=True,
                            capture_output=True, timeout=300)
    if result.returncode:
        raise AssertionError(f"{args}: {result.returncode}\n{result.stdout}\n{result.stderr}")
    return result.stdout


class InstalledLibraryJqBrowser(unittest.TestCase):
    def test_original_jq_installed_browser_has_pure_profile(self):
        self.assertEqual(hashlib.sha256(JQ.read_bytes()).hexdigest(), EXPECTED_JQ)
        cli = os.environ.get("WISP_CLI", "wisp")
        with tempfile.TemporaryDirectory(prefix="wisp-library-jq-") as temp:
            work = Path(temp)
            release = Path(os.environ["WISP_LIBRARY_JQ_RELEASE"]) if "WISP_LIBRARY_JQ_RELEASE" in os.environ else work / "release"
            if "WISP_LIBRARY_JQ_RELEASE" not in os.environ:
                run(cli, "library", "pack", ROOT / "fixtures/jq", "--out", release)
            result = json.loads(run(cli, "library", "validate", release))
            self.assertTrue(result["valid"])
            self.assertEqual(result["identity"], "wisp:jq")
            self.assertEqual((release / "component.wasm").read_bytes(), JQ.read_bytes())
            with tarfile.open(release / "rust.crate") as crate:
                embedded = next(member for member in crate if member.name.endswith("/component.wasm"))
                self.assertEqual(crate.extractfile(embedded).read(), JQ.read_bytes())
            browser = work / "browser"
            browser.mkdir()
            (browser / "package.json").write_text('{"private":true,"type":"module"}')
            run("npm", "install", "--offline", "--ignore-scripts", "--no-audit", "--no-fund",
                release / "node.tgz", cwd=browser)
            installed = browser / "node_modules/@wisp/jq"
            self.assertIn("export function evaluate(filter: string, json: string): string[];",
                          (installed / "index.d.ts").read_text())
            self.assertEqual(json.loads((installed / "package.json").read_text())["name"], "@wisp/jq")
            node_result = json.loads(run("node", "--input-type=module", "-e", '''
import {evaluate} from '@wisp/jq';
let denied=false;try {evaluate('now','null')} catch(e) {denied=/denied.*clock|clock.*denied/i.test(e.message)}
console.log(JSON.stringify({result:evaluate('.[]','[1,2]'),environment:evaluate('env','null'),denied}));
''', cwd=browser))
            self.assertEqual(node_result, {"result": ["1", "2"], "environment": ["{}"], "denied": True})
            # Invoke the installed package's public browser entry. A generated staging
            # directory is not evidence of what the archive actually shipped.
            (browser / "index.html").write_text('''<script type="module">
import {evaluate} from './node_modules/@wisp/jq/browser.js';
const eq=(a,b)=>{if(JSON.stringify(a)!==JSON.stringify(b))throw Error(JSON.stringify({a,b}));};
try {
  for(let i=0;i<20;i++)eq(evaluate('.[]','[1,"雪",true]'),['1','"雪"','true']);
  eq(evaluate('env','null'),['{}']);eq(evaluate('$ENV','null'),['{}']);
  eq(evaluate('empty','null'),[]);eq(evaluate('test("雪")','"雪"'),['true']);
  eq(evaluate('halt','null'),[]);
  for(const filter of ['error("bad")','1|halt_error(3)','include "wisp-sentinel"; sentinel','. [']) {
    let failed=false;try{evaluate(filter,'null')}catch{failed=true}
    if(!failed)throw Error('missing error '+filter);
    eq(evaluate('.','42'),['42']);
  }
  let denied=false;try{evaluate('now','null')}catch(e){denied=/denied.*clock|clock.*denied/i.test(e.message)}
  if(!denied)throw Error('clock was not denied');
  eq(evaluate('.','42'),['42']);document.body.textContent='PASS browser pure jq schema3';
} catch(e) {document.body.textContent='FAIL '+e.stack;}
</script>''')
            (browser / "wisp-sentinel.jq").write_text('def sentinel: "host file leaked";')
            run("node", ROOT / "tests/browser.mjs", browser)
            evidence = json.loads((browser / "browser-evidence.json").read_text())
            self.assertEqual(evidence["text"], "PASS browser pure jq schema3")
            self.assertTrue(evidence["externalControlDenied"])
            self.assertEqual([url for url in evidence["requests"] if not url.startswith("http://127.0.0.1:")],
                             ["https://wisp.example/negative-control"])


if __name__ == "__main__":
    unittest.main()
