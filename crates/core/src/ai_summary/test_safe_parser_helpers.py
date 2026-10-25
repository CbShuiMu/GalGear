"""Data-only helpers exercised by actual model analysis and parser repairs."""
import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location('safe_parser_helpers_host', pathlib.Path(__file__).with_name('agent_host.py'))
host = importlib.util.module_from_spec(spec)
spec.loader.exec_module(host)


class SafeParserHelperTests(unittest.TestCase):
    def test_decode_failure_and_entropy_math_are_available_in_worker(self):
        source = '''import math
def extract(data, metadata):
    try:
        data.decode('utf-8')
    except UnicodeDecodeError:
        return {'decode_failed': True, 'log2': math.log2(8), 'log': math.log(1)}
    return {'decode_failed': False}
'''
        self.assertEqual(host.run_parser(source, b'\xff', {}), dict(decode_failed=True, log2=3.0, log=0.0))


if __name__ == '__main__':
    unittest.main()
