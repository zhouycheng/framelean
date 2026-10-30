#!/usr/bin/env python3
"""Prepare public, attributed P0 fixtures; FFmpeg CLI is test-only."""
import argparse,hashlib,pathlib,urllib.request,zipfile,subprocess
p=argparse.ArgumentParser();p.add_argument('--output',default='.workspace/p0-media');a=p.parse_args();root=pathlib.Path(a.output);root.mkdir(parents=True,exist_ok=True)
url='https://download.blender.org/peach/bigbuckbunny_movies/BigBuckBunny_320x180.mp4.zip'
archive=root/'bbb.zip'
if not archive.exists():urllib.request.urlretrieve(url,archive)
assert hashlib.sha256(archive.read_bytes()).hexdigest()=='109e3ede8790bd633f374ca311d9cc61dce8d7f98f5b0797ca98199c9fbceedf','Unexpected source archive'
name='BigBuckBunny_320x180.mp4'
with zipfile.ZipFile(archive) as z:
 with z.open(name) as src,(root/name).open('wb') as dst:
  import shutil
  shutil.copyfileobj(src,dst)
subprocess.run(['ffmpeg','-v','error','-ss','30','-i',str(root/name),'-t','30','-map','0:v:0','-map','0:a:0','-c','copy','-y',str(root/'bbb-30s.mp4')],check=True)
print('Prepared bbb-30s.mp4 — © 2008 Blender Foundation / www.bigbuckbunny.org, CC BY 3.0')
