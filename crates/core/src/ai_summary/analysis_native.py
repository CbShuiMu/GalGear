"""一次格式研究程序内复用固定原生原语进程；不接收模型指定的命令或方法。"""
import json
import os
from pathlib import Path
import runpy
import subprocess


class ReferenceBridge:
    """惰性启动、顺序请求；退出或异常时关闭管道并回收子进程。"""
    def __init__(self, command=None):
        self.command=command
        self.process=None

    def __enter__(self):
        return self

    def request(self,payload):
        if self.process is None:
            command=self.command
            if command is None:
                codecs=runpy.run_path(str(Path(__file__).with_name('bytecodecs.py')))
                assembly=codecs['_erisa_assembly']()
                command=['powershell.exe','-NoLogo','-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass',
                    '-File',str(Path(__file__).with_name('reference-primitives.ps1')),
                    '-Assembly',str(assembly),'-Persistent']
            self.process=subprocess.Popen(command,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,
                creationflags=0x08000000 if os.name=='nt' else 0)
        self.process.stdin.write(json.dumps(payload).encode('utf-8')+b'\n')
        self.process.stdin.flush()
        # 原生 helper 仅执行固定纯计算；程序外层仍管理总时限。父进程被取消时，
        # stdin EOF 也会结束 helper，避免为每个成员重新启动进程。
        line=self.process.stdout.readline(128*1024*1024+1)
        if not line:
            raise RuntimeError('Reference helper ended: '+self.process.stderr.read(2000).decode('utf-8','replace'))
        if len(line)>128*1024*1024 or not line.endswith(b'\n'):
            raise ValueError('Reference helper response exceeded the bounded frame')
        reply=json.loads(line)
        if reply.get('error'):
            raise ValueError(reply['error'])
        return reply['result']

    def __exit__(self,*_):
        if self.process is None:return
        try:
            try:self.process.stdin.close()
            except BrokenPipeError:pass
            try:self.process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=2)
        finally:
            self.process.stdout.close()
            self.process.stderr.close()
