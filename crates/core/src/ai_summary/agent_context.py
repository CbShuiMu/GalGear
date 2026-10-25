"""Bound API context without changing the local extraction evidence or audit trail."""
import hashlib
import json

VERSION = 17
MAX_CONTEXT_BYTES = 480000
MAX_PROGRESS_MESSAGE_BYTES = 24000
MAX_TOOL_BYTES = 8192
RECENT_OBSERVATIONS = 4
PROGRAM_SOURCE_BYTES = 12000
PROGRAM_SOURCES_BYTES = 24000
HELPER_SOURCE_BYTES = 12000
HELPER_SOURCES_BYTES = 24000
ANALYSIS_SOURCE_BYTES = 6000
ANALYSIS_SOURCES_BYTES = 12000
RECENT_ANALYSIS_SOURCES = 2


def encode(value):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode('utf-8')


def clip(text, budget):
    return text.encode('utf-8')[:max(0, budget)].decode('utf-8', 'ignore')


def archive_result(work, value):
    data = encode(value)
    digest = hashlib.sha256(data).hexdigest()
    path = work / 'tool-results' / (digest + '.json')
    path.parent.mkdir(exist_ok=True)
    if not path.parent.resolve().is_relative_to(work.resolve()) or path.is_symlink():
        raise ValueError('Tool output must stay inside the work directory')
    if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != digest:
        path.write_bytes(data)
    return str(path.relative_to(work)), data


