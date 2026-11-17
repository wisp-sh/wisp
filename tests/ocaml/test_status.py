"""Exercise the workflow's actual terminal status script with a local API stub."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class AcceptanceStatus(unittest.TestCase):
    def test_success_requires_complete_matching_evidence(self):
        workflow = (ROOT/'.github/workflows/ocaml-source-acceptance.yml').read_text()
        block = workflow.split('name: Report complete public acceptance on the exact tested commit', 1)[1]
        script = '\n'.join(line[10:] for line in block.split('        run: |\n', 1)[1].splitlines())+'\n'
        cases = ('complete', 'missing', 'incomplete', 'wrong-wisp', 'wrong-compiler', 'failed-job')
        for case in cases:
            with self.subTest(case=case), tempfile.TemporaryDirectory(prefix='ocaml-status-') as temporary:
                root=Path(temporary); evidence=root/'ocaml'; evidence.mkdir()
                setup={'wispCommit':'a'*40, 'buildtoolsCommit':'b'*40, 'installer':'wisp',
                       'cleanInstall':True, 'compilerManifestSha256':'c'*64}
                result={'passed':True, 'complete':True, 'compilerManifestSha256':'c'*64}
                if case=='incomplete': result['complete']=False
                if case=='wrong-wisp': setup['wispCommit']='d'*40
                if case=='wrong-compiler': setup['buildtoolsCommit']='e'*40
                (evidence/'setup.json').write_text(json.dumps(setup))
                if case!='missing': (evidence/'result.json').write_text(json.dumps(result))
                stub=root/'gh'
                stub.write_text('#!/bin/sh\nprintf "%s\\n" "$@" > "$STATUS_CAPTURE"\n')
                stub.chmod(0o755)
                capture=root/'status.txt'
                env=dict(os.environ, PATH=str(root)+os.pathsep+os.environ['PATH'],
                         RUNNER_TEMP=str(root), GITHUB_SHA='a'*40, BUILDTOOLS_REF='b'*40,
                         ACCEPTANCE_JOB_STATUS='failure' if case=='failed-job' else 'success',
                         GITHUB_REPOSITORY='wisp-sh/wisp', GITHUB_SERVER_URL='https://github.com',
                         GITHUB_RUN_ID='1', STATUS_CAPTURE=str(capture), GH_TOKEN='unused-test-token')
                process=subprocess.run(['bash','-e'],input=script,text=True,env=env,
                                       stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=10)
                expected='success' if case=='complete' else 'failure'
                self.assertIn('state='+expected, capture.read_text())
                self.assertIn('OCaml public installed acceptance', capture.read_text())
                self.assertIn('CPDF is separate', capture.read_text())
                self.assertEqual(process.returncode==0, case=='complete', process.stderr[-2000:])


if __name__=='__main__': unittest.main()
