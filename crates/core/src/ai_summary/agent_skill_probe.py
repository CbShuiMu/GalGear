"""Bounded read-only Skill probes; observations never qualify a saved node.

Only fixed installed entry points run in a short-lived worker. Input/tool hashes
bind local reusable results, including negative diagnoses; no model command or
game Python is executable through this bridge.
"""
import hashlib
import json
import pathlib
import runpy
import subprocess
import sys
import tempfile
import time


MAX_INPUT = 32 * 1024 * 1024
MAX_REPORT = 64 * 1024
VERSION = 2


def script_reuse_candidates(module, inspection, frame_names=()):
    """Turn observed call samples into reviewable adapter arguments, never install them."""
    proposals = []
    for callee in inspection.get('call_abi_candidates', []):
        samples = callee.get('samples', [])
        signatures = {(s['scalar_arguments'], s['call_arity'], s['speaker_argument']) for s in samples}
        if not samples or len(signatures) != 1:
            continue
        scalars, arity, speaker = next(iter(signatures))
        profile = dict(message_callees=[callee['callee']], scalar_arguments=scalars,
                       call_arity=arity, speaker_argument=speaker)
        # Only propose this scope spelling when actual function and save-frame
        # names demonstrate the relation; no game-title or filename heuristics.
        if any('@' in f['name'] and f['name'].rsplit('@', 1)[-1] in frame_names for f in module['functions']):
            profile['scope_transform'] = dict(kind='last_component', separator='@')
        rows = []
        for sample in samples:
            row = dict(position=sample['instruction_position'],
                       text=module['strings'][sample['text_index']]['text'][:160], speaker=None)
            if speaker:
                at = sample['instruction_position'] - 4
                index = int.from_bytes(module['code'][at:at + 4], 'little')
                if index < len(module['strings']):
                    row['speaker'] = module['strings'][index]['text'][:80] or None
            rows.append(row)
        proposals.append(dict(function='reuse_engine_adapter',
            arguments=dict(adapter='entis_cotopha', kind='script', profile=profile), sample_rows=rows,
            analysis_only=True, requires='Confirm displayed dialogue/name semantics from these actual samples, select this resource, then reuse and test_program. A candidate is not validation.'))
    return proposals



def write_artifact(path, data):
    if path.exists() and (path.is_symlink() or not path.is_file()):
        raise ValueError('Probe artifact must be a regular file')
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as file:
        temporary = pathlib.Path(file.name)
        file.write(data)
    try:
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def matches(data, kind):
    if kind == 'script':
        return data[:8] == b'Entis\x1a\0\0' and data[16:56].startswith(b'Cotopha Image file')
    if kind == 'save':
        if data[:2] != b'BM':
            return False
        at = int.from_bytes(data[2:6], 'little')
        return 0 <= at <= len(data) - 64 and data[at:at+8] == b'Entis\x1a\0\0' and data[at+16:at+56].startswith(b'cotomi context file')
    return False


def files(skill):
    names = ['scripts/inspect_cotomi_save.py', 'scripts/parse_cotopha_csx.py',
             'tools/GARbro-cli/GalArchiveExport.exe']
    paths = [(skill / name).resolve() for name in names]
    missing = [name for name, path in zip(names, paths)
               if not path.is_relative_to(skill) or not path.is_file()]
    if missing:
        raise ValueError('Installed Skill lacks required files: ' + ', '.join(missing))
    # DLL changes can alter a decoder even if the launcher is unchanged.
    paths.extend(sorted(paths[-1].parent.glob('*.dll')))
    return paths


