"""Durable, bounded model research; this module never validates a game parser.

Only tool records are archived here. Assistant messages and private reasoning are
never copied into the ledger. The ledger links back to complete tool records and
keeps a small structural preview rather than duplicating game dialogue.
"""
import difflib
import hashlib
import json
import math
import pathlib
import re

VERSION = 1
MAX_PREVIEW_BYTES = 1200
MAX_NOTES_BYTES = 12000
STRUCTURAL_TOOLS = {'inspect_data', 'inspect_engine_adapter', 'inspect_structured', 'inspect_archive_member', 'read_bytes', 'load_script_metadata',
                    'verify_resource_selection', 'run_skill', 'test_program',
                    'test_program_batch'}
NON_EVIDENCE_TOOLS = {'record_findings', 'record_script_role'}
COORDINATES = {'archive', 'member', 'section_header', 'section_payload',
               'string_ordinal', 'instruction', 'save', 'root', 'path', 'file',
               'coordinate_base', 'position_base', 'base', 'offset', 'length',
               'unit', 'encoding', 'endian', 'table_offset', 'count_offset',
               'count', 'pool_offset', 'pool_length', 'width', 'index', 'message_id'}
# Export the actual accepted field contract so the host tool schema cannot drift
# from validation again. These coordinates remain model claims, not byte proofs.
COORDINATES_SCHEMA = {
    'type': 'object',
    'description': 'Explicit model-claimed coordinates, at most 2000 UTF-8 JSON bytes. '
                   'Values are strings, finite numbers, null, or small nested objects '
                   'with those values; no arrays or booleans. Preserve the actual '
                   'base and unit. These notes never replace byte evidence.',
    'properties': {key: {'type': ['string', 'number', 'object', 'null']}
                   for key in sorted(COORDINATES)},
    'additionalProperties': False}
STRUCTURAL_STRINGS = {'encoding', 'endian', 'kind', 'format', 'type', 'unit',
                      'mode', 'root', 'path', 'archive', 'member', 'section',
                      'signature', 'magic', 'status', 'sample_type', 'base',
                      'table', 'field', 'section_name', 'coordinate_system',
                      'length_unit', 'hex'}


def encode(value):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode('utf-8')


def clipped(value, limit):
    return value.encode('utf-8')[:max(0, limit)].decode('utf-8', 'ignore')


def successful(value):
    """A successful tool observation still does not mean independently verified."""
    return not (isinstance(value, dict) and
                (value.get('error') or value.get('exit_code', 0) not in (0, None)
                 or value.get('success') is False or value.get('ok') is False))


def arguments(function):
    value = function.get('arguments', {})
    if isinstance(value, str):
        try:
            value = json.loads(value)
        except (TypeError, ValueError):
            return {}
    return value if isinstance(value, dict) else {}


def validate_coordinates(coordinates):
    """Bound explicit coordinates without interpreting or inventing their bases."""
    if not isinstance(coordinates, dict):
        raise ValueError('coordinates must be an object; omit it when no coordinates are known')
    unknown = sorted(set(coordinates) - COORDINATES)
    if unknown:
        raise ValueError('Unsupported coordinate fields: ' + ', '.join(str(key) for key in unknown)
                         + '. Allowed fields: ' + ', '.join(sorted(COORDINATES)))
    if len(encode(coordinates)) > 2000:
        raise ValueError('coordinates must contain at most 2000 UTF-8 JSON bytes')

    def check(value, depth=0):
        if value is None or isinstance(value, str):
            return
        if isinstance(value, bool):
            raise ValueError('Coordinate values must be strings, finite numbers, null or explicit base/offset/unit objects; no booleans or arrays')
        if isinstance(value, (int, float)):
            if isinstance(value, float) and not math.isfinite(value):
                raise ValueError('Coordinate numbers must be finite')
            return
        if isinstance(value, dict):
            if depth >= 3 or len(value) > 32 or any(not isinstance(key, str) or len(key.encode('utf-8')) > 64 for key in value):
                raise ValueError('Coordinate objects must be small, with string keys and at most three nested levels')
            for child in value.values():
                check(child, depth + 1)
            return
        raise ValueError('Coordinate values must be strings, finite numbers, null or explicit base/offset/unit objects; no booleans or arrays')

    for value in coordinates.values():
        check(value)


