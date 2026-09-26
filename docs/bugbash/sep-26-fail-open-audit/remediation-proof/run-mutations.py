from pathlib import Path
import json, subprocess, os, re, time, hashlib
root=Path(Path('/tmp/qmp-sep26-current-candidate.txt').read_text().strip())
proof=root/'proof';cases=json.loads((proof/'mutation-spec.json').read_text());rows=[]
env=os.environ.copy();env.update({'CARGO_TARGET_DIR':'/tmp/qmp-sep26-candidate-y2s09ws1/target','CARGO_PROFILE_TEST_DEBUG':'0','CARGO_BUILD_JOBS':'4'})
(proof/'build-env.json').write_text(json.dumps({k:env[k] for k in ['CARGO_TARGET_DIR','CARGO_PROFILE_TEST_DEBUG','CARGO_BUILD_JOBS']},indent=2)+'\n')
def run(case,phase):
 cmd=['cargo','test','--locked','-p',case['package'],*case['cargo_args'],'--',case['selector'],'--exact','--nocapture']
 log=proof/(case['id']+'-'+phase+'.log');started=time.time()
 with log.open('w') as stream:
  result=subprocess.run(cmd,cwd=root,env=env,stdout=stream,stderr=subprocess.STDOUT)
 text=log.read_text();executed=bool(re.search(r'(?:^|\n)running 1 test\b',text))
 failed=bool(re.search(r'test result: FAILED\. 0 passed; 1 failed;',text))
 passed=bool(re.search(r'test result: ok\. 1 passed; 0 failed;',text))
 verdict='GREEN' if result.returncode==0 and executed and passed else ('RED' if executed and failed and result.returncode!=0 else 'INVALID')
 row={'id':case['id'],'phase':phase,'verdict':verdict,'exit_code':result.returncode,'executed_one':executed,'command':cmd,'log':str(log),'seconds':time.time()-started}
 rows.append(row);(proof/'mutation-results.json').write_text(json.dumps(rows,indent=2)+'\n');print(phase,case['id'],verdict,flush=True)
 return verdict
# Every exact selector must execute once and pass before any mutation.
for case in cases:
 if run(case,'baseline')!='GREEN':raise SystemExit('baseline invalid: '+case['id'])
for case in cases:
 path=root/case['path'];original=path.read_bytes();text=original.decode()
 try:
  for a,b in case['edits']:
   assert text.count(a)==1,(case['id'],text.count(a));text=text.replace(a,b,1)
  path.write_text(text)
  if run(case,'mutant')!='RED':raise SystemExit('mutation survived or invalid: '+case['id'])
 finally:path.write_bytes(original)
# Re-run all repaired selectors after restoration, then attest byte custody.
for case in cases:
 if run(case,'restored')!='GREEN':raise SystemExit('restore invalid: '+case['id'])
expected=json.loads((proof/'source-sha256.json').read_text())
assert all(hashlib.sha256((root/p).read_bytes()).hexdigest()==digest for p,digest in expected.items())
(proof/'restored.txt').write_text('All source hashes match the frozen candidate. Every selector baseline/restored GREEN and mutant RED.\n')
print('COMPLETE',len(cases),'mutations rejected; sources restored',root,flush=True)
