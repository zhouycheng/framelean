#!/usr/bin/env python3
"""Real Worker/Host/FLL integration. CLI encoders are only fixture preparation/oracles."""
import argparse, json, os, pathlib, signal, subprocess, tempfile, time, urllib.request, urllib.error, uuid, shutil, hashlib
parser=argparse.ArgumentParser();parser.add_argument('--engine',required=True);parser.add_argument('--library',required=True);parser.add_argument('--sample',required=True);parser.add_argument('--report');args=parser.parse_args()
engine=str(pathlib.Path(args.engine).resolve());library=str(pathlib.Path(args.library).resolve());sample=pathlib.Path(args.sample).resolve();checks=[]
def record(name):checks.append(name);print('PASS',name,flush=True)
with tempfile.TemporaryDirectory(prefix='framelean-p0-') as root:
 root=pathlib.Path(root);runtime=root/'runtime';env={**os.environ,'FRAMELEAN_RUNTIME_DIR':str(runtime)}
 proc=subprocess.Popen([engine,'worker',library],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
 ep=None
 try:
  for _ in range(150):
   if (runtime/'endpoint.json').exists():ep=json.loads((runtime/'endpoint.json').read_text());break
   if proc.poll() is not None:raise RuntimeError(proc.stderr.read().decode())
   time.sleep(.1)
  assert ep
  def call(name,body=None,token=True):
   headers={'Content-Type':'application/json'}
   if token:headers['Authorization']='Bearer '+ep['token']
   if body is not None:body={'request_id':str(uuid.uuid4()),**body}
   r=urllib.request.Request(ep['url']+'/v1/'+name,data=None if body is None else json.dumps(body).encode(),headers=headers)
   with urllib.request.urlopen(r,timeout=10) as res:return json.load(res)
  def wait(task,expected='succeeded'):
   for _ in range(1200):
    t=call('snapshot')['task']
    if t and t['task_id']==task['task_id'] and t['state'] in ('succeeded','failed','canceled'):
     assert t['state']==expected,t;return t
    time.sleep(.05)
   raise AssertionError('operation timeout')
  def analyze(path):return wait(call('analyze',{'input':str(path)}))
  def execute(a):return call('execute',{'analysis_id':a['analysis_id'],'output_dir':str(root)})
  health=call('health');assert health['components']['available'],health;record('packaged FLL probe and ABI')
  try:call('health',token=False);raise AssertionError('unauthenticated request accepted')
  except urllib.error.HTTPError as e:assert e.code==401
  record('unauthenticated request rejected')
  stream_request=urllib.request.Request(ep['url']+'/v1/events',headers={'Authorization':'Bearer '+ep['token']})
  with urllib.request.urlopen(stream_request,timeout=3) as stream:
   assert stream.readline().startswith(b'id: ')
   assert stream.readline().startswith(b'data: ')
  record('SSE delivers first snapshot without buffering')
  source=root/'中文 视频.mp4';shutil.copyfile(sample,source);digest=hashlib.sha256(source.read_bytes()).hexdigest()
  a=analyze(source);assert a['result']['video']['width']>0;record('analyze real media with Unicode path')
  t=wait(execute(a));output=pathlib.Path(t['output']);assert output.is_file()
  subprocess.run(['ffmpeg','-v','error','-xerror','-i',str(output),'-f','null','-'],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
  assert hashlib.sha256(source.read_bytes()).hexdigest()==digest;record('transcode decodes and source preserved')
  def packets(file):
   return json.loads(subprocess.check_output(['ffprobe','-v','error','-show_packets','-show_entries','packet=stream_index,pts_time,data_hash','-show_data_hash','sha256','-of','json',str(file)],text=True))['packets']
  source_packets=packets(source);output_packets=packets(output)
  def pts(items,index):return sorted(float(p['pts_time']) for p in items if p['stream_index']==index)
  vi=a['result']['video']['index'];vp=pts(source_packets,vi);vq=pts(output_packets,0)
  assert len(vp)==len(vq),(len(vp),len(vq));offset=vq[0]-vp[0]
  assert max(abs((y-x)-offset) for x,y in zip(vp,vq))<0.002
  if a['result']['audio']:
   ai=a['result']['audio']['index'];ap=[p for p in source_packets if p['stream_index']==ai];aq=[p for p in output_packets if p['stream_index']==1]
   assert [p['data_hash'] for p in ap]==[p['data_hash'] for p in aq]
   assert max(abs(float(y['pts_time'])-float(x['pts_time'])-offset) for x,y in zip(ap,aq))<0.002
  record('frame timestamps and AAC packet hashes preserve A/V timing')
  before=hashlib.sha256(output.read_bytes()).hexdigest();t2=wait(execute(a));assert t2['output']!=str(output);assert hashlib.sha256(output.read_bytes()).hexdigest()==before;record('output collision never overwrites')
  noaudio=root/'noaudio.mp4';subprocess.run(['ffmpeg','-v','error','-i',str(source),'-map','0:v:0','-c','copy',str(noaudio)],check=True)
  na=analyze(noaudio);assert na['result']['audio'] is None;wait(execute(na));record('video without audio')
  broken=root/'broken.mp4';broken.write_bytes(b'not a movie');wait(call('analyze',{'input':str(broken)}),'failed');record('corrupt input rejected')
  a=analyze(source);task=execute(a);call('cancel',{'task_id':task['task_id']});wait(task,'canceled');assert not list(root.glob('*.partial.mp4'));record('cooperative cancel and partial cleanup')
  if hasattr(signal,'SIGSTOP'):
   a=analyze(source);task=execute(a);os.kill(task['host_pid'],signal.SIGSTOP)
   begin=time.monotonic();call('cancel',{'task_id':task['task_id']});wait(task,'canceled')
   assert time.monotonic()-begin<5;assert not list(root.glob('*.partial.mp4'));record('unresponsive Host force-canceled within 5 seconds')
  a=analyze(source);task=execute(a);os.kill(task['host_pid'],getattr(signal,'SIGKILL',signal.SIGTERM));wait(task,'failed');assert not list(root.glob('*.partial.mp4'));analyze(source);record('Host crash isolated and next analysis succeeds')
  a=analyze(source);source.touch()
  try:execute(a);raise AssertionError('changed source accepted')
  except urllib.error.HTTPError as e:assert e.code==400
  record('changed source rejected')
  a=analyze(source);task=execute(a)
  # A new client discovers the same Worker and current task without submitting again.
  again=json.loads((runtime/'endpoint.json').read_text());assert again['pid']==ep['pid'];assert call('snapshot')['task']['task_id']==task['task_id'];wait(task);record('client reconnect preserves task')
  second=subprocess.run([engine,'worker',library],env=env,capture_output=True,timeout=10);assert second.returncode==0;assert call('health')['pid']==ep['pid'];record('single Worker instance')
  unsupported=root/'unsupported.mp4'
  subprocess.run(['ffmpeg','-v','error','-i',str(source),'-t','1','-c:v','mpeg4','-c:a','copy',str(unsupported)],check=True)
  wait(call('analyze',{'input':str(unsupported)}),'failed');record('unsupported codec rejected')
  a=analyze(source);task=execute(a);call('shutdown',{});proc.wait(timeout=10);assert not (runtime/'endpoint.json').exists();assert not list(root.glob('*.partial.mp4'));record('explicit shutdown cancels active task')
 finally:
  if proc.poll() is None:proc.terminate();proc.wait(timeout=10)
 report={'checks':checks,'sample_sha256':hashlib.sha256(sample.read_bytes()).hexdigest(),'sample_name':sample.name,'platform':os.uname().sysname if hasattr(os,'uname') else 'Windows'}
 if args.report:pathlib.Path(args.report).write_text(json.dumps(report,ensure_ascii=False,indent=2))
