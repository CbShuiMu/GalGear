"""Package the current Windows release from a clean portable staging directory."""
from pathlib import Path
import hashlib
import shutil
import tomllib
import zipfile

root = Path(__file__).resolve().parent.parent
version = tomllib.loads((root / 'Cargo.toml').read_text(encoding='utf-8'))['workspace']['package']['version']
artifacts = root / 'artifacts'
stage = artifacts / f'release-v{version}' / 'portable'
stage.mkdir(parents=True, exist_ok=False)
for name in ('7zip', 'ai-agent', 'waifu2x', 'magpie', 'locale-emulator'):
    shutil.copytree(root / 'tools' / name, stage / 'tools' / name,
                    ignore=shutil.ignore_patterns('logs', '*.log', '*.tmp', '__pycache__'))
for source, target in (
    (artifacts / 'GalGear.exe', 'GalGear.exe'),
    (artifacts / 'GalGear.sha256', 'GalGear.sha256'),
    (root / 'LICENSE', 'LICENSE'),
    (root / 'docs/bundled-tools.txt', '内置工具说明.txt'),
    (root / 'docs/cloud-storage.md', 'cloud-storage.md'),
):
    shutil.copy2(source, stage / target)
(stage / 'portable.flag').touch()
(stage / '使用说明.txt').write_text(f'''GalGear v{version}

完整解压后运行 GalGear.exe，请保留旁边的 tools 文件夹。
运行环境：Windows 10 / 11 x64、Microsoft Edge WebView2 Runtime。
便携包数据保存在同目录 data 文件夹；迁移时保留 data。
账号凭据由本机 Windows 凭据管理器保存，换机需重新配置。
Skills 启动时按需下载，首次运行需要网络与程序目录写入权限。
AI 宿主随包附带，无需安装 Python。内置工具说明见《内置工具说明.txt》。
云存储说明见 cloud-storage.md；GalGear.sha256 用于校验主程序。
''', encoding='utf-8')
required = ('GalGear.exe', 'portable.flag', 'LICENSE', 'tools/7zip/7z.exe',
            'tools/7zip/7z.dll', 'tools/7zip/License.txt',
            'tools/ai-agent/GalGear.AiAgent.exe',
            'tools/waifu2x/waifu2x-ncnn-vulkan.exe', 'tools/waifu2x/LICENSE',
            'tools/magpie/Magpie.exe', 'tools/magpie/LICENSE',
            'tools/magpie/config/config.json', 'tools/locale-emulator/LEProc.exe',
            'tools/locale-emulator/COPYING', 'tools/locale-emulator/LEConfig.xml')
for name in required:
    assert (stage / name).is_file(), name
expected = (stage / 'GalGear.sha256').read_text().split()[0]
assert hashlib.sha256((stage / 'GalGear.exe').read_bytes()).hexdigest() == expected
archive = artifacts / f'GalGear-{version}-win64-portable.zip'
with zipfile.ZipFile(archive, 'x', zipfile.ZIP_DEFLATED, compresslevel=6) as bundle:
    for item in sorted(stage.rglob('*')):
        if item.is_file():
            relative = item.relative_to(stage).as_posix()
            assert relative.split('/')[0] not in ('data', 'Galgear-GalgameReverse-Skill')
            assert item.suffix.lower() not in ('.log', '.pdb', '.pyc')
            bundle.write(item, relative)
with zipfile.ZipFile(archive) as bundle:
    assert bundle.testzip() is None
    assert set(required).issubset(bundle.namelist())
    assert bundle.read('GalGear.exe') == (stage / 'GalGear.exe').read_bytes()
    print(f'Portable verified: {len(bundle.namelist())} files, {archive.stat().st_size} bytes')
installer = artifacts / f'GalGear_{version}_x64-setup.exe'
shutil.copy2(root / f'target/release/bundle/nsis/{installer.name}', installer)
assert installer.read_bytes().startswith(b'MZ')
(artifacts / 'SHA256SUMS.txt').write_text(''.join(
    f'{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n'
    for path in (installer, archive)), encoding='ascii')
print('Installer copied; SHA256SUMS.txt generated')
