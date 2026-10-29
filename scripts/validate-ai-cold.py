"""Run an unseeded preparation using inputs selected by the production Rust code.

The input JSON is produced by the ai_cold_inputs example. Each invocation uses
a new output directory and work directory. Credentials travel through a pipe;
neither prior preparations nor local investigation reports enter model context.
"""
import argparse
import ctypes
from datetime import datetime, timezone, timedelta
import hashlib
import json
import os
from pathlib import Path
import runpy
import signal
import subprocess
import sys
import time
import uuid


REPO = Path(__file__).resolve().parents[1]


def read_json(path):
    return json.loads(Path(path).read_text('utf-8-sig'))


def write_json(path, value):
    Path(path).write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


def now():
    return datetime.now(timezone(timedelta(hours=8))).isoformat()


def digest(path):
    result = hashlib.sha256()
    with Path(path).open('rb') as source:
        for block in iter(lambda: source.read(1024 * 1024), b''):
            result.update(block)
    return result.hexdigest()


def stamp(path):
    path = Path(path).resolve()
    stat = path.stat()
    return dict(path=str(path), bytes=stat.st_size, modified_ns=stat.st_mtime_ns, sha256=digest(path))


def unchanged(stamps):
    changed = []
    for expected in stamps:
        path = Path(expected['path'])
        if not path.is_file() or stamp(path) != expected:
            changed.append(str(path))
    return dict(unchanged=not changed, checked_files=len(stamps), changed=changed)


def runtime_files(host):
    return sorted(path for path in host.parent.iterdir() if path.is_file()
                  and path.suffix in {'.py', '.ps1'} and not path.name.startswith('test_'))


def skill_files(skill):
    return sorted(path for path in skill.rglob('*') if path.is_file()
                  and '__pycache__' not in path.parts and path.suffix not in {'.pyc', '.pyo'})


def bindings(plan):
    checks = {key: unchanged(plan[key]) for key in ('runtime', 'skill')}
    for key, paths in [('runtime', runtime_files(Path(plan['host']))),
                       ('skill', skill_files(Path(plan['config']['skill'])))]:
        membership = [str(path.resolve()) for path in paths] == [item['path'] for item in plan[key]]
        checks[key]['membership_matches'] = membership
        checks[key]['unchanged'] = checks[key]['unchanged'] and membership
    checks['harness'] = unchanged([plan['test_harness']])
    checks['unchanged'] = all(item['unchanged'] for item in checks.values())
    return checks


def credential(settings):
    """Read the app's existing vault entry without exposing the secret."""
    from ctypes import wintypes as w
    class Credential(ctypes.Structure):
        _fields_ = [('Flags', w.DWORD), ('Type', w.DWORD), ('TargetName', w.LPWSTR),
                    ('Comment', w.LPWSTR), ('LastWritten', w.FILETIME),
                    ('CredentialBlobSize', w.DWORD), ('CredentialBlob', ctypes.POINTER(ctypes.c_byte)),
                    ('Persist', w.DWORD), ('AttributeCount', w.DWORD), ('Attributes', ctypes.c_void_p),
                    ('TargetAlias', w.LPWSTR), ('UserName', w.LPWSTR)]
    identity = os.path.abspath(settings).replace('/', '\\').lower()
    target = 'GalGear/AI/deepseek/' + hashlib.sha256(identity.encode()).hexdigest()
    pointer = ctypes.POINTER(Credential)()
    dll = ctypes.WinDLL('Advapi32.dll', use_last_error=True)
    dll.CredReadW.argtypes = [w.LPCWSTR, w.DWORD, w.DWORD, ctypes.POINTER(ctypes.POINTER(Credential))]
    dll.CredReadW.restype = w.BOOL
    dll.CredFree.argtypes = [ctypes.c_void_p]
    if not dll.CredReadW(target, 1, 0, ctypes.byref(pointer)):
        raise RuntimeError('Existing DeepSeek app credential is unavailable')
    try:
        token = json.loads(ctypes.string_at(pointer.contents.CredentialBlob,
                                           pointer.contents.CredentialBlobSize))['token']
    finally:
        dll.CredFree(pointer)
    if not isinstance(token, str) or not token or len(token) > 1024:
        raise RuntimeError('Invalid DeepSeek app credential')
    return token


