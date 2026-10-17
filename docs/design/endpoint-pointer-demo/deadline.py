"""Private loopback deadline falsifier; stdlib only, copies all executables."""
import argparse,http.server,json,os,pathlib,shutil,subprocess,tempfile,threading,time
p=argparse.ArgumentParser(description=__doc__)
p.add_argument("binary_directory",type=pathlib.Path)
a=p.parse_args()
ART=a.binary_directory.resolve()
requests=[]
class Handler(http.server.BaseHTTPRequestHandler):
 def do_POST(self):
  requests.append({'path':self.path,'at':time.monotonic()})
  self.rfile.read(int(self.headers.get('Content-Length','0')))
  data=b'{"head":{"vars":["s"]},"results":{"bindings":[{"s":{"type":"uri","value":"urn:served"}}]}}'
  self.send_response(200);self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
 def log_message(self,*a): pass
server=http.server.HTTPServer(('127.0.0.1',0),Handler)
th=threading.Thread(target=server.serve_forever,daemon=True);th.start()
results=[]
try:
 with tempfile.TemporaryDirectory() as td:
  root=pathlib.Path(td); bins=root/'bin';bins.mkdir()
  for name in ['quipu-service-candidate','endpoint_pointer_fixture']:shutil.copy2(ART/name,bins/name)
  env=dict(os.environ,HOME=str(root),XDG_CONFIG_HOME=str(root/'.config'),PATH=str(bins))
  endpoint=f'http://127.0.0.1:{server.server_port}/sparql/'
  pendant=root/'declaration.pendant'
  subprocess.run([str(bins/'endpoint_pointer_fixture'),str(pendant),endpoint],env=env,cwd=root,check=True,capture_output=True)
  cfg=root/'.bobbin/config.toml';cfg.parent.mkdir()
  cfg.write_text('[quipu.search]\nquery_timeout_ms=20\n[[quipu.federation.pendants]]\npath='+json.dumps(str(pendant))+'\nrepo_remote="https://github.com/foreign/project.git"\n')
  query=f'SELECT ?s WHERE {{ SERVICE <{endpoint}> {{ ?s ?p ?o }} }}'
  for label,delay in [('immediate',0),('delayed',0.25)]:
   trust=bins/'caboodle';trust.write_text('#!/bin/sh\n'+('/bin/sleep '+str(delay)+'\n' if delay else '')+'exit 0\n');trust.chmod(0o755)
   before=len(requests);start=time.monotonic()
   p=subprocess.run([str(bins/'quipu-service-candidate'),'query',query,'--federation','--db',str(root/(label+'.db'))],env=env,cwd=root,capture_output=True,text=True,timeout=8)
   results.append({'label':label,'trust_delay_ms':int(delay*1000),'query_budget_ms':20,'elapsed_ms':round((time.monotonic()-start)*1000,3),'exit':p.returncode,'requests':len(requests)-before,'output':(p.stdout+p.stderr)[-600:]})
 print(json.dumps({'results':results,'fixture':'copied executables; temporary stores and loopback endpoint; stub trust CLI; no installed-service proof'},indent=2))
finally:
 server.shutdown();server.server_close();th.join()
