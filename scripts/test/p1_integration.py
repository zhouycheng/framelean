#!/usr/bin/env python3
"""Real v2 queue, receipts, retries and restart integration; isolated data only."""
import pathlib,tempfile,subprocess,os,time,json,urllib.request,urllib.error,uuid,sqlite3,hashlib,argparse
p=argparse.ArgumentParser();p.add_argument('--engine',required=True);p.add_argument('--library',required=True);p.add_argument('--sample',required=True);a=p.parse_args()
engine=str(pathlib.Path(a.engine).resolve());library=str(pathlib.Path(a.library).resolve());sample=str(pathlib.Path(a.sample).resolve());checks=[]
with tempfile.TemporaryDirectory(prefix='framelean-p1-') as tmp:
 root=pathlib.Path(tmp);runtime=root/'runtime';data=root/'data';out=root/'中文 输出';out.mkdir();bad=root/'broken.mp4';bad.write_bytes(b'broken');proc=None
 def start():
  global proc,ep
  proc=subprocess.Popen([engine,'worker',library],env={**os.environ,'FRAMELEAN_RUNTIME_DIR':str(runtime),'FRAMELEAN_DATA_DIR':str(data)},stderr=subprocess.PIPE)
  for _ in range(100):
   if proc.poll() is not None:raise AssertionError(proc.stderr.read().decode())
   try:
    ep=json.loads((runtime/'endpoint.json').read_text());call('health');return
   except (OSError,ValueError):time.sleep(.1)
  raise AssertionError('startup timeout')
 def call(name,body=None):
  req=urllib.request.Request(ep['url']+'/v2/'+name,data=None if body is None else json.dumps({'request_id':str(uuid.uuid4()),**body}).encode(),headers={'Authorization':'Bearer '+ep['token'],'Content-Type':'application/json'})
  with urllib.request.urlopen(req,timeout=10) as r:return json.load(r)
 def wait(pred):
  for _ in range(600):
   s=call('snapshot')
   if pred(s):return s
   time.sleep(.1)
  raise AssertionError('timeout '+json.dumps(s))
 def stop():
  call('shutdown',{});proc.wait(timeout=8);assert not (runtime/'endpoint.json').exists()
 try:
  start();assert call('snapshot')['paused'];req={'request_id':'same','inputs':[sample,str(bad)]};r=call('analyze',req);assert call('analyze',req)==r
  try:call('analyze',{'request_id':'same','inputs':[sample]});raise AssertionError('conflicting receipt accepted')
  except urllib.error.HTTPError as e:assert e.code==400
  checks.append('durable idempotency and request conflict')
  call('resume',{});s=wait(lambda s:len(s['analyses'])==2 and all(x['status'] in ['succeeded','failed'] for x in s['analyses']));assert [x['status'] for x in s['analyses']]==['succeeded','failed'];aid=r['ids'][0];checks.append('mixed analysis batch continues after failure')
  call('pause',{});r=call('execute',{'analysis_ids':[aid,aid],'output_dir':str(out)});call('cancel',{'attempt_id':r['ids'][0]});task=call('snapshot')['attempts'][0]['task_id'];call('retry',{'task_id':task});stop();start();s=call('snapshot');assert s['paused'];assert len(s['attempts'])==3;assert call('analyze',req)['ids'][0]==aid;checks.append('queued cancel retry history and restart pause')
  call('resume',{});s=wait(lambda s:sum(x['status']=='succeeded' for x in s['attempts'])==2);assert len(list(out.glob('*.mp4')))==2
  for o in s['outputs']:subprocess.run(['ffmpeg','-v','error','-i',o['final_path'],'-f','null','-'],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
  checks.append('real compression Unicode output collision and decode')
  call('pause',{});r=call('execute',{'analysis_ids':[aid],'output_dir':str(out)});call('resume',{});s=wait(lambda s:any(e['id']==r['ids'][0] and e['status']=='running' for e in s['attempts']));proc.kill();proc.wait();start();s=call('snapshot');assert next(e for e in s['attempts'] if e['id']==r['ids'][0])['status']=='interrupted';checks.append('running crash becomes interrupted')
  stop()
  # Simulate the precise filesystem/DB commit gap without timing-dependent sleeps.
  db=sqlite3.connect(data/'framelean-v2.sqlite3');attempt=r['ids'][0];target=out/'recover.mp4';target.write_bytes(b'verified output');digest=hashlib.sha256(target.read_bytes()).hexdigest()
  db.execute("UPDATE execution_attempt SET status='publishing' WHERE id=?",(attempt,));db.execute("INSERT INTO output_artifact VALUES(?,?,?,'publishing',?,?)",(str(uuid.uuid4()),attempt,str(target),target.stat().st_size,digest));db.commit();db.close();start();s=call('snapshot');assert next(e for e in s['attempts'] if e['id']==attempt)['status']=='succeeded';checks.append('publication commit gap hash reconciliation');stop()
  for case in ['missing','conflict']:
   db=sqlite3.connect(data/'framelean-v2.sqlite3')
   db.execute("UPDATE execution_attempt SET status='publishing' WHERE id=?",(attempt,))
   db.execute("UPDATE output_artifact SET status='publishing' WHERE attempt_id=?",(attempt,))
   db.commit();db.close()
   if case=='missing':target.unlink()
   else:target.write_bytes(b'unrelated existing file')
   start();s=call('snapshot');assert next(e for e in s['attempts'] if e['id']==attempt)['status']=='interrupted'
   if case=='conflict':assert target.read_bytes()==b'unrelated existing file'
   stop()
  checks.append('missing or conflicting publication never overwrites files')
  start();call('resume',{});r=call('execute',{'analysis_ids':[aid],'output_dir':str(out)});s=wait(lambda s:s['active'] is not None and s['active']['id']==r['ids'][0]);os.kill(s['active']['host_pid'],getattr(__import__('signal'),'SIGKILL',__import__('signal').SIGTERM));wait(lambda s:next(e for e in s['attempts'] if e['id']==r['ids'][0])['status']=='failed')
  r=call('execute',{'analysis_ids':[aid],'output_dir':str(out)});wait(lambda s:s['active'] is not None and s['active']['id']==r['ids'][0]);call('cancel',{'attempt_id':r['ids'][0]});wait(lambda s:next(e for e in s['attempts'] if e['id']==r['ids'][0])['status']=='canceled');checks.append('Host crash isolated and subsequent running cancellation succeeds');stop()
  start();copy=root/'changed.mp4';copy.write_bytes(pathlib.Path(sample).read_bytes());ar=call('analyze',{'inputs':[str(copy)]});call('resume',{});wait(lambda s:next(x for x in s['analyses'] if x['id']==ar['ids'][0])['status']=='succeeded');call('pause',{});r=call('execute',{'analysis_ids':ar['ids'],'output_dir':str(out)});os.utime(copy,None);call('resume',{});wait(lambda s:next(e for e in s['attempts'] if e['id']==r['ids'][0])['status']=='failed');checks.append('changed source refused before execution')
  call('pause',{});gone=root/'removed-output';gone.mkdir();r=call('execute',{'analysis_ids':[aid],'output_dir':str(gone)});gone.rmdir();call('resume',{});wait(lambda s:next(e for e in s['attempts'] if e['id']==r['ids'][0])['status']=='failed');checks.append('output creation failure recorded without publication');stop()
  unsupported=root/'mpeg4.mp4';subprocess.run(['ffmpeg','-v','error','-i',sample,'-t','1','-c:v','mpeg4','-an',str(unsupported)],check=True)
  start();r=call('analyze',{'inputs':[str(unsupported),sample]});call('resume',{});s=wait(lambda s:all(next(x for x in s['analyses'] if x['id']==i)['status'] in ['failed','succeeded'] for i in r['ids']));assert [next(x for x in s['analyses'] if x['id']==i)['status'] for i in r['ids']]==['failed','succeeded'];checks.append('unsupported codec does not block next supported input');stop()
  print(json.dumps({'checks':checks,'platform':os.name},ensure_ascii=False,indent=2))
 finally:
  if proc is not None and proc.poll() is None:proc.kill();proc.wait()