def inventory(game):
    extensions = {'.csx', '.ks', '.txt', '.json', '.csv', '.rpy', '.mes', '.ws2',
                  '.lng', '.scn', '.s', '.scr', '.noa', '.xp3', '.arc', '.pfs',
                  '.pak', '.ypf', '.xml'}
    result = []
    def visit(folder, depth):
        for path in sorted(folder.iterdir())[:4096]:
            if path.name.startswith('.') or path.name.lower() in {'savedata', 'save', 'saves', 'node_modules'}:
                continue
            if path.is_symlink():
                raise ValueError('Cold inputs cannot contain symlink resources')
            if path.is_file() and path.suffix.lower() in extensions:
                result.append(dict(path=str(path), size=path.stat().st_size, modified=path.stat().st_mtime_ns))
            elif path.is_dir() and depth:
                visit(path, depth - 1)
            if len(result) > 20000:
                raise ValueError('Source inventory exceeds the production bound')
    visit(game, 3)
    return sorted(result, key=lambda item: item['path'])


def prepare_plan(inputs_path, settings_path, output, timeout, turns):
    data, settings = read_json(inputs_path), read_json(settings_path)
    game = Path(data['game']).resolve()
    selection = data['save_inputs']
    saves = Path(selection['save']).resolve()
    host = REPO / 'crates/core/src/ai_summary/agent_host.py'
    skill = REPO / 'Galgear-GalgameReverse-Skill/GalgameResourceExtract'
    output = Path(output).resolve()
    if output.exists():
        raise ValueError('Output directory already exists; every test requires a new directory')
    work = game / '.galgear-ai-summary' / ('.cold-' + uuid.uuid4().hex)
    if work.exists():
        raise ValueError('Cold work directory already exists')
    required = list(selection['required_saves'])
    if any(not Path(path).resolve().is_relative_to(saves) for path in required):
        raise ValueError('Production required save is outside the selected directory')
    source_inventory = data.get('inventory') or inventory(game)
    protected_paths = {Path(row['path']) for row in source_inventory}
    protected_paths.update(path for path in saves.iterdir() if path.is_file())
    protected_paths.update(Path(path) for path in required)
    for name in ('.agent-' + data.get('language', settings.get('language', 'zh-CN')) + '-deepseek',
                 data.get('language', settings.get('language', 'zh-CN'))):
        previous = game / '.galgear-ai-summary' / name
        if previous.is_dir():
            protected_paths.update(path for path in previous.iterdir() if path.is_file())
    config = dict(provider='deepseek', model=settings.get('ai_models', {}).get('deepseek') or 'deepseek-flash',
                  title=data['title'], language=data.get('language', settings.get('language', 'zh-CN')),
                  game=str(game), save=str(saves), skill=str(skill), work=str(work),
                  max_turns=turns, save_samples=1, inventory=source_inventory,
                  required_saves=required, save_candidates=selection.get('candidates', []),
                  runtime_limit_seconds=timeout, repair_reason=None, cold_start=True)
    plan = dict(created_at=now(), source='production ai_cold_inputs example', config=config,
                production_inputs=stamp(inputs_path), settings_path=os.path.abspath(settings_path),
                seed_artifacts_copied=False, previous_preparation_reused=False,
                host=str(host), timeout_seconds=timeout,
                runtime=[stamp(path) for path in runtime_files(host)],
                skill=[stamp(path) for path in skill_files(skill)],
                original_inputs=[stamp(path) for path in sorted(protected_paths)],
                test_harness=stamp(__file__))
    output.mkdir(parents=True, exist_ok=False)
    write_json(output / 'plan.json', plan)
    return output, plan


def progress(work):
    session = read_json(work / 'session.json') if (work / 'session.json').is_file() else {}
    checkpoint = read_json(work / 'checkpoint.json') if (work / 'checkpoint.json').is_file() else {}
    return dict(observed_at=now(), status=session.get('status'), stage=session.get('stage'),
                requests=session.get('requests_sent', 0), context_resets=session.get('context_resets', 0),
                dialogue_rows=sum(len(rows) for rows in checkpoint.get('parts', {}).values()),
                save_tests=len(checkpoint.get('tests', {})), result_present=(work / 'result.json').is_file())


