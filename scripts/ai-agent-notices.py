"""将新增宿主计算依赖的版本与许可证随离线运行时分发。"""
import importlib.metadata
import json
from pathlib import Path
import shutil
import sys


def main(output):
    versions = {}
    for name in ('cryptography', 'pefile'):
        package = importlib.metadata.distribution(name)
        versions[name] = package.version
        licenses = [entry for entry in package.files or [] if entry.name.startswith('LICENSE')]
        if not licenses:
            raise RuntimeError('Missing dependency license: ' + name)
        directory = output / 'licenses' / name
        directory.mkdir(parents=True, exist_ok=True)
        for entry in licenses:
            shutil.copyfile(package.locate_file(entry), directory / entry.name)
    (output / 'format-dependencies.json').write_text(json.dumps(versions, indent=2), encoding='utf-8')


if __name__ == '__main__':
    main(Path(sys.argv[1]))
