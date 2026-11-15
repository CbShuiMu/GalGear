"""固定原生桥接进程的复用与回收，不依赖游戏或外部库。"""
from pathlib import Path
import runpy
import sys
import unittest

NATIVE=runpy.run_path(str(Path(__file__).with_name('analysis_native.py')))
HELPER='''import json,os,sys
for line in sys.stdin:
    value=json.loads(line)
    print(json.dumps({'error':'fixture failure'} if value.get('fail') else
          {'result':{'pid':os.getpid(),'items':value['items']}}),flush=True)
'''


class NativeTests(unittest.TestCase):
    def test_reuses_one_process_and_reaps_it_after_success(self):
        bridge=NATIVE['ReferenceBridge']([sys.executable,'-u','-c',HELPER])
        with bridge:
            self.assertIsNone(bridge.process)
            first=bridge.request(dict(items=[[1,2],[]]))
            for value in range(20):
                result=bridge.request(dict(items=[value]))
                self.assertEqual(result['pid'],first['pid'])
                self.assertEqual(result['items'],[value])
            self.assertEqual(first['items'],[[1,2],[]])
            self.assertIsNone(bridge.process.poll())
        self.assertEqual(bridge.process.returncode,0)

    def test_request_error_and_caller_exception_close_helper(self):
        bridge=NATIVE['ReferenceBridge']([sys.executable,'-u','-c',HELPER])
        with self.assertRaisesRegex(RuntimeError,'caller'):
            with bridge:
                with self.assertRaisesRegex(ValueError,'fixture failure'):
                    bridge.request(dict(fail=True))
                self.assertEqual(bridge.request(dict(items=[]))['items'],[])
                raise RuntimeError('caller')
        self.assertIsNotNone(bridge.process.poll())

    def test_unexpected_helper_exit_is_reported_and_reaped(self):
        bridge=NATIVE['ReferenceBridge']([sys.executable,'-c','import sys; sys.stdin.readline(); sys.exit(3)'])
        with self.assertRaisesRegex(RuntimeError,'helper ended'):
            with bridge:bridge.request(dict(items=[]))
        self.assertEqual(bridge.process.returncode,3)


if __name__=='__main__':unittest.main()