def cache_read_audit(events, config):
    work = Path(config['work']).resolve()
    roots = {key: Path(config[key]).resolve() for key in ('game', 'save', 'work', 'skill')}
    outside = []
    def inspect(value, inherited=None):
        if isinstance(value, list):
            for item in value:
                inspect(item, inherited)
        elif isinstance(value, dict):
            root = value.get('root', inherited)
            for key, item in value.items():
                if key in {'path', 'archive', 'selected', 'configuration'} and isinstance(item, str):
                    base = roots.get('game' if key in {'archive', 'selected', 'configuration'} else root)
                    if base is not None:
                        target = (base / item).resolve()
                        if ('.galgear-ai-summary' in (part.casefold() for part in target.parts)
                                and not target.is_relative_to(work)):
                            outside.append(str(target))
                elif isinstance(item, (dict, list)) and key not in {'proof', 'patch'}:
                    inspect(item, root)
    attempts, successful = [], []
    for event in events:
        if event.get('tool', '').startswith(('read_', 'inspect_', 'list_', 'load_', 'test_', 'run_', 'verify_')):
            outside.clear()
            inspect(event.get('arguments', {}))
            if outside:
                attempt = dict(tool=event['tool'], denied=bool(event.get('error')), paths=sorted(set(outside)))
                attempts.append(attempt)
                if not attempt['denied']:
                    successful.append(attempt)
    return dict(passed=not successful, attempts=attempts, successful_reads=successful)


def stop_worker(process):
    """Bounded cleanup of this run's process tree, including on interruption."""
    if process.poll() is None:
        if os.name == 'nt':
            try:
                subprocess.run(['taskkill', '/PID', str(process.pid), '/T', '/F'], timeout=10,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, creationflags=0x08000000)
            except (OSError, subprocess.TimeoutExpired):
                pass
        else:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        if process.poll() is None:
            process.kill()
    try:
        return process.communicate(timeout=15)
    except subprocess.TimeoutExpired as error:
        # A failed tree termination must not make cleanup wait indefinitely on
        # an inherited stdout handle. Preserve only bytes already received.
        return error.output or b'', error.stderr or b''


def native_archive_metadata(host, checkpoint, work, game, path, data):
    """Rebuild optional native declarations from this run's real archive bytes."""
    origin = next(((Path(folder).resolve(), Path(archive).resolve())
                   for folder, archive in checkpoint.get('extracted_from', {}).items()
                   if path.is_relative_to(Path(folder).resolve())), None)
    if origin is None:
        return None
    folder, archive = origin
    if not folder.is_relative_to(work) or not archive.is_relative_to(game):
        raise ValueError('Native metadata provenance escaped this game/run')
    try:
        return host['ARCHIVES']['native_member_metadata'](
            archive, path.relative_to(folder).as_posix(), data)
    except ValueError:
        # Unsupported/ambiguous raw binding supplies no native declarations.
        # The existing independently replayed member_filter proof still applies.
        return None


def replay_member_boundary(host, checkpoint, work, game, path, data, output):
    """Check the original outer member before any internal PSB document layers."""
    origins = [(Path(folder).resolve(), Path(archive).resolve())
               for folder, archive in checkpoint.get('extracted_from', {}).items()
               if path.is_relative_to(Path(folder).resolve())]

    def origin():
        if len(origins) != 1:
            raise ValueError('Document/member decoding needs unique extraction provenance')
        folder, archive = origins[0]
        if not folder.is_relative_to(work) or not archive.is_relative_to(game):
            raise ValueError('Document/member provenance escaped this game/run')
        return folder, archive

    outer = data
    if isinstance(output, dict) and output.get('member_filter'):
        folder, archive = origin()
        outer = host['EVIDENCE']['view'](data, output['member_filter'])
        host['ARCHIVES']['verify_member'](archive, path.relative_to(folder).as_posix(), outer, original=data)
    if host['ARCHIVES']['requires_document_member_verification'](output, outer):
        if origins:
            folder, archive = origin()
            host['ARCHIVES']['verify_document_member'](
                archive, path.relative_to(folder).as_posix(), data, outer, output)
        elif path.is_relative_to(work):
            raise ValueError('Internal document decoding has no actual extraction provenance')
        elif not path.is_relative_to(game):
            raise ValueError('Internal document input escaped this game')
        else:
            host['ARCHIVES']['verify_document_member'](None, None, data, outer, output)
    return outer