def structured_preview(value, budget, depth=0):
    """Preview JSON as data, never as another escaped JSON document.

    The caller checks the final encoded size. This estimate deliberately leaves
    room for escaping and container keys; archived full results remain exact.
    """
    if value is None or isinstance(value, (bool, int, float)):
        return value
    if isinstance(value, str):
        shortened = clip(value, max(0, budget // 2))
        return shortened if shortened == value else dict(text=shortened, total_bytes=len(value.encode('utf-8')))
    if not isinstance(value, (dict, list)):
        return dict(type=type(value).__name__)
    if depth >= 4 or budget < 100:
        return dict(type=type(value).__name__, total_entries=len(value))
    entries = list(value.items()) if isinstance(value, dict) else list(enumerate(value))
    count = min(len(entries), 8)
    per_entry = max(0, (budget - 100) // max(1, count))
    if isinstance(value, dict):
        result = {clip(str(key), 160): structured_preview(item, per_entry, depth + 1)
                  for key, item in entries[:count]}
        if len(entries) > count:
            result['_omitted_entries'] = len(entries) - count
        return result
    return dict(total_entries=len(value), items=[structured_preview(item, per_entry, depth + 1)
                                               for _, item in entries[:count]])


def bounded_section(work, value, limit, projection=None):
    """Bound one semantic section without hiding unrelated checkpoint fields."""
    if len(encode(value)) <= limit:
        return value, None
    path, _ = archive_result(work, value)
    projected = projection(value) if projection else value
    if len(encode(projected)) <= limit:
        return projected, path
    if isinstance(projected, (dict, list)):
        entries = list(projected.items()) if isinstance(projected, dict) else list(enumerate(projected))
        kept = {} if isinstance(projected, dict) else []
        result = dict(truncated=True, total_entries=len(entries), entries=kept, full_output=path)
        for key, item in entries:
            if isinstance(kept, dict):
                kept[key] = item
            else:
                kept.append(item)
            if len(encode(result)) > limit:
                if isinstance(kept, dict):
                    kept.pop(key)
                else:
                    kept.pop()
                break
        result['omitted_entries'] = len(entries) - len(kept)
        return result, path
    return dict(truncated=True, full_output=path, preview=structured_preview(projected, limit // 2)), path


def bounded_result(work, value, limit=MAX_TOOL_BYTES):
    data = encode(value)
    if len(data) <= limit:
        return value
    path, data = archive_result(work, value)
    result = dict(truncated=True, total_bytes=len(data), full_output=path,
                  instruction='Preview only. Read full_output with read_text(root="work", path=full_output, offset/length) when more detail is needed.')
    if isinstance(value, dict):
        if isinstance(value.get('source_context'), dict) and len(encode(value['source_context'])) <= 1200:
            result['source_context'] = value['source_context']
        for key in ('error', 'exit_code', 'offset', 'next_offset', 'total_characters', 'total_entries', 'observation_ref'):
            if key in value and (value[key] is None or isinstance(value[key], (str, int, float, bool))):
                field = clip(value[key], min(512, limit // 16)) if isinstance(value[key], str) else value[key]
                if len(encode(field)) <= min(1024, limit // 8):
                    result[key] = field
    budget = max(0, limit - len(encode(result)) - 256)
    preview_budget = budget // 2
    result['preview'] = structured_preview(value, preview_budget)
    while len(encode(result)) > limit:
        preview_budget //= 2
        result['preview'] = structured_preview(value, preview_budget)
        if not preview_budget:
            result.pop('preview')
            break
    return result


def record_call(work, call_id, function, result):
    path, _ = archive_result(work, dict(tool_call_id=call_id, function=function, result=result))
    index = work / 'tool-results' / 'calls.jsonl'
    if index.is_symlink():
        raise ValueError('Tool index must stay inside the work directory')
    # Source code remains in the full record; the compact index is only a locator.
    entry = dict(tool_call_id=call_id, tool=function['name'], arguments_preview=clip(str(function.get('arguments', '')), 512), full_record=path)
    with index.open('a', encoding='utf-8') as handle:
        handle.write(json.dumps(entry, ensure_ascii=False) + '\n')
    return path


def call_records(audit):
    calls = {call['id']: call['function'] for message in audit for call in message.get('tool_calls', [])}
    records = {}
    for message in audit:
        if message.get('role') != 'tool' or message.get('tool_call_id') not in calls:
            continue
        try:
            value = json.loads(message['content'])
        except (TypeError, ValueError):
            continue
        call_id = message['tool_call_id']
        records[call_id] = dict(tool_call_id=call_id, function=calls[call_id], result=value)
    return records


def ensure_index(work, audit):
    if (work / 'tool-results' / 'calls.jsonl').exists():
        return
    calls = {call['id']: call['function'] for message in audit for call in message.get('tool_calls', [])}
    for message in audit:
        if message.get('role') == 'tool' and message.get('tool_call_id') in calls:
            try:
                value = json.loads(message['content'])
            except (TypeError, ValueError):
                continue
            record_call(work, message['tool_call_id'], calls[message['tool_call_id']], value)


def recent_observations(work, audit, exclude=()):
    calls = {call['id']: call['function'] for message in audit
             for call in message.get('tool_calls', [])}
    observations = []
    # Deduplication must not turn "recent" into older, superseded instructions.
    # Pick the actual last window first, then allow it to become smaller/empty.
    recent = [message for message in audit if message.get('role') == 'tool'][-RECENT_OBSERVATIONS:]
    for message in reversed(recent):
        function = calls.get(message.get('tool_call_id'))
        if not function or message.get('tool_call_id') in exclude:
            continue
        try:
            arguments = json.loads(function['arguments'])
            value = json.loads(message['content'])
        except (TypeError, ValueError, KeyError):
            continue
        if not isinstance(arguments, dict):
            arguments = dict(invalid_arguments=arguments)
        record_path, _ = archive_result(work, dict(tool_call_id=message['tool_call_id'], function=function, result=value))
        if isinstance(arguments.get('source'), str):
            arguments['source_sha256'] = hashlib.sha256(arguments.pop('source').encode()).hexdigest()
        path, _ = archive_result(work, value)
        # Reading an archived exchange must not turn its escaped JSON into a new
        # default research preview at every reset. The original remains linked.
        archive_read = (function['name'] in ('read_text', 'read_json') and arguments.get('root') == 'work'
                        and str(arguments.get('path', '')).replace('\\', '/').startswith('tool-results/'))
        preview = (dict(archived_read=True, instruction='Use current parser_files and metadata_helpers directly; read this archived exchange only for a specific missing detail.')
                   if archive_read else bounded_result(work, value, 1800))
        observations.append(dict(tool_call_id=message['tool_call_id'], full_record=record_path,
                                 tool=function['name'], arguments=bounded_result(work, arguments, 1200),
                                 result=preview, full_output=path))
    return list(reversed(observations))


def tool_input_identity(name, args):
    """Identify a tool's data target across a repaired source/proof attempt.

    Source code, encoding choices and explanatory notes may change when fixing
    the same input. Keep distinct tools, parser kinds, roots and read pages apart.
    This only resolves an old tool failure; it never grants host verification.
    """
    keys = ('kind', 'root', 'path', 'archive', 'script_name', 'key', 'command',
            'action', 'filter', 'target_script', 'pointer', 'offset', 'fields')
    target = {key: args[key] for key in keys if key in args}
    for key in ('path', 'archive'):
        if isinstance(target.get(key), str):
            target[key] = target[key].replace('\\', '/')
    return name, json.dumps(target, ensure_ascii=False, sort_keys=True, separators=(',', ':'))


def research_context(work, audit, exclude=()):
    """Keep a few older locators not already supplied by the durable ledger."""
    observations, errors, seen_errors, per_file = [], [], set(), {}
    later_successes, resolved_errors = set(), 0
    for record in reversed(list(call_records(audit).values())):
        function, value = record['function'], record['result']
        try:
            args = json.loads(function['arguments'])
        except (TypeError, ValueError, KeyError):
            continue
        if not isinstance(args, dict):
            continue
        identity = tool_input_identity(function['name'], args)
        error = value.get('error') if isinstance(value, dict) else None
        if error:
            if identity in later_successes:
                resolved_errors += 1
                continue
            key = (identity, str(error))
            if key not in seen_errors and len(errors) < 4:
                path, _ = archive_result(work, record)
                errors.append(dict(tool_call_id=record['tool_call_id'], tool=function['name'],
                                   root=args.get('root'), path=args.get('path', args.get('archive')),
                                   error=clip(str(error), 500), full_record=path,
                                   historical=True, resolved=False))
                seen_errors.add(key)
            continue
        successful = not (isinstance(value, dict) and
                          (value.get('exit_code', 0) not in (0, None)
                           or value.get('success') is False or value.get('ok') is False))
        if successful:
            later_successes.add(identity)
        if record['tool_call_id'] in exclude:
            continue
        if function['name'] not in ('inspect_data', 'load_script_metadata', 'verify_resource_selection', 'run_skill'):
            continue
        if isinstance(value, dict) and value.get('exit_code', 0) != 0:
            continue
        key = (function['name'], args.get('root'), args.get('path', args.get('archive', args.get('script_name'))))
        if per_file.get(key, 0) >= 1 or len(observations) >= 4:
            continue
        per_file[key] = per_file.get(key, 0) + 1
        path, _ = archive_result(work, record)
        observations.append(dict(tool_call_id=record['tool_call_id'], tool=function['name'],
                                 root=args.get('root'), path=key[-1], full_record=path,
                                 result=bounded_result(work, value, 800)))
    return dict(observations=list(reversed(observations)), known_errors=list(reversed(errors)),
                resolved_errors_omitted=resolved_errors,
                known_errors_instruction='Historical failed tool attempts, not current host blockers. Failures followed by a successful call on the same tool/input are omitted here; their complete records remain in the audit. Recheck current host state before acting on an old error.')


def speaker_resource_preview(value):
    """Keep the actual verified access path and one binding, never a name dump."""
    if not isinstance(value, dict):
        return value
    keys = ('key', 'role', 'scope', 'host_verified', 'records', 'entries', 'helper_file',
            'input_access', 'target', 'target_script', 'empty', 'companion_input')
    result = {key: value[key] for key in keys if key in value}
    sample = value.get('sample_entry')
    if isinstance(sample, dict):
        result['sample_entry'] = {key: sample[key] for key in ('entry', 'id', 'text') if key in sample}
    return result


def compact_speaker_progress(work, value, archives, prefix):
    """Give resource roles their own budgets while retaining sample/next_step."""
    if not isinstance(value, dict):
        return value
    groups = dict(global_resources=3200, scoped_resources=5200, empty_resources=2200)
    result = {key: item for key, item in value.items() if key not in groups}
    counts = {}
    for key, limit in groups.items():
        if key not in value:
            continue
        items = value[key]
        counts[key] = len(items) if isinstance(items, (dict, list)) else 0
        projected = ({name: speaker_resource_preview(row) for name, row in items.items()}
                     if isinstance(items, dict) else [speaker_resource_preview(row) for row in items]
                     if isinstance(items, list) else items)
        # Preserve the representative target even when it appears at the end of
        # a large batch. Other targets remain distinct and fully archived.
        if key in ('scoped_resources', 'empty_resources') and isinstance(projected, list) and value.get('sample'):
            sample = {name: value['sample'][name] for name in ('root', 'path') if name in value['sample']}
            projected.sort(key=lambda row: not (isinstance(row, dict) and row.get('target') == sample))
        result[key], path = bounded_section(work, projected, limit)
        if projected != items:
            path, _ = archive_result(work, items)
        if path:
            archives[prefix + '.' + key] = path
    result['resource_counts'] = counts
    result['empty_resource_meaning'] = 'An empty resource has no verified speaker/message bindings. It does not prove that displayed speakers are unavailable or that all messages are narration.'
    return result


def speaker_progress_reference(value):
    """One canonical resource view avoids three copies of each helper alias."""
    groups = ('global_resources', 'scoped_resources', 'empty_resources')
    result = {key: item for key, item in value.items() if key not in groups}
    result['state_ref'] = 'state.speaker_metadata_progress'
    result['resource_counts'] = {key: len(value[key]) for key in groups if isinstance(value.get(key), (dict, list))}
    return result


def compact_companions(values):
    result = {}
    for key, value in values.items():
        row = speaker_resource_preview(value)
        if isinstance(value, dict) and isinstance(value.get('preview'), dict) and value['preview']:
            entry, record = next(iter(value['preview'].items()))
            if isinstance(record, dict):
                row['preview'] = {entry: {name: record[name] for name in ('id', 'text') if name in record}}
        result[key] = row
    return result


def compact_helpers(values):
    """Group aliases of identical saved code without merging speaker scopes."""
    groups = {}
    for key, value in values.items():
        if not isinstance(value, dict):
            return values
        identity = (value.get('path'), value.get('sha256'))
        if identity not in groups:
            groups[identity] = dict(path=identity[0], sha256=identity[1], helper_keys=[])
        groups[identity]['helper_keys'].append(key)
    return dict(total_helpers=len(values), groups=list(groups.values()),
                instruction='These keys reuse the same saved helper code. Speaker records and target scopes remain separate in speaker_metadata_progress.')


def compact_sources(work, values, archives, prefix, helpers=False):
    """Keep current source strings exact, with separate budgets from inventories.

    A source that does not fit is archived whole. Do not replace it with escaped
    JSON or a partial function that could be mistaken for the current program.
    """
    if not isinstance(values, dict):
        return values
    entries = {}
    seen_sources = {}
    for key, original in values.items():
        if not isinstance(original, dict):
            continue
        row = dict(original)
        source = row.get('source')
        identity = hashlib.sha256(source.encode('utf-8')).hexdigest() if isinstance(source, str) else None
        if helpers and identity is not None and identity in seen_sources:
            primary = entries[seen_sources[identity]]
            primary['helper_keys'] = list(dict.fromkeys(primary.get('helper_keys', []) + row.get('helper_keys', [])))
            primary.setdefault('source_aliases', []).append(key)
            continue
        entries[key] = row
        if identity is not None:
            seen_sources[identity] = key
    # The host orders helper sources by global names and representative scoped
    # bindings. Preserve that order; a large save parser cannot crowd out script.
    ordered = list(entries.items()) if helpers else sorted(entries.items(), key=lambda item: (item[0] != 'script', item[0] != 'save'))
    source_limit = HELPER_SOURCE_BYTES if helpers else PROGRAM_SOURCE_BYTES
    total_limit = HELPER_SOURCES_BYTES if helpers else PROGRAM_SOURCES_BYTES
    used, result = 0, {}
    for key, row in ordered:
        description = {name: row[name] for name in ('path', 'sha256', 'role', 'scope', 'source_aliases') if name in row}
        source = row.get('source')
        if isinstance(source, str):
            size = len(encode(source))
            description['source_utf8_bytes'] = len(source.encode('utf-8'))
            if size <= source_limit and used + size <= total_limit:
                description['source'] = source
                used += size
            else:
                path, _ = archive_result(work, row)
                description.update(source_archived=True, full_source=path, source_pointer='/source',
                    instruction='The full source is archived; no partial function is shown. Read this exact source with read_json(root="work", path=full_source, pointer="/source") or read the current path directly.')
                archives[prefix + '.' + key + '.source'] = path
        if helpers and 'helper_keys' in row:
            description['helper_keys'], path = bounded_section(work, row['helper_keys'], 1600)
            if path:
                archives[prefix + '.' + key + '.helper_keys'] = path
        result[key] = description
    return result


def failure_preview(value):
    """Retain the current failure identity if a large diagnostic needs archiving."""
    if not isinstance(value, dict):
        return value
    priority = ('tool', 'kind', 'category', 'root', 'path', 'error', 'message', 'hint', 'tool_call_id',
                'source_sha256', 'program_sha256', 'parser_sha256', 'sample', 'target', 'current')
    result = {}
    for key in priority:
        if key not in value:
            continue
        item = value[key]
        if isinstance(item, str) and len(encode(item)) > 2000:
            result[key] = clip(item, 1200)
            result[key + '_truncated'] = True
        elif len(encode(item)) <= 2000:
            result[key] = item
    return result


def compact_progress(work, value, archives, prefix, canonical_speakers=None):
    """Keep counts/flags authoritative while bounding only large member lists."""
    if not isinstance(value, dict):
        return value
    result = {}
    for key, item in value.items():
        if key == 'speaker_metadata_progress' and isinstance(item, dict):
            if canonical_speakers is not None and item == canonical_speakers:
                result[key] = speaker_progress_reference(item)
            else:
                result[key] = compact_speaker_progress(work, item, archives, prefix + '.' + key)
        elif isinstance(item, dict):
            result[key] = compact_progress(work, item, archives, prefix + '.' + key, canonical_speakers)
        elif isinstance(item, list):
            result[key], path = bounded_section(work, item, 2400)
            if path:
                archives[prefix + '.' + key] = path
        else:
            result[key] = item
    return result


def progress_message(work, progress, instruction):
    """Bound a daily update without replaying the entire resource inventory.

    Current failures remain exact. Inventories are previews with exact archived
    originals; neither this projection nor an archive grants byte verification.
    The limit counts the final message, including JSON string escaping.
    """
    archives = {}
    failures = {key: progress[key] for key in ('last_script_failure', 'last_save_failure') if key in progress}
    value = {key: item for key, item in progress.items() if key not in failures}
    value = compact_progress(work, value, archives, 'progress')
    value.update(failures)
    guidance = value.get('guidance')
    if isinstance(guidance, dict):
        guidance = dict(guidance)
        action = guidance.get('next_action') or guidance.get('goal')
        if action is not None:
            guidance['next_action'] = action
            for key in ('goal', 'gate_instruction'):
                if guidance.get(key) == action:
                    guidance.pop(key, None)
        value['guidance'] = guidance
    if value != progress:
        full, _ = archive_result(work, progress)
        value['full_progress'] = full
    if archives:
        value['section_archives'] = archives

    def message():
        return dict(role='user', content='Host progress: ' + json.dumps(value, ensure_ascii=False) + '\n' + instruction)

    if len(encode(message())) > MAX_PROGRESS_MESSAGE_BYTES:
        full, _ = archive_result(work, progress)
        value['full_progress'] = full
        speakers = value.get('speaker_metadata_progress')
        if isinstance(speakers, dict):
            speakers = dict(speakers)
            for key in ('global_resources', 'scoped_resources', 'empty_resources'):
                if key in speakers:
                    original = progress.get('speaker_metadata_progress', {}).get(key, [])
                    speakers[key] = dict(archived=True, count=len(original), full_output=full,
                                         pointer='/speaker_metadata_progress/' + key)
            value['speaker_metadata_progress'] = speakers
        coverage = value.get('transcript_coverage')
        if isinstance(coverage, dict) and 'pending_members' in coverage:
            coverage = dict(coverage)
            original = progress.get('transcript_coverage', {}).get('pending_members', [])
            coverage['pending_members'] = dict(archived=True, count=len(original), full_output=full,
                                               pointer='/transcript_coverage/pending_members')
            value['transcript_coverage'] = coverage
    result = message()
    if len(encode(result)) > MAX_PROGRESS_MESSAGE_BYTES:
        raise RuntimeError('AI progress message exceeds its byte budget; full progress archived, current failures preserved')
    return result


def compact_tests(value):
    """Keep outcomes and limitations without replaying dialogue anchors."""
    if not isinstance(value, dict):
        return value
    result = {}
    # Keep failed/limited outcomes visible ahead of a long list of successes.
    entries = sorted(value.items(), key=lambda item: not (isinstance(item[1], dict) and
                     (item[1].get('error') or item[1].get('coverage') in ('scene_only', 'unresolved')
                      or item[1].get('limitations') or item[1].get('warnings'))))
    for path, test in entries:
        if not isinstance(test, dict):
            result[path] = test
            continue
        summary = {key: test[key] for key in ('coverage', 'sha256', 'error', 'limitations', 'warnings', 'rejected', 'valid') if key in test}
        matches = test.get('matching_anchors')
        if isinstance(matches, list):
            summary['matching_anchor_count'] = len(matches)
        elif matches is not None:
            summary['matching_anchors'] = matches
        facts = test.get('facts')
        if isinstance(facts, dict):
            summary['facts'] = {key: facts[key] for key in ('source', 'line', 'scene', 'resume', 'warnings', 'limitations') if key in facts}
        result[path] = summary
    return result


def checkpoint_state(work, facts):
    """Each field owns its budget: a resource inventory cannot erase progress."""
    state, archives = {}, {}
    budgets = dict(resource_selections=3000, samples=2400, speaker_resource_targets=5000,
                   companion_metadata=6000, metadata_helpers=6000, required_script_sets=2400,
                   extracted_from=2400, documents=5000, verified_tests=6000, speaker_contract=10000,
                   last_script_failure=8000, last_save_failure=8000, script_roles=4000)
    projections = dict(
        resource_selections=lambda values: {key: {name: row[name] for name in ('selected', 'priority', 'configuration') if name in row}
                                             for key, row in values.items()},
        metadata_helpers=compact_helpers,
        last_script_failure=failure_preview,
        last_save_failure=failure_preview,
        required_script_sets=lambda values: {key: dict(archive=row.get('archive'), member_count=len(row.get('members', [])))
                                             for key, row in values.items()},
        documents=lambda values: {key: {name: row[name] for name in ('complete', 'total_characters', 'next_unread_offset', 'sha256') if name in row}
                                  for key, row in values.items()},
        verified_tests=compact_tests)
    for key, value in facts.items():
        if key in ('progress', 'current_request'):
            state[key] = compact_progress(work, value, archives, key, facts.get('speaker_metadata_progress'))
        elif key == 'speaker_metadata_progress':
            state[key] = compact_speaker_progress(work, value, archives, key)
        elif key in ('program_sources', 'helper_sources'):
            state[key] = compact_sources(work, value, archives, key, helpers=key == 'helper_sources')
        elif key == 'companion_metadata' and isinstance(value, dict):
            projected = compact_companions(value)
            state[key], path = bounded_section(work, projected, budgets[key])
            if projected != value:
                path, _ = archive_result(work, value)
            if path:
                archives[key] = path
        elif key in budgets:
            state[key], path = bounded_section(work, value, budgets[key], projections.get(key))
            if path:
                archives[key] = path
            if key == 'verified_tests' and isinstance(value, dict):
                state['test_outcome_counts'] = dict(
                    total=len(value),
                    failed=sum(bool(test.get('error')) for test in value.values() if isinstance(test, dict)),
                    scene_only=sum(test.get('coverage') == 'scene_only' for test in value.values() if isinstance(test, dict)),
                    unresolved=sum(test.get('coverage') == 'unresolved' for test in value.values() if isinstance(test, dict)))
        else:
            # Schemas, current program paths/hashes, workflow, and the next host
            # action are small independent fields and must always remain direct.
            state[key] = value
    if archives:
        state['section_archives'] = archives
        state['section_archive_instruction'] = 'Only the named sections were shortened. Full JSON is available with read_json(root="work", path=<section archive>, pointer=...) or paginated read_text. Do not re-read whole history or treat omitted entries as completed.'
    return state


def ledger_context(work, ledger):
    """Avoid replaying both copies of every ledger observation and its notes."""
    if not isinstance(ledger, dict) or not ledger:
        return ledger
    result = {key: item for key, item in ledger.items() if key not in ('notes', 'observations', 'recent_observations')}
    result['notes_file'] = 'analysis-notes.json'
    observations = ledger.get('observations', [])[-4:]
    included = {row.get('tool_call_id') for row in observations}
    recent = [row for row in ledger.get('recent_observations', [])[-4:] if row.get('tool_call_id') not in included]
    for key, items in (('observations', observations), ('recent_observations', recent)):
        result[key] = []
        for row in items:
            value, _ = bounded_section(work, row, 2400)
            result[key].append(value)
    return result


def recent_analysis_sources(work, ledger, structural_preview=None):
    """Recover completed analysis code from existing tool records, never reasoning.

    The short ledger snapshot may no longer include the last useful inspection
    after many unrelated calls. Its fixed durable ledger preserves those locators
    without copying the conversation or executing any archived source.
    """
    if not isinstance(ledger, dict):
        return []
    rows = list(ledger.get('observations', [])) + list(ledger.get('recent_observations', []))
    ledger_path = work / 'analysis-ledger.json'
    if (ledger.get('ledger_file') == 'analysis-ledger.json' and ledger_path.is_file()
            and not ledger_path.is_symlink() and ledger_path.resolve().is_relative_to(work.resolve())):
        try:
            saved = json.loads(ledger_path.read_text('utf-8'))
            if isinstance(saved, dict) and isinstance(saved.get('observations'), list):
                rows = saved['observations']
        except (OSError, ValueError, TypeError):
            pass
    sources, seen, used = [], set(), 0
    for row in reversed(rows):
        if not isinstance(row, dict) or row.get('tool') != 'inspect_data' or row.get('successful') is not True:
            continue
        call_id = row.get('tool_call_id')
        if not isinstance(call_id, str) or call_id in seen:
            continue
        seen.add(call_id)
        locator = row.get('full_record')
        if not isinstance(locator, str):
            continue
        parts = locator.replace('\\', '/').split('/')
        if (len(parts) != 2 or parts[0] != 'tool-results' or not parts[1].endswith('.json')
                or len(parts[1]) != 69 or any(char not in '0123456789abcdef' for char in parts[1][:-5])):
            continue
        path = work / parts[0] / parts[1]
        if path.is_symlink() or not path.resolve().is_relative_to(work.resolve()):
            continue
        try:
            raw = path.read_bytes()
            if hashlib.sha256(raw).hexdigest() != parts[1][:-5]:
                continue
            record = json.loads(raw)
            function, result = record['function'], record['result']
            arguments = function['arguments']
            if isinstance(arguments, str):
                arguments = json.loads(arguments)
            if (record.get('tool_call_id') != call_id or function.get('name') != 'inspect_data'
                    or not isinstance(arguments, dict) or not isinstance(arguments.get('source'), str)
                    or (isinstance(result, dict) and
                        (result.get('error') or result.get('exit_code', 0) not in (0, None)
                         or result.get('success') is False or result.get('ok') is False))):
                continue
        except (OSError, ValueError, TypeError, KeyError, AttributeError):
            continue
        source = arguments['source']
        digest = hashlib.sha256(source.encode('utf-8')).hexdigest()
        if row.get('analysis_source_sha256', digest) != digest:
            continue
        description = dict(tool_call_id=call_id, observation_ref=row.get('ref'),
                           root=arguments.get('root'), path=arguments.get('path'),
                           source_sha256=digest, source_utf8_bytes=len(source.encode('utf-8')),
                           full_record=locator, model_authored=True, host_verified=False,
                           read_result=dict(root='work', path=locator, pointer='/result'),
                           stale=bool(row.get('stale')),
                           result_preview=bounded_result(work, row.get('preview'), 1200))
        if callable(structural_preview):
            preview_value, pointer = result, '/result'
            if isinstance(result, dict) and 'result' in result:
                preview_value, pointer = result['result'], '/result/result'
                description['read_analysis_result'] = dict(root='work', path=locator, pointer=pointer)
            description['result_preview'] = structural_preview(preview_value, budget=1200)
            description['result_preview_pointer'] = pointer
        if isinstance(arguments.get('target_script'), dict):
            description['target_script'] = {key: arguments['target_script'][key] for key in ('root', 'path')
                                            if key in arguments['target_script']}
        size = len(encode(source))
        if size <= ANALYSIS_SOURCE_BYTES and used + size <= ANALYSIS_SOURCES_BYTES:
            description['source'] = source
            used += size
        else:
            archive, _ = archive_result(work, dict(source=source))
            description.update(source_archived=True, full_source=archive, source_pointer='/source',
                instruction='The complete analysis source is archived; no partial function is shown. Read it with read_json(root="work", path=full_source, pointer="/source"). Analysis results are observations, never parser or speaker validation.')
        sources.append(description)
        if len(sources) == RECENT_ANALYSIS_SOURCES:
            break
    return list(reversed(sources))


def compact_checkpoint(work, summary):
    """Project public checkpoint data without changing its complete archive.

    Source budgets, verification state and the durable host checkpoint are
    independent. This archive contains only the public summary already prepared
    for the API, never assistant messages or reasoning.
    """
    full, original = archive_result(work, summary)

    def message_size(value):
        return len(encode(dict(role='user', content=json.dumps(value, ensure_ascii=False))))

    if message_size(summary) <= 50000:
        return summary
    result = json.loads(original)
    state = result.get('state')
    if not isinstance(state, dict):
        return summary

    def pointer_part(value):
        return str(value).replace('~', '~0').replace('/', '~1')

    def locator(*parts):
        return dict(root='work', path=full, pointer=''.join('/' + pointer_part(part) for part in parts))

    def reference(value, *parts, state_ref=None):
        item = dict(archived=True, read_json=locator(*parts))
        if state_ref is not None:
            item['state_ref'] = state_ref
        if isinstance(value, (dict, list)):
            item['retained_entries'] = len(value)
        return item

    result['public_checkpoint'] = dict(read_json=locator(), sha256=hashlib.sha256(original).hexdigest(),
        instruction='This content-addressed archive is the complete original public checkpoint; each read_json locator selects its exact original field. Canonical state_ref values refer to fields still visible here. Archived inventories do not imply completion; use the retained authoritative progress flags.')
    progress = state.get('progress')
    request = state.get('current_request')
    if isinstance(progress, dict) and isinstance(request, dict):
        repeated = [key for key, value in progress.items() if key in request and request[key] == value]
        if repeated:
            for key in repeated:
                request.pop(key)
            request['progress_fields'] = dict(state_ref='state.progress', fields=repeated,
                                            read_json=locator('state', 'current_request'))
    if isinstance(progress, dict):
        for key in ('last_script_failure', 'last_save_failure'):
            if isinstance(progress.get(key), dict) and progress[key] == state.get(key):
                progress[key] = reference(progress[key], 'state', 'progress', key, state_ref='state.' + key)
    if isinstance(request, dict) and isinstance(request.get('guidance'), dict):
        guidance = request['guidance']
        for key in ('goal', 'next_action', 'gate_instruction'):
            if isinstance(guidance.get(key), str) and guidance[key] == state.get('next_action'):
                guidance[key] = reference(guidance[key], 'state', 'current_request', 'guidance', key,
                                          state_ref='state.next_action')

    # These are inventories, not the source/evidence itself. Retain counts from
    # the actual projected value and the host's unchanged validation flags;
    # never synthesize a successful/complete marker from an archived list.
    inventories = ('resource_selections', 'speaker_resource_targets', 'companion_metadata',
                   'speaker_metadata_progress', 'metadata_helpers', 'required_script_sets', 'extracted_from')
    coverage = progress.get('transcript_coverage') if isinstance(progress, dict) else None
    workflow = state.get('workflow')
    completed_scripts = (isinstance(workflow, dict)
        and workflow.get('stage') in ('save_program', 'save_validation', 'finish')
        and isinstance(coverage, dict) and type(coverage.get('expected_members')) is int
        and coverage['expected_members'] > 0
        and coverage.get('covered_members') == coverage['expected_members']
        and coverage.get('pending_members') == []
        and progress.get('pending_script_inputs') == []
        and progress.get('pending_speaker_inputs') == []
        and progress.get('speaker_review_required') is False
        and isinstance(state.get('speaker_metadata_progress'), dict)
        and state['speaker_metadata_progress'].get('pending_targets') == [])
    for key in inventories:
        value = state.get(key)
        if not completed_scripts or not isinstance(value, (dict, list)) or len(encode(value)) <= 500:
            continue
        item = reference(value, 'state', key)
        item['validation_state_ref'] = 'state.progress'
        item['script_preparation_complete'] = True
        if isinstance(value, dict):
            for name in ('total_entries', 'total_helpers', 'total_resources', 'resource_counts',
                         'complete', 'sample', 'pending_targets', 'next_step'):
                if name in value and len(encode(value[name])) <= 350:
                    item[name] = value[name]
            if isinstance(value.get('entries'), (dict, list)):
                item['retained_entries'] = len(value['entries'])
        state[key] = item
    if completed_scripts and len(encode(state.get('speaker_contract'))) > 500:
        state['speaker_contract'] = reference(state['speaker_contract'], 'state', 'speaker_contract')

    if isinstance(workflow, dict) and isinstance(workflow.get('read_pages'), list):
        if len(encode(workflow['read_pages'])) > 500:
            pages = reference(workflow['read_pages'], 'state', 'workflow', 'read_pages')
            pages['document_state_ref'] = 'state.documents'
            workflow['read_pages'] = pages

    def observation(value, *parts):
        if not isinstance(value, dict):
            return value
        item = {key: value[key] for key in ('ref', 'observation_ref', 'tool_call_id', 'tool',
                'full_record', 'successful', 'stale', 'analysis_source_sha256') if key in value}
        item['read_json'] = locator(*parts)
        return item

    for section, keys in (('structural_ledger', ('observations', 'recent_observations')),
                          ('research', ('observations', 'known_errors'))):
        value = result.get(section)
        if not isinstance(value, dict):
            continue
        for key in keys:
            if isinstance(value.get(key), list):
                value[key] = [observation(row, section, key, index) for index, row in enumerate(value[key])]
    if isinstance(result.get('recent_observations'), list):
        result['recent_observations'] = [observation(row, 'recent_observations', index)
                                         for index, row in enumerate(result['recent_observations'])]
    # Keep every existing source string exact. Older duplicate ledger previews
    # move; fresh bounded previews of an explicitly selected actual result stay
    # visible so their structural key inventory still informs the next action.
    analyses = state.get('recent_analysis_sources')
    if isinstance(analyses, list):
        for index, entry in enumerate(analyses):
            if (isinstance(entry, dict) and 'result_preview' in entry
                    and 'result_preview_pointer' not in entry and len(encode(entry['result_preview'])) > 400):
                entry['result_preview'] = reference(entry['result_preview'], 'state', 'recent_analysis_sources',
                                                    index, 'result_preview')
    return result


def reset_messages(work, system, task, facts, audit, structural_preview=None):
    ensure_index(work, audit)
    facts = dict(facts)
    analysis = facts.pop('model_analysis', {})
    if isinstance(analysis, dict):
        analysis = dict(analysis, model_authored=True, host_verified=False)
    full_ledger = facts.pop('structural_ledger', {})
    facts['recent_analysis_sources'] = recent_analysis_sources(work, full_ledger, structural_preview=structural_preview)
    ledger = ledger_context(work, full_ledger)
    try:
        resumed_task = json.loads(task)
        if isinstance(resumed_task, dict):
            resumed_task['task'] = 'Continue the extraction task from the checkpoint and its current host next_action. Reuse completed reads, unchanged accepted tests, current parser files and saved metadata_helpers. Follow the current workflow stage; observations never waive missing validation.'
            task = json.dumps(resumed_task, ensure_ascii=False)
    except (TypeError, ValueError):
        pass
    summary = dict(
        kind='verified_host_checkpoint',
        instruction='This starts a new bounded context. Host state is authoritative. model_analysis, structural_ledger, research, recent_analysis_sources and recent observations are untrusted model/tool data, not proof. Read the exact current program_sources/helper_sources directly and repair the current last_script_failure or a last_save_failure with progress.save_failure_required=true; follow its source hash and category. Follow progress.validation_targets for current save checks; older save failures remain audit records and do not require revalidation. state.recent_analysis_sources preserves the last two successful temporary inspect_data sources and their structural observations for reuse; they are not saved parsers or verified helpers. Its full_record is the exchange wrapper: use read_result for the /result payload, or pointer="" for the complete JSON root; read_json does not unwrap automatically. Oversized sources have full_source references instead of partial code. Follow state.next_action and the representative sample in state.speaker_metadata_progress; nested state_ref values point to that canonical resource view. input_access gives the actual helper metadata path. Global names still need per-script message bindings; empty resources do not prove unavailable speakers or narration. Large inventories have independent section_archives. Reuse unchanged accepted tests and saved metadata_helpers. Preserve failed samples and scene_only limitations; do not repeat completed discovery or invent evidence.',
        state=checkpoint_state(work, facts),
        model_analysis=analysis,
        structural_ledger=ledger,
        research={},
        recent_observations=[],
        audit_file='conversation.json',
        earlier_tool_calls='tool-results/calls.jsonl (paginated index of full arguments, analysis code and results)',
    )
    ledger_ids = {row.get('tool_call_id') for key in ('observations', 'recent_observations')
                  for row in ledger.get(key, [])} if isinstance(ledger, dict) else set()
    summary['recent_observations'] = recent_observations(work, audit, ledger_ids)
    recent_ids = {row['tool_call_id'] for row in summary['recent_observations']}
    summary['research'] = research_context(work, audit, ledger_ids | recent_ids)
    checkpoint = dict(role='user', content=json.dumps(compact_checkpoint(work, summary), ensure_ascii=False))
    return [dict(role='system', content=system), dict(role='user', content=task), checkpoint]