def structural_preview(value, budget=MAX_PREVIEW_BYTES):
    """Return bounded JSON structure, never a serialized-JSON text prefix.

    This pure function does not open full_output paths, inspect parser sources,
    or unwrap a result packet. Callers select the actual value and advertise its
    matching archive pointer. Inventory rows are [actual key, JSON type, optional
    container count/string UTF-8 bytes]; truncated keys have explicit descriptors.
    """
    if not isinstance(budget, int) or isinstance(budget, bool) or budget < 64:
        raise ValueError('Structural preview budget must be an integer of at least 64 UTF-8 bytes')

    def kind(item):
        if item is None:
            return 'null'
        if isinstance(item, bool):
            return 'boolean'
        if isinstance(item, (int, float)):
            return 'number'
        if isinstance(item, str):
            return 'string'
        if isinstance(item, dict):
            return 'object'
        if isinstance(item, list):
            return 'array'
        return type(item).__name__

    def scalar(item, key=''):
        if item is None or isinstance(item, bool):
            return item
        if isinstance(item, int):
            return item if item.bit_length() <= 128 else dict(type='number', integer_bits=item.bit_length())
        if isinstance(item, float):
            return item if math.isfinite(item) else dict(type='number', finite=False)
        if isinstance(item, str):
            size = len(item.encode('utf-8'))
            return item if key in STRUCTURAL_STRINGS and size <= 160 else dict(string_bytes=size)
        return dict(type=kind(item))

    def summary(item):
        result = dict(type=kind(item))
        if isinstance(item, (dict, list)):
            result.update(count=len(item), ellipsis=bool(item))
        elif isinstance(item, str):
            result.update(utf8_bytes=len(item.encode('utf-8')), ellipsis=bool(item))
        return result

    def walk(item, key='', depth=0):
        if not isinstance(item, (dict, list)):
            return scalar(item, key)
        if depth >= 3:
            return summary(item)
        if isinstance(item, dict):
            if len(item) > 32 or any(not isinstance(k, str) or len(k.encode('utf-8')) > 64 for k in item):
                return summary(item)
            return {k: walk(v, k, depth + 1) for k, v in item.items()}
        return dict(count=len(item), items=[walk(v, key, depth + 1) for v in item[:3]],
                    omitted_items=max(0, len(item) - 3))

    force_inventory = isinstance(value, dict) and (len(value) > 32
        or any(not isinstance(key, str) or len(key.encode('utf-8')) > 64 for key in value))
    result = walk(value) if not force_inventory else None
    if result is not None and len(encode(result)) <= budget:
        return result
    result = summary(value)
    if isinstance(value, dict):
        # Reserve inventory for every short top-level key before optional scalar
        # detail. An early large child cannot consume the later fields' budget.
        result.update(key_inventory=[], inventory_columns=['key', 'type', 'count_or_utf8_bytes'], omitted_keys=len(value))
        for key, item in value.items():
            if isinstance(key, str):
                raw_key = key.encode('utf-8')
                label = key if len(raw_key) <= 64 else dict(key_prefix=clipped(key, 24),
                    key_utf8_bytes=len(raw_key), key_sha256=hashlib.sha256(raw_key).hexdigest())
            else:
                label = dict(key_type=type(key).__name__)
            row = [label, kind(item)]
            if isinstance(item, (dict, list)):
                row.append(len(item))
            elif isinstance(item, str):
                row.append(len(item.encode('utf-8')))
            result['key_inventory'].append(row)
            result['omitted_keys'] -= 1
            if len(encode(result)) > budget:
                result['key_inventory'].pop()
                result['omitted_keys'] += 1
                break
        # Scalar observations are optional and never replace inventory entries.
        if result['omitted_keys'] == 0:
            for key, item in value.items():
                if isinstance(item, (dict, list)) or not isinstance(key, str) or len(key.encode('utf-8')) > 64:
                    continue
                if isinstance(item, str) and (key not in STRUCTURAL_STRINGS or len(item.encode('utf-8')) > 160):
                    continue
                result.setdefault('scalars', {})[key] = scalar(item, key)
                if len(encode(result)) > budget:
                    del result['scalars'][key]
                    if not result['scalars']:
                        del result['scalars']
        if not result['key_inventory']:
            del result['key_inventory']
            del result['inventory_columns']
    elif isinstance(value, list):
        result.update(items=[], omitted_items=len(value))
        for item in value[:3]:
            result['items'].append(walk(item, depth=1))
            result['omitted_items'] -= 1
            if len(encode(result)) > budget:
                result['items'].pop()
                result['omitted_items'] += 1
                break
        if not result['items']:
            del result['items']
    if len(encode(result)) > budget:
        # The smallest supported budget still gives a complete structural fact.
        result = dict(type=kind(value), ellipsis=True)
    return result