def audit_prepared(host, config, checkpoint, result):
    """Independently execute the model's fresh parsers and replay host proofs."""
    work, game = Path(config['work']), Path(config['game'])
    artifacts = result.get('artifacts', {})
    expected = {'script_extractor.py', 'save_extractor.py', 'dialogue.jsonl'}
    if not expected.issubset(artifacts) or any(not (work / key).resolve().is_relative_to(work)
            or not (work / key).is_file()
            or digest(work / key) != value for key, value in artifacts.items()):
        return dict(passed=False, category='missing_or_changed_preparation_artifacts')
    dialogue = [json.loads(line) for line in (work / 'dialogue.jsonl').read_text('utf-8').splitlines() if line]
    story_rows = [row for row in dialogue if not host['resume_marker'](row)]
    script = (work / 'script_extractor.py').read_text('utf-8')
    save = (work / 'save_extractor.py').read_text('utf-8')
    replayed, script_checks = [], []
    targets = checkpoint.get('speaker_resource_targets', {})
    def metadata(path, with_dialogue=False, data=None):
        canonical = lambda item: os.path.normcase(str(Path(item).resolve()))
        scoped = any(canonical(item) == canonical(path) for item in targets.values())
        resources = {key: value for key, value in checkpoint.get('speaker_resources', {}).items()
                     if (not scoped and key not in targets) or key in targets
                     and canonical(targets[key]) == canonical(path)}
        values = dict(path=str(path), name=path.name, language=config['language'], speaker_resources=resources,
                      resources={key: value for key, value in checkpoint.get('script_resources', {}).items()
                                 if key not in targets or canonical(targets[key]) == canonical(path)})
        if with_dialogue:
            values['dialogue'] = dialogue
        if data is not None:
            native = native_archive_metadata(host, checkpoint, work, game, path, data)
            if native is not None:
                values['native_archive'] = native
        return values, scoped
    for name, accepted in checkpoint.get('parts', {}).items():
        path = Path(name).resolve()
        if not path.is_relative_to(work) and not path.is_relative_to(game):
            raise ValueError('Script input escaped this game/run')
        data = path.read_bytes()
        values, scoped = metadata(path, data=data)
        output = host['run_parser'](script, data, values)
        evidence = host['EVIDENCE']
        member_data = replay_member_boundary(host, checkpoint, work, game, path, data, output)
        regions = host['WORKFLOW']['required_table_regions'](Path(config['skill']), member_data)
        rows = evidence['script_rows'](data, output, required_table_regions=regions,
            speaker_resources=values['speaker_resources'], source=str(path), required_speaker_scope=scoped)
        raw = evidence['view'](member_data, output.get('view', [])) if isinstance(output, dict) else data
        native = evidence['ADAPTERS']['attest_native_nodes'](raw, rows, span=evidence['span'],
            integer=evidence['integer'], view=evidence['view'],
            profiles=output.get('native_profiles') if isinstance(output, dict) else None)
        source = str(path.relative_to(work) if path.is_relative_to(work) else path.relative_to(game))
        normalized = []
        for item in rows:
            row = dict(source=source, position=item['position'], text=item['text'], speaker=item['speaker'])
            if native.get(item['position']):
                row['_native'] = native[item['position']]
            normalized.append(row)
        replayed.extend(normalized)
        script_checks.append(dict(path=str(path), rows=len(rows), passed=normalized == accepted))
    validated = result.get('validated_saves', {})
    required = config['required_saves'] or list(validated)
    save_checks = []
    for name in required:
        path = Path(name).resolve()
        accepted = next((item for key, item in validated.items()
                         if Path(key).resolve() == path), {})
        if not path.is_relative_to(Path(config['save']).resolve()):
            raise ValueError('Save input escaped the selected directory')
        values, _ = metadata(path, True)
        facts = host['run_parser'](save, path.read_bytes(), values, verify_save=True)
        resume = facts.get('resume', {})
        resume_ok = (accepted.get('coverage') == 'resume_node' and isinstance(resume, dict)
                     and resume.get('profile') == 'kag_parser_label_v1'
                     and resume.get('state_role') == 'resume_node' and facts.get('anchors') == []
                     and resume.get('source') == facts.get('source')
                     and type(resume.get('position')) is int and resume['position'] == facts.get('line'))
        current_ok = (accepted.get('coverage') == 'node' and len(facts.get('anchors', [])) == 1
                      and sum(row.get('source') == facts.get('source') and row.get('position') == facts.get('line')
                              and row.get('text') == facts['anchors'][0] for row in dialogue) == 1
                      and accepted.get('matching_anchors') == facts['anchors'])
        passed = ((current_ok or resume_ok) and accepted.get('facts') == facts
                  and accepted.get('sha256') == digest(path)
                  and accepted.get('save_verification_version') == host['SAVE_VERIFICATION_VERSION'])
        save_checks.append(dict(path=str(path), passed=bool(passed)))
    coverage = result.get('transcript_coverage', {})
    speaker = result.get('speaker_verification', {})
    key = lambda item: (item['source'], item['position'], item['text'])
    expected_members = {(group['archive'], member.casefold())
                        for group in checkpoint.get('required_script_sets', {}).values() for member in group['members']}
    covered_members = set()
    for name in checkpoint.get('parts', {}):
        for folder, archive in checkpoint.get('extracted_from', {}).items():
            if Path(name).is_relative_to(Path(folder)):
                covered_members.add((archive, Path(name).relative_to(folder).as_posix().casefold()))
    complete = (bool(expected_members) and expected_members.issubset(covered_members)
                and not coverage.get('pending_members')
                and coverage.get('expected_members') == len(expected_members)
                and coverage.get('covered_members') == len(expected_members & covered_members))
    stamps = result.get('source_stamps', {})
    sources_ok = bool(stamps) and all(Path(name).is_file()
        and [Path(name).stat().st_size, Path(name).stat().st_mtime_ns] == value for name, value in stamps.items())
    versions = all(result.get(key) == host[constant] for key, constant in
                   [('contract_version', 'CONTRACT_VERSION'), ('save_verification_version', 'SAVE_VERIFICATION_VERSION'),
                    ('script_verification_version', 'SCRIPT_VERIFICATION_VERSION')])
    speakers = (speaker.get('version') == 1 and speaker.get('named_rows') == sum(bool(row['speaker']) for row in dialogue)
                and (not speaker.get('external_rows') or speaker.get('scope_version') == 2))
    passed = (versions and complete and sources_ok and speakers and bool(story_rows) and bool(script_checks) and bool(save_checks)
              and all(row['passed'] for row in script_checks + save_checks)
              and sorted(replayed, key=key) == sorted(dialogue, key=key)
              and result.get('dialogue_count') == len(story_rows)
              and result.get('artifact_rows_count', len(dialogue)) == len(dialogue)
              and result.get('resume_marker_count', len(dialogue)-len(story_rows)) == len(dialogue)-len(story_rows))
    return dict(passed=bool(passed), versions_match=versions, transcript_complete=complete,
                source_stamps_unchanged=sources_ok, speaker_verification_complete=speakers, dialogue_count=len(story_rows),
                resume_marker_count=len(dialogue)-len(story_rows), scripts=script_checks, saves=save_checks)