def inspect(config):
    """Worker entry: inspect a recognized slot with the installed Skill."""
    skill = pathlib.Path(config['skill']).resolve()
    paths = files(skill)
    path = pathlib.Path(config['input']).resolve()
    if not path.is_file() or not 0 < path.stat().st_size <= MAX_INPUT:
        raise ValueError('Skill probe input must be a nonempty file of at most 32 MiB')
    data = path.read_bytes()
    if not matches(data, 'save'):
        raise ValueError('Input header does not match Cotomi save')
    sys.path.insert(0, str(paths[0].parent))
    helper = runpy.run_path(str(paths[0]))
    report, decoded = helper['inspect'](path, paths[2])
    adapter = runpy.run_path(str(pathlib.Path(__file__).with_name('entis_adapter.py')))
    observations = adapter['observed_save_fields'](data, decoded)
    # Keep the raw decoded file available for focused reads; no current-caption
    # or current-PC semantics are assigned to a previously unknown save version.
    output = pathlib.Path(config['output']).resolve()
    output.mkdir(parents=True, exist_ok=True)
    write_artifact(output / 'context.bin', decoded)
    return dict(ok=True,
                tool='scripts/inspect_cotomi_save.py', input_sha256=hashlib.sha256(data).hexdigest(),
                report=report, **observations,
                decoded_path=str(output / 'context.bin'),
                instruction='Reuse this decoded context and raw field evidence. Infer no current node from title/stack names alone. Write only missing version semantics, then test_program with independent evidence.')


class SkillProbes:
    def __init__(self, skill, work, command):
        self.skill, self.work, self.command = skill, work, command

    def run(self, path, timeout=40):
        started = time.monotonic()
        if timeout <= 0:
            raise subprocess.TimeoutExpired(self.command, timeout)
        if not 0 < path.stat().st_size <= MAX_INPUT:
            raise ValueError('Probe input exceeds 32 MiB')
        input_hash = digest(path)
        tool_identity = [(str(p.relative_to(self.skill)), digest(p)) for p in files(self.skill)]
        key = hashlib.sha256(json.dumps([VERSION, input_hash, tool_identity,
            digest(pathlib.Path(__file__)), digest(pathlib.Path(__file__).with_name('entis_adapter.py'))], sort_keys=True).encode()).hexdigest()
        directory = self.work / 'skill-probes' / key
        if not directory.resolve().is_relative_to(self.work.resolve()):
            raise ValueError('Skill probe output escapes work directory')
        directory.mkdir(parents=True, exist_ok=True)
        report_path = directory / 'report.json'
        if (report_path.is_file() and report_path.stat().st_size <= MAX_REPORT
                and report_path.resolve().is_relative_to(directory.resolve())):
            try:
                record = json.loads(report_path.read_text(encoding='utf-8'))
                decoded = directory / 'context.bin'
                if (record.get('key') == key and record.get('ok') is True
                        and decoded.is_file() and decoded.stat().st_size <= MAX_INPUT
                        and decoded.resolve().is_relative_to(directory.resolve())
                        and record.get('decoded_sha256') == digest(decoded)):
                    record['decoded_path'] = str(decoded)
                    return dict(record, reused=True, elapsed_seconds=round(time.monotonic() - started, 3))
            except (OSError, ValueError, TypeError):
                pass
        payload = dict(skill=str(self.skill), input=str(path), output=str(directory))
        # File-backed stdout/stderr bound memory even if an installed helper is noisy.
        with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as errors:
            remaining = min(40, timeout - (time.monotonic() - started))
            if remaining <= 0:
                raise subprocess.TimeoutExpired(self.command, timeout)
            result = subprocess.run(self.command + ['--skill-probe'], input=json.dumps(payload).encode(),
                                    stdout=output, stderr=errors, timeout=remaining,
                                    creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
            output.seek(0)
            raw = output.read(MAX_REPORT + 1)
            if result.returncode or len(raw) > MAX_REPORT:
                errors.seek(0)
                detail = errors.read(2000).decode('utf-8', errors='replace').strip()[-500:]
                raise ValueError('Installed Skill probe failed or exceeded its report limit: ' + detail)
            record = json.loads(raw)
        if record.get('input_sha256') != input_hash or digest(path) != input_hash:
            raise ValueError('Probe input changed during inspection')
        record.update(key=key, elapsed_seconds=round(time.monotonic() - started, 3),
                      decoded_sha256=digest(directory / 'context.bin'))
        write_artifact(report_path, json.dumps(record, ensure_ascii=False).encode('utf-8'))
        return dict(record, reused=False)
