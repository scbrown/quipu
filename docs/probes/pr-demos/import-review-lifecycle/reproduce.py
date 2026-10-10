import pathlib,tempfile,subprocess,hashlib,os,json,sys
before,candidate=sys.argv[1:3]
with tempfile.TemporaryDirectory(prefix='review-demo-') as d:
 root=pathlib.Path(d);env={**os.environ,'HOME':d}
 def run(binary,args):return subprocess.run([binary,*args],cwd=d,env=env,text=True,capture_output=True)
 data=b'<https://example.org/item> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://foreign.example/Unknown> .\n'
 (root/'source.nt').write_bytes(data)
 r=run(candidate,['ingest','source.nt','--graph','https://example.org/window','--timestamp','2026-10-10T00:00:00Z','--declare-count','1','--declare-sha256',hashlib.sha256(data).hexdigest(),'--db','source.db']);assert r.returncode==0,r.stderr
 r=run(candidate,['share','--graph','https://example.org/window','--output','share','--no-shapes','--destination','internal','--db','source.db']);assert r.returncode==0,r.stderr
 sid=json.loads((root/'share/manifest.json').read_text())['share_id']
 for title,binary,db in [('Before',before,'before.db'),('After',candidate,'after.db')]:
  print(title,': actual native CLI, same private unknown-type share')
  r=run(binary,['--version']);print(r.stdout.strip())
  r=run(binary,['import','share','--destination','internal','--db',db]);print('import exit=',r.returncode);assert r.returncode==0,r.stderr
  r=run(binary,['import','review','pending','--limit','1','--db',db]);print('pending exit=',r.returncode);print(r.stdout.strip());print(r.stderr.strip())
  if title=='Before': assert r.returncode!=0;continue
  assert r.returncode==0;assert len(json.loads(r.stdout)['reviews'])==1
  for action in ['rejected','reopen']:
   r=run(binary,['import','review',action,sid,'--actor','reviewer','--reason','private fixture decision','--db',db]);print(action,'exit=',r.returncode);print(r.stdout.strip());assert r.returncode==0,r.stderr
   r=run(binary,['import','share','--destination','internal','--db',db]);print('reimport after',action,'exit=',r.returncode)
   print(r.stderr.strip())
   assert (r.returncode!=0)==(action=='rejected')
 print('No production import, timer, outbound delivery or activation in this demonstration.')
