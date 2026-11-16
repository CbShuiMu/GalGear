"""媒体任务的受限文件操作：来源只读、转换器固定、所有交付写入本轮目录。"""
import hashlib
import json
import os
from pathlib import Path, PureWindowsPath
import shutil
import stat
import subprocess
import time
import uuid

IMAGE_EXTENSIONS = {'.png', '.jpg', '.jpeg', '.bmp', '.webp', '.gif', '.tga', '.dds', '.tlg', '.hg2', '.hg3', '.g00', '.g01', '.eri', '.rct', '.rc8', '.pimg', '.dpng', '.crx', '.pb3', '.pb2', '.gyu', '.abm', '.grd', '.pdt', '.pna', '.pgd', '.psd', '.img'}
AUDIO_EXTENSIONS = {'.ogg', '.wav', '.mp3', '.flac', '.opus', '.m4a', '.aac', '.wma', '.mid', '.midi', '.nwa', '.hca', '.adx', '.at9', '.wem', '.xwma', '.spx', '.dwa', '.vaw', '.wa1', '.wa2', '.waf', '.nxa'}


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def checked(path, root):
    """拒绝 Windows junction 和每级链接；resolve 后在根内仍不足以排除链接别名。"""
    if not path.is_relative_to(root):
        raise ValueError('Path outside allowed media root')
    for item in (path, *path.parents):
        if not item.is_relative_to(root):
            break
        info = item.lstat()
        if stat.S_ISLNK(info.st_mode) or getattr(info, 'st_file_attributes', 0) & 0x400:
            raise ValueError('Media paths cannot contain links or reparse points')
    if not path.resolve().is_relative_to(root.resolve()):
        raise ValueError('Path outside allowed media root')
    return path


def source(agent, root, relative):
    if root not in ('game', 'work'):
        raise ValueError('Media inputs must come from game or work')
    name = Path(relative)
    if name.is_absolute() or PureWindowsPath(relative).drive or '..' in name.parts or ':' in relative:
        raise ValueError('Use a relative media input path')
    base = agent.host.roots[root]
    if root == 'game' and '.galgear-ai-summary' in (p.casefold() for p in name.parts):
        raise ValueError('Use work for current task files; previous task outputs are not inputs')
    path = checked(base / name, base)
    if root == 'work' and path.is_relative_to(agent.work / 'delivery'):
        raise ValueError('Delivery files cannot be re-exported as sources')
    return path


def select(agent, args):
    base = source(agent, args['root'], args['path'])
    if args.get('_paths'):
        directory = base
        paths = [checked(base / name, base) for name in args['_paths']]
    elif base.is_file():
        paths = [base]
        directory = base.parent
    else:
        directory = base
        pattern = args.get('pattern')
        if not isinstance(pattern, str) or not pattern or '..' in Path(pattern).parts or PureWindowsPath(pattern).drive or pattern.startswith(('/', '\\')):
            raise ValueError('Supply a relative glob selecting the requested media')
        paths = sorted(base.glob(pattern))
    result = []
    for path in paths:
        checked(path, agent.host.roots[args['root']])
        if path.is_dir():
            continue
        relative = path.relative_to(agent.host.roots[args['root']])
        if args['root'] == 'game' and ('.galgear-ai-summary' in relative.parts or
                len(relative.parts) >= 3 and relative.parts[0].casefold() in ('cgs', 'audios')
                and relative.parts[1].startswith('media-') and (agent.host.roots['game'] / relative.parts[0] / relative.parts[1] / 'manifest.json').is_file()):
            continue
        if not path.is_file() or path.stat().st_size == 0:
            continue
        result.append(path)
        if len(result) > 200000:
            raise ValueError('Too many selected media files; narrow the selection')
    if not result:
        raise ValueError('No nonempty media files matched')
    if args.get('_base') is not None:
        directory = source(agent, args['root'], args['_base'])
        if not directory.is_dir() or any(not p.is_relative_to(directory) for p in result):
            raise ValueError('Media source base does not contain selected files')
    return directory, result


def copy_member(agent, path, relative, provenance):
    target = agent.delivery / relative
    if target.exists():
        if digest(target) == digest(path):
            return False
        raise ValueError('Different resources would overwrite the same media path')
    target.parent.mkdir(parents=True, exist_ok=True)
    checked(target.parent, agent.delivery)
    before = digest(path)
    with path.open('rb') as original, target.open('xb') as output:
        shutil.copyfileobj(original, output, 1024 * 1024)
    if digest(target) != before or digest(path) != before:
        target.unlink()
        raise ValueError('Media source changed while copying')
    agent.files.append(dict(path=target.relative_to(agent.delivery).as_posix(), size=target.stat().st_size,
                            sha256=before, source=provenance))
    return True


