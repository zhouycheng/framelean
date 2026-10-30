#!/usr/bin/env python3
"""Copy the complete non-system dylib closure and rewrite it to @loader_path."""
import pathlib, shutil, subprocess, sys
root=pathlib.Path(sys.argv[1]).resolve()
queue=list(root.glob('*.dylib')); sources={p.name:p for p in queue}; done=set()
while queue:
 target=queue.pop(0)
 if target.name in done: continue
 done.add(target.name)
 source=sources[target.name]
 output=subprocess.check_output(['otool','-L',str(target)],text=True)
 for line in output.splitlines()[2:]:
  dep=line.strip().split(' (',1)[0]
  if dep.startswith(('/usr/lib/','/System/')):continue
  if dep.startswith('@loader_path/'):
   original=source.parent/dep[len('@loader_path/'):]
  elif dep.startswith('@rpath/'):
   original=source.parent/dep[len('@rpath/'):]
  else: original=pathlib.Path(dep)
  if not original.is_file():raise RuntimeError(f'Unresolved dependency: {dep}')
  dest=root/original.name
  if dest.name not in sources:
   shutil.copy2(original,dest);dest.chmod(0o755);sources[dest.name]=original;queue.append(dest)
  subprocess.run(['install_name_tool','-change',dep,'@loader_path/'+dest.name,str(target)],check=True)
 subprocess.run(['install_name_tool','-id','@loader_path/'+target.name,str(target)],check=True)
for p in root.glob('*.dylib'):
 subprocess.run(['codesign','--force','--sign','-',str(p)],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
 for line in subprocess.check_output(['otool','-L',str(p)],text=True).splitlines()[1:]:
  dep=line.strip().split(' (',1)[0]
  if not dep.startswith(('@loader_path/','/usr/lib/','/System/')):raise RuntimeError('External dependency: '+dep)
print(f'Bundled {len(sources)} dylibs; no external development paths')