class ResearchStore:
    def __init__(self, work, audit_provider=None):
        self.work = pathlib.Path(work).resolve()
        self.work.mkdir(parents=True, exist_ok=True)
        self.audit_provider = audit_provider or (lambda: [])
        self.records = {}
        self.call_refs = {}
        self.notes = {'findings': {}, 'next_steps': []}
        self.needs_recheck = False
        self._next_id = 1
        self.restore()

    def _path(self, name):
        path = self.work / name
        if not path.resolve().is_relative_to(self.work) or path.is_symlink():
            raise ValueError('Research records must stay inside the work directory')
        return path

    def _write(self, name, value):
        path = self._path(name)
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary = self._path(name + '.tmp')
        temporary.write_bytes(encode(value))
        temporary.replace(path)

    def _archive(self, record):
        data = encode(record)
        name = 'tool-results/' + hashlib.sha256(data).hexdigest() + '.json'
        path = self._path(name)
        path.parent.mkdir(parents=True, exist_ok=True)
        if not path.is_file() or path.read_bytes() != data:
            path.write_bytes(data)
        return name

    def _persist(self):
        self._write('analysis-ledger.json', {
            'version': VERSION, 'model_observations': True, 'host_verified': False,
            'needs_recheck': self.needs_recheck, 'next_id': self._next_id,
            'observations': list(self.records.values()), 'notes': self.notes})
        self._write('analysis-notes.json', dict(model_authored=True,
                    needs_recheck=self.needs_recheck, **self.notes))

    def restore(self, notes=None, stale=False):
        """Load the ledger, bootstrap old audit IDs, and optionally import old notes.

        `stale=True` is the host's signal that the checkpoint/input stamps changed.
        It never revalidates observations or parser artifacts.
        """
        path = self._path('analysis-ledger.json')
        legacy_notes = None
        if not path.is_file():
            old_path = self._path('analysis-notes.json')
            if old_path.is_file() and old_path.stat().st_size <= MAX_NOTES_BYTES + 4000:
                try:
                    legacy_notes = json.loads(old_path.read_text('utf-8'))
                    # Preserve the original even when some cited audit records are
                    # unavailable. Missing evidence never becomes an observation.
                    if isinstance(legacy_notes, dict):
                        legacy_path = self._path('analysis-notes-legacy.json')
                        if not legacy_path.exists():
                            self._write('analysis-notes-legacy.json', legacy_notes)
                except (OSError, ValueError, TypeError):
                    legacy_notes = None
        if not self.records and path.is_file():
            try:
                saved = json.loads(path.read_text('utf-8'))
                if saved.get('version') == VERSION:
                    for entry in saved.get('observations', []):
                        if (isinstance(entry, dict) and re.fullmatch(r'O[1-9][0-9]*', str(entry.get('ref', '')))
                                and isinstance(entry.get('tool_call_id'), str)):
                            self.records[entry['ref']] = entry
                            self.call_refs[entry['tool_call_id']] = entry['ref']
                    self._next_id = max([int(ref[1:]) + 1 for ref in self.records] + [1])
                    self.notes = saved.get('notes', self.notes)
                    self.needs_recheck = bool(saved.get('needs_recheck'))
            except (OSError, ValueError, TypeError, KeyError):
                # The audit remains the recovery source for a damaged ledger.
                self.records, self.call_refs = {}, {}
                self.notes = {'findings': {}, 'next_steps': []}
        if legacy_notes is not None and not (self.audit_provider() or []):
            # Hosts load their audit after constructing the store. For the one
            # upgrade from old notes, recover references from that saved audit
            # before the first persistence can replace the old notes file.
            conversation = self._path('conversation.json')
            if conversation.is_file():
                try:
                    old_audit = json.loads(conversation.read_text('utf-8'))
                    if isinstance(old_audit, list):
                        self.bootstrap(old_audit)
                except (OSError, ValueError, TypeError):
                    pass
        self.bootstrap()
        if legacy_notes is not None:
            self._import_notes(legacy_notes)
            if legacy_notes.get('needs_recheck'):
                self.mark_stale(persist=False)
        if notes is not None:
            self._import_notes(notes)
        if stale:
            self.mark_stale(persist=False)
        self._persist()
        return self.snapshot(bootstrap=False)

    def _import_notes(self, notes):
        if not isinstance(notes, dict):
            return
        findings = notes.get('findings', {})
        items = []
        if isinstance(findings, dict):
            for identity, finding in findings.items():
                # The ledger is persisted per tool call, before a host checkpoint.
                # A crash can therefore leave that checkpoint one tool behind.
                # Never replace the newer durable ledger with older checkpoint notes.
                if not isinstance(finding, dict) or identity in self.notes['findings']:
                    continue
                refs = finding.get('evidence_refs') or [
                    item.get('ref', item.get('tool_call_id'))
                    for item in finding.get('evidence', []) if isinstance(item, dict)]
                items.append(dict(id=identity, status=finding.get('status'),
                                  summary=finding.get('summary'), evidence_refs=refs,
                                  **({'coordinates': finding['coordinates']} if 'coordinates' in finding else {})))
        # Migration uses the same bounds and reference validation as new notes.
        self.record_findings({'findings': items[:20],
                              'next_steps': (self.notes['next_steps'] if self.notes['findings'] or self.notes['next_steps']
                                             else notes.get('next_steps', []))}, bootstrap=False)

    def bootstrap(self, audit=None):
        """Assign stable short references to old successful and failed tool calls."""
        audit = (self.audit_provider() or []) if audit is None else audit
        calls = {call['id']: call.get('function', {}) for message in audit if isinstance(message, dict)
                 for call in message.get('tool_calls', []) if isinstance(call, dict) and 'id' in call}
        changed = False
        for message in audit:
            if not isinstance(message, dict) or message.get('role') != 'tool':
                continue
            call_id = message.get('tool_call_id')
            if call_id not in calls or call_id in self.call_refs:
                continue
            try:
                value = json.loads(message.get('content', ''))
            except (TypeError, ValueError):
                continue
            self.observe(call_id, calls[call_id], value, persist=False)
            changed = True
        if changed:
            self._persist()

    def _inputs(self, inputs, function):
        if inputs is None:
            args = arguments(function)
            inputs = {key: args[key] for key in ('root', 'path', 'archive') if key in args}
        values = inputs if isinstance(inputs, list) else [inputs]
        result = []
        for value in values[:8]:
            if not isinstance(value, dict):
                continue
            item = {}
            for key in ('root', 'path', 'archive', 'member', 'size', 'mtime_ns', 'sha256', 'coordinate_base'):
                entry = value.get(key)
                if isinstance(entry, str):
                    item[key] = clipped(entry, 512)
                elif isinstance(entry, int) and not isinstance(entry, bool):
                    item[key] = entry
            if item:
                result.append(item)
        return result

    def observe(self, call_id, function, result, full_record=None, inputs=None, persist=True):
        """Record an actual completed tool call and return its small citation handle.

        `inputs` must come from the host's actual opened files, never model claims.
        A host can supply its existing archived `full_record` to avoid duplication.
        """
        if not isinstance(call_id, str) or not call_id or not isinstance(function, dict):
            raise ValueError('A real tool call ID and function are required')
        if call_id in self.call_refs:
            record = self.records[self.call_refs[call_id]]
            # A historical audit may have created a locator before the host stamps
            # were available. Enrich that same reference rather than assigning anew.
            if inputs is not None and not record.get('inputs_stamped'):
                record['inputs'] = self._inputs(inputs, function)
                record['inputs_stamped'] = True
                if persist:
                    self._persist()
            return self._handle(record)
        if full_record is None:
            full_record = self._archive(dict(tool_call_id=call_id, function=function, result=result))
        else:
            full_record = str(pathlib.Path(full_record).as_posix())
            self._path(full_record)
        described = self._inputs(inputs, function)
        if inputs is not None:
            self.set_inputs(described, persist=False)
        ref = 'O' + str(self._next_id)
        self._next_id += 1
        args = arguments(function)
        preview_value, preview_pointer = result, '/result'
        if function.get('name') == 'inspect_data' and isinstance(result, dict) and 'result' in result:
            preview_value, preview_pointer = result['result'], '/result/result'
        elif function.get('name') == 'inspect_engine_adapter' and isinstance(result, dict) and 'inspection' in result:
            preview_value, preview_pointer = result['inspection'], '/result/inspection'
        entry = dict(ref=ref, tool_call_id=call_id, tool=function.get('name', ''),
                     full_record=full_record, inputs=described, inputs_stamped=inputs is not None,
                     successful=successful(result), stale=False,
                     preview=structural_preview(preview_value), preview_target_pointer=preview_pointer,
                     read_preview=dict(root='work', path=full_record, pointer=preview_pointer))
        if isinstance(args.get('source'), str):
            entry['analysis_source_sha256'] = hashlib.sha256(args['source'].encode('utf-8')).hexdigest()
        self.records[ref] = entry
        self.call_refs[call_id] = ref
        if persist:
            self._persist()
        return self._handle(entry)

    @staticmethod
    def _handle(entry):
        return {key: entry[key] for key in ('ref', 'tool_call_id', 'full_record', 'successful', 'stale')}

    def set_inputs(self, inputs, persist=True):
        """Mark observations and notes stale when host-supplied file stamps differ."""
        values = inputs if isinstance(inputs, list) else [inputs]
        current = {(item.get('root'), item.get('path', item.get('archive'))): item
                   for item in values if isinstance(item, dict)}
        changed = False
        for record in self.records.values():
            if record.get('stale'):
                continue
            for old in record.get('inputs', []):
                now = current.get((old.get('root'), old.get('path', old.get('archive'))))
                fields = ('size', 'mtime_ns', 'sha256')
                # Missing hashes are unknown, not evidence of changed bytes.
                if now and any(key in old and key in now and old[key] != now[key] for key in fields):
                    record['stale'] = True
                    changed = True
        if changed:
            self.needs_recheck = True
            self._stale_findings()
            if persist:
                self._persist()
        return changed

    def _stale_findings(self):
        for finding in self.notes['findings'].values():
            if any(self.records.get(ref, {}).get('stale') for ref in finding.get('evidence_refs', [])):
                finding['stale'] = True

    def mark_stale(self, persist=True):
        self.needs_recheck = True
        for record in self.records.values():
            record['stale'] = True
        for finding in self.notes['findings'].values():
            finding['stale'] = True
        if persist:
            self._persist()

    def resolve(self, reference, require_success=False):
        if reference == 'latest':
            candidates = reversed(list(self.records.values()))
            return next((record for record in candidates
                         if record['tool'] not in NON_EVIDENCE_TOOLS and not record.get('stale')
                         and record.get('successful')), None)
        ref = self.call_refs.get(reference, reference) if isinstance(reference, str) else None
        record = self.records.get(ref)
        if record and record['tool'] not in NON_EVIDENCE_TOOLS and (not require_success or record.get('successful')):
            return record
        return None

    def _reference_error(self, reference):
        choices = list(self.records) + list(self.call_refs)
        closest = difflib.get_close_matches(str(reference), choices, n=2, cutoff=0.5)
        refs = [self.call_refs.get(value, value) for value in closest]
        available = [row['ref'] for row in self.records.values()
                     if row.get('successful') and not row.get('stale') and row['tool'] not in NON_EVIDENCE_TOOLS][-6:]
        return dict(reference=reference, closest_refs=list(dict.fromkeys(refs)), available_refs=available)

    def _findings_result(self, accepted, errors, next_steps_saved=False):
        """Report what survived validation, not merely that a notes file exists."""
        status = 'saved' if not errors else ('partial' if accepted else 'rejected')
        result = dict(ok=not errors, status=status,
                      saved='analysis-notes.json' if accepted or next_steps_saved else None,
                      notes_file='analysis-notes.json', accepted=accepted, errors=errors,
                      findings_saved=len(accepted), next_steps_saved=next_steps_saved,
                      findings=len(self.notes['findings']), next_steps=self.notes['next_steps'],
                      available_refs=self._reference_error('')['available_refs'],
                      model_authored=True, needs_recheck=self.needs_recheck,
                      instruction='Notes and observations are not verification. Implement/test the next parser and retain all required byte evidence and save tests.')
        if errors:
            result['error'] = ('No submitted findings were saved.' if not accepted else
                               'Only ' + str(len(accepted)) + ' submitted finding(s) were saved.')
            result['instruction'] = ('Only entries listed in accepted were saved. Repair errors and resubmit rejected '
                                     'findings before relying on them across resets; next_steps_saved reports that field '
                                     'separately. Reuse the available evidence refs; repairing a notes field does not '
                                     'require repeating byte discovery. Notes never replace parser/save verification.')
        return result

    def record_findings(self, args, bootstrap=True):
        """Accept valid findings independently; a typo never discards other notes."""
        if bootstrap:
            self.bootstrap()
        if not isinstance(args, dict):
            return self._findings_result([], [dict(error='Supply a findings object')])
        errors, accepted, next_steps_saved = [], [], False
        items = args.get('findings', [])
        if not isinstance(items, list):
            errors.append(dict(field='findings', error='Supply an array of at most 20 concise findings'))
            items = []
        if len(items) > 20:
            errors.append(dict(field='findings', error='Only the first 20 findings were considered'))
        for index, item in enumerate(items[:20]):
            identity = item.get('id') if isinstance(item, dict) else None
            detail = dict(index=index, id=identity)
            try:
                if not isinstance(item, dict):
                    raise ValueError('Each finding must be an object')
                status, summary = item.get('status'), item.get('summary')
                if not isinstance(identity, str) or not re.fullmatch(r'[A-Za-z0-9_-]{1,64}', identity):
                    raise ValueError('Use a stable finding ID with 1 to 64 letters, digits, underscores or hyphens')
                if status not in ('observed', 'hypothesis', 'rejected'):
                    raise ValueError('Use observed, hypothesis or rejected status')
                if not isinstance(summary, str) or not summary.strip() or len(summary.encode('utf-8')) > 2400:
                    raise ValueError('Use a nonempty structural summary of at most 2400 UTF-8 bytes; no dialogue dump or private reasoning')
                detail['field'] = 'evidence_refs'
                references = item.get('evidence_refs', item.get('evidence_call_ids'))
                if not isinstance(references, list) or not 1 <= len(references) <= 6:
                    raise ValueError('Cite 1 to 6 short evidence_refs, old tool call IDs, or latest')
                evidence, invalid = [], []
                for reference in references:
                    record = self.resolve(reference)
                    if record is None or (status == 'observed' and not record.get('successful')):
                        invalid.append(self._reference_error(reference))
                    else:
                        evidence.append(self._handle(record))
                if invalid:
                    detail['references'] = invalid
                    raise ValueError('References must identify actual evidence calls; observed notes require successful calls')
                coordinates = item.get('coordinates')
                if coordinates is not None:
                    detail['field'] = 'coordinates'
                    validate_coordinates(coordinates)
                detail.pop('field', None)
                finding = dict(status=status, summary=summary, evidence=evidence,
                               evidence_refs=list(dict.fromkeys(row['ref'] for row in evidence)),
                               stale=any(row['stale'] for row in evidence))
                if coordinates is not None:
                    finding['coordinates'] = json.loads(encode(coordinates))
                proposed = dict(self.notes['findings'])
                proposed[identity] = finding
                if len(encode(dict(findings=proposed, next_steps=self.notes['next_steps']))) > MAX_NOTES_BYTES:
                    raise ValueError('Notes exceed 12000 bytes; shorten or consolidate existing notes')
                self.notes['findings'] = proposed
                accepted.append(dict(id=identity, evidence_refs=finding['evidence_refs'], stale=finding['stale']))
            except (ValueError, TypeError, OverflowError) as error:
                errors.append(dict(**detail, error=str(error)))
        if 'next_steps' in args:
            steps = args['next_steps']
            if (not isinstance(steps, list) or len(steps) > 8
                    or any(not isinstance(step, str) or len(step.encode('utf-8')) > 1000 for step in steps)):
                errors.append(dict(field='next_steps', error='Supply at most 8 next steps, each at most 1000 UTF-8 bytes'))
            elif len(encode(dict(findings=self.notes['findings'], next_steps=steps))) > MAX_NOTES_BYTES:
                errors.append(dict(field='next_steps', error='Notes exceed 12000 bytes; previous next steps retained'))
            else:
                self.notes['next_steps'] = steps
                next_steps_saved = True
        self._persist()
        return self._findings_result(accepted, errors, next_steps_saved)

    def snapshot(self, limit=12, bootstrap=True):
        if bootstrap:
            self.bootstrap()
        limit = max(0, min(int(limit), 20))
        observations = [entry for entry in self.records.values()
                        if entry.get('successful') and entry['tool'] in STRUCTURAL_TOOLS]
        # Each ledger row remains bounded; long histories are linked on disk.
        recent = list(self.records.values())[-limit:] if limit else []
        return dict(model_authored=True, host_verified=False, needs_recheck=self.needs_recheck,
                    notes=self.notes, observations=observations[-limit:] if limit else [],
                    recent_observations=recent, total_observations=len(self.records),
                    ledger_file='analysis-ledger.json',
                    legacy_notes_file='analysis-notes-legacy.json' if self._path('analysis-notes-legacy.json').is_file() else None,
                    instruction='These are model observations, never proof of parser correctness. Reuse full_record code and exact coordinate bases; stale entries require new evidence.')