def export(agent, args):
    directory, paths = select(agent, args)
    extensions = IMAGE_EXTENSIONS if agent.kind == 'cg' else AUDIO_EXTENSIONS
    if any(path.suffix.casefold() not in extensions for path in paths):
        raise ValueError('Selection contains unrecognized media extensions; narrow the glob or decode images with convert_images')
    group = hashlib.sha256(str(directory).encode()).hexdigest()[:12]
    count = 0
    for path in paths:
        if time.monotonic() >= agent.deadline:
            raise TimeoutError('Media extraction time budget exhausted')
        agent.stamp(path)
        count += copy_member(agent, path, Path('raw') / group / path.relative_to(directory),
                             dict(root=args['root'], path=path.relative_to(agent.host.roots[args['root']]).as_posix()))
    return dict(copied=count, total_files=len(agent.files), group='raw/'+group)


def convert_images(agent, args):
    if agent.kind != 'cg':
        raise ValueError('Image conversion is only available for CG extraction')
    executable = agent.host.roots['skill'] / 'tools/GARbro-cli/GalImageExport.exe'
    if not executable.is_file():
        raise ValueError('Installed Skill lacks GalImageExport.exe; update the Skill package')
    document = 'commands/garbro-images.md'
    record = agent.host.documents.get(document, {})
    if not record.get('complete') or record.get('sha256') != digest(agent.host.path('skill', document)):
        raise ValueError('Read complete commands/garbro-images.md before converting images')
    mode = args.get('mode', 'files')
    output = agent.work / ('images-' + uuid.uuid4().hex)
    if mode == 'archive':
        path = source(agent, args['root'], args['path'])
        if not path.is_file():
            raise ValueError('Select an actual image archive')
        agent.stamp(path)
        job = dict(mode='archive', source=str(path), output=str(output), all=bool(args.get('all', False)))
        if args.get('filter'):
            job['filter'] = args['filter']
    elif mode == 'files':
        directory, paths = select(agent, args)
        for path in paths:
            agent.stamp(path)
        job = dict(mode='files', source=str(directory), files=[str(path) for path in paths], output=str(output))
    else:
        raise ValueError('Image mode must be archive or files')
    budget = max(1, min(240, int(agent.deadline - time.monotonic()) - 5))
    job['seconds'] = budget
    job_path = output.with_suffix('.job.json')
    agent.api['atomic_json'](job_path, job)
    completed = subprocess.run([str(executable), str(job_path)], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        timeout=budget+5, creationflags=0x08000000 if os.name == 'nt' else 0)
    manifest = output / 'manifest.jsonl'
    if completed.returncode or not manifest.is_file():
        raise ValueError('Skill image conversion failed: ' + completed.stderr.decode('utf-8', 'replace')[-2000:])
    errors, count = [], 0
    with manifest.open(encoding='utf-8-sig') as stream:
        for line in stream:
            if not line.strip():
                continue
            entry = json.loads(line)
            if entry.get('kind', '').endswith(('_error', '_deferred')):
                errors.append(entry)
            if entry.get('kind') not in ('raw', 'png'):
                continue
            path = checked(Path(entry['path']), output)
            if not path.is_file() or digest(path) != entry.get('sha256'):
                raise ValueError('Image converter output/hash mismatch')
            if path.stat().st_size:
                group = hashlib.sha256(job['source'].encode()).hexdigest()[:12]
                count += copy_member(agent, path, Path('images') / group / path.relative_to(output), entry)
                if agent.is_batch:
                    agent.progress('extracting')
    if errors:
        # 部分成功必须保持可见，不能让模型的 finish 参数清掉工具报告的失败。
        agent.warnings.append('Image batch '+output.name+': '+str(len(errors))+' failed/deferred members; see manifest.json')
        agent.conversion_errors.extend(errors)
    if not count:
        raise ValueError('Image conversion produced no publishable resources')
    return dict(files=count, errors=errors[:12], failed=len(errors), manifest=manifest.relative_to(agent.work).as_posix())


def finish(agent, args):
    if not agent.files:
        raise ValueError('No media extracted; an empty directory is not a successful result')
    if not isinstance(args.get('scope'), str) or not args['scope'].strip():
        raise ValueError('Describe the actual selected resource scope')
    warnings = args.get('warnings', [])
    if not isinstance(warnings, list) or len(warnings) > 64 or any(not isinstance(w, str) or len(w) > 2000 for w in warnings):
        raise ValueError('Provide bounded, explicit extraction limitations')
    agent.check_inputs()
    manifest = dict(schema=1, kind=agent.kind, files=agent.files, warnings=(agent.warnings + warnings)[:128], scope=args['scope'][:8000], conversion_errors=agent.conversion_errors)
    agent.api['atomic_json'](agent.delivery / 'manifest.json', manifest)
    agent.emit('extraction-report.json', dict(scope=manifest['scope'], warnings=manifest['warnings'], conversion_errors=agent.conversion_errors))
    agent.done = dict(schema=1, kind=agent.kind, files=len(agent.files))
    return agent.done
