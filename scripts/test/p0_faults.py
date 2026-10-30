#!/usr/bin/env python3
"""macOS fault checks for ABI diagnostics, dependency closure and Worker death."""
import argparse,json,os,pathlib,shutil,signal,subprocess,tempfile,time,urllib.request,uuid
p=argparse.ArgumentParser();p.add_argument('--runtime',required=True);p.add_argument('--sample',required=True);a=p.parse_args();runtime=pathlib.Path(a.runtime).resolve();sample=pathlib.Path(a.sample).resolve();engine=runtime/'framelean-engine';lib=runtime/'libframelean_fll.dylib'
with tempfile.TemporaryDirectory(prefix='framelean-faults-') as tmp:
 root=pathlib.Path(tmp)
 missing=subprocess.run([str(engine),'probe',str(root/'missing.dylib')],capture_output=True,text=True);assert missing.returncode!=0 and 'FLL 加载失败' in missing.stderr;print('PASS missing FLL diagnostic')
 c=root/'abi.c';c.write_text('unsigned int fll_abi_version(void){return 999;}')
 subprocess.run(['clang','-dynamiclib',str(c),'-o',str(root/'abi.dylib')],check=True)
 incompatible=subprocess.run([str(engine),'probe',str(root/'abi.dylib')],capture_output=True,text=True);assert incompatible.returncode!=0 and 'ABI' in incompatible.stderr;print('PASS ABI mismatch diagnostic')
 broken=root/'broken';broken.mkdir();shutil.copy2(lib,broken/lib.name)
 absent=subprocess.run([str(engine),'probe',str(broken/lib.name)],capture_output=True,text=True);assert absent.returncode!=0;print('PASS missing bundled dependency diagnostic')
 env={**os.environ,'FRAMELEAN_RUNTIME_DIR':str(root/'runtime')};proc=subprocess.Popen([str(engine),'worker',str(lib)],env=env)
 try:
  for _ in range(100):
   ep=root/'runtime/endpoint.json'
   if ep.exists():break
   time.sleep(.1)
  data=json.loads(ep.read_text())
  def call(path,body=None):
   if body is not None:body={'request_id':str(uuid.uuid4()),**body}
   req=urllib.request.Request(data['url']+'/v1/'+path,data=None if body is None else json.dumps(body).encode(),headers={'Authorization':'Bearer '+data['token'],'Content-Type':'application/json'})
   with urllib.request.urlopen(req,timeout=5) as r:return json.load(r)
  call('analyze',{'input':str(sample)})
  for _ in range(100):
   t=call('snapshot')['task']
   if t['state']=='succeeded':break
   assert t['state']!='failed',t
   time.sleep(.1)
  task=call('execute',{'analysis_id':t['analysis_id'],'output_dir':str(root)})
  pid=task['host_pid'];proc.kill();proc.wait()
  stopped=False
  for _ in range(100):
   result=subprocess.run(['ps','-p',str(pid),'-o','stat='],capture_output=True,text=True)
   if result.returncode!=0 or result.stdout.strip().startswith('Z'):stopped=True;break
   time.sleep(.05)
  assert stopped,'Host outlived Worker';assert not list(root.glob('*-compressed*.mp4'));print('PASS Worker death stops Host without publishing output')
 finally:
  if proc.poll() is None:proc.terminate();proc.wait(timeout=5)