def run(args):
    output, plan = prepare_plan(args.inputs, args.settings, args.output, args.timeout, args.turns)
    if args.plan_only:
        print(json.dumps(dict(status='planned', plan=str(output / 'plan.json'), model_requests=0)))
        return 0
    config = dict(plan['config'])
    work = Path(config['work'])
    token = credential(args.settings)
    work.mkdir(parents=True, exist_ok=False)
    write_json(output / 'start.json', dict(started_at=now(), work_empty_before_launch=not any(work.iterdir()),
               original_cache_reused=False, runtime=unchanged(plan['runtime']), skill=unchanged(plan['skill'])))
    if not bindings(plan)['unchanged']:
        raise ValueError('Runtime or skill changed during planning; start a fresh test after changes finish')
    config['token'] = token
    started = time.monotonic()
    process = subprocess.Popen([sys.executable, '-I', '-S', plan['host']], stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               creationflags=0x08000000 if os.name == 'nt' else 0,
                               start_new_session=os.name != 'nt')
    request = json.dumps(config, ensure_ascii=False).encode('utf-8')
    timed_out, last_observed = False, started
    print(json.dumps(dict(status='running', output=str(output), work=str(work)), ensure_ascii=False), flush=True)
    try:
        while True:
            remaining = args.timeout - (time.monotonic() - started)
            if remaining <= 0:
                timed_out = True
                stdout, stderr = stop_worker(process)
                break
            try:
                stdout, stderr = process.communicate(input=request, timeout=min(60, remaining))
                break
            except subprocess.TimeoutExpired:
                request = None
                if time.monotonic() - last_observed >= 300:
                    try:
                        snapshot = progress(work)
                    except (OSError, ValueError):
                        snapshot = dict(observed_at=now(), status='no_stable_progress_snapshot')
                    write_json(output / ('observation-' + str(time.time_ns()) + '.json'), snapshot)
                    print(json.dumps(snapshot, ensure_ascii=False), flush=True)
                    last_observed = time.monotonic()
    finally:
        if process.poll() is None:
            stop_worker(process)
        config.pop('token', None)
    (output / 'stdout.json').write_bytes(stdout.replace(token.encode(), b'[redacted]'))
    (output / 'stderr.txt').write_bytes(stderr.replace(token.encode(), b'[redacted]'))
    token = None
    write_json(output / 'exit.json', dict(completed_at=now(), timed_out=timed_out,
               elapsed_seconds=round(time.monotonic() - started, 3), exit_code=process.returncode))
    original, protocol = unchanged(plan['original_inputs']), bindings(plan)
    host = runpy.run_path(plan['host'])
    session = read_json(work / 'session.json') if (work / 'session.json').is_file() else {}
    checkpoint = read_json(work / 'checkpoint.json') if (work / 'checkpoint.json').is_file() else {}
    result = read_json(work / 'result.json') if (work / 'result.json').is_file() else {}
    events = read_json(work / 'events.json') if (work / 'events.json').is_file() else []
    cache_audit = cache_read_audit(events, plan['config'])
    try:
        verification = audit_prepared(host, plan['config'], checkpoint, result)
    except Exception as error:
        verification = dict(passed=False, category='independent_reverification_error',
                            error_type=type(error).__name__, detail=str(error)[:2000])
    report = dict(completed_at=now(), passed=bool(not timed_out and process.returncode == 0
        and session.get('status') == 'completed' and session.get('closed') and session.get('requests_sent', 0) > 0
        and original['unchanged'] and protocol['unchanged']
        and cache_audit['passed'] and verification['passed']),
        cold_start=True, seed_artifacts_copied=False, prior_preparation_reused=False,
        original_inputs=original, protocol=protocol, cache_reads=cache_audit, verification=verification,
        final_progress=progress(work), model_requests_from_audit=0)
    write_json(output / 'audit.json', report)
    print(json.dumps(report, ensure_ascii=False, indent=2), flush=True)
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    sys.stdout.reconfigure(encoding='utf-8')
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inputs', type=Path, required=True)
    parser.add_argument('--settings', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--timeout', type=int, default=2700)
    parser.add_argument('--turns', type=int, default=150)
    parser.add_argument('--plan-only', action='store_true')
    arguments = parser.parse_args()
    if not 1 <= arguments.timeout <= 7200 or not 1 <= arguments.turns <= 150:
        parser.error('Timeout must be 1..7200 seconds and turns must be 1..150')
    raise SystemExit(run(arguments))
