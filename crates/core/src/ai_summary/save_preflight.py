"""Byte-grounded save preflight receipts, independent of any engine strategy.

The host supplies its existing root resolver and independent span/scalar replay
functions. Model classifications are workflow evidence, never verified nodes.
No engine names, format heuristics, model calls or subprocesses belong here.

Caller interface:
  store = SavePreflight(resolve_path=agent.path, replay_span=checks.span,
                        integer=checks.integer, view=checks.view,
                        required_saves=lambda: current_relative_paths)
  store.record(tool_arguments)
  store.check(tool, arguments, intent_for_tool=host_intent_classifier)

The classifier maps tools to inspect/resource_helper/script_sample/script_batch/
finish/other. report_unavailable remains outside this gate so a missing save can
be reported immediately by its existing independently checked API.
"""
from copy import deepcopy
import hashlib
import json
from pathlib import Path, PureWindowsPath
import re


MAX_BYTES = 32 * 1024 * 1024
HASH_BLOCK_BYTES = 1024 * 1024
MAX_OBSERVATIONS = 32
MAX_PLAN_TARGETS = 3
MAX_RECORD_BYTES = 48 * 1024
MAX_RECORDS = 64
GUARDED_INTENTS = frozenset(('script_sample', 'script_batch', 'finish'))
RECHECK_INTENTS = frozenset(('script_batch', 'finish'))


class PreflightRequired(ValueError):
    """Machine-readable feedback; presentation remains the caller's policy."""

    def __init__(self, code, message, pending):
        super().__init__(message)
        self.code = code
        self.pending = deepcopy(pending)


def _text(value, field, maximum, single_line=False):
    if (not isinstance(value, str) or not value.strip()
            or len(value.encode('utf-8')) > maximum
            or single_line and any(c in value for c in '\r\n')):
        raise ValueError(field + ' must be bounded nonempty text')
    return value.strip()


def _relative(value):
    # The host resolver is authoritative for root escape, symlink and cold-cache
    # rules. This check rejects malformed arguments before calling it.
    value = _text(value, 'path', 2048, single_line=True)
    if (Path(value).is_absolute() or PureWindowsPath(value).is_absolute()
            or '..' in value.replace('\\', '/').split('/')):
        raise ValueError('Preflight needs a relative file path under its named root')
    return value


class SavePreflight:
    def __init__(self, *, resolve_path, replay_span, required_saves=(),
                 replay_scalar=None, integer=None, view=None, structured_reader=None):
        if not callable(resolve_path) or not callable(replay_span):
            raise TypeError('Preflight needs trusted root and span callbacks')
        if not callable(replay_scalar) and not callable(integer):
            raise TypeError('Preflight needs a trusted scalar replay callback')
        if replay_scalar is not None and not callable(replay_scalar):
            raise TypeError('Preflight scalar replay must be a trusted callback')
        if view is not None and not callable(view):
            raise TypeError('Preflight view replay must be a trusted callback')
        if structured_reader is not None and not callable(structured_reader):
            raise TypeError('Preflight structured replay must be a trusted callback')
        self.resolve_path = resolve_path
        self.replay_span = replay_span
        self.replay_scalar = replay_scalar
        self.integer = integer
        self.view = view
        self.structured_reader = structured_reader
        self.required_source = required_saves
        self.records = {}
        self.dependency_cache = {}

    def _required(self):
        paths = self.required_source() if callable(self.required_source) else self.required_source
        if not isinstance(paths, (list, tuple, set, frozenset)) or len(paths) > MAX_RECORDS:
            raise ValueError('Required saves must be a bounded collection of relative paths')
        return tuple(dict.fromkeys(_relative(path) for path in paths))

    def set_required_saves(self, paths):
        """Call after authoritative latest-progress discovery, not model ranking."""
        self.required_source = tuple(_relative(path) for path in paths)

    def _scalar(self, data, proof):
        if self.replay_scalar is not None:
            return self.replay_scalar(data, proof)
        transformed = data
        if 'view' in proof:
            if self.view is None:
                raise ValueError('Scalar byte-view evidence needs independent view replay')
            transformed = self.view(data, proof['view'])
        return self.integer(transformed, proof['offset'], proof.get('width', 4),
                            proof.get('endian', 'little'))

    def _input(self, root, relative):
        path = Path(self.resolve_path(root, _relative(relative))).resolve()
        if not path.is_file():
            raise ValueError('Preflight targets must be existing exact files')
        before = path.stat()
        if not 0 < before.st_size <= MAX_BYTES:
            raise ValueError('Preflight input must be a nonempty bounded file')
        data = path.read_bytes()
        after = path.stat()
        if ((before.st_size, before.st_mtime_ns) != (after.st_size, after.st_mtime_ns)
                or len(data) != after.st_size):
            raise ValueError('Preflight input changed while it was being read')
        stamp = dict(path=str(path), sha256=hashlib.sha256(data).hexdigest(),
                     size=len(data), modified_ns=after.st_mtime_ns)
        return data, stamp

    def _context_dependency(self, root, relative, *, force_hash=False):
        """Fingerprint a helper/archive without reading it wholly into memory.

        At record/restore the SHA is replayed from bytes. During scheduling an
        unchanged path/size/mtime reuses that in-memory SHA, avoiding repeated
        multi-gigabyte archive reads. This is a workflow invalidation hint, not a
        resource-verification receipt: a same-size replacement with deliberately
        preserved mtime remains the final resource/native verifier's concern.
        """
        path = Path(self.resolve_path(root, _relative(relative))).resolve()
        if not path.is_file():
            raise ValueError('Context targets must be existing exact files')
        before = path.stat()
        if before.st_size <= 0:
            raise ValueError('Context targets must be nonempty files')
        signature = (str(path), before.st_size, before.st_mtime_ns)
        cached = self.dependency_cache.get(str(path))
        if not force_hash and cached is not None and cached['signature'] == signature:
            return deepcopy(cached['input'])
        digest = hashlib.sha256()
        length = 0
        with path.open('rb') as handle:
            while True:
                block = handle.read(HASH_BLOCK_BYTES)
                if not block:
                    break
                length += len(block)
                digest.update(block)
        after = path.stat()
        if ((before.st_size, before.st_mtime_ns) != (after.st_size, after.st_mtime_ns)
                or length != after.st_size):
            raise ValueError('Context target changed while it was being fingerprinted')
        stamp = dict(path=str(path), sha256=digest.hexdigest(), size=length,
                     modified_ns=after.st_mtime_ns)
        self.dependency_cache[str(path)] = dict(signature=signature, input=stamp)
        return deepcopy(stamp)

    def _observation(self, data, item):
        if not isinstance(item, dict) or set(item) != {'role', 'proof', 'value'}:
            raise ValueError('An observation needs role, proof and value')
        role = item['role']
        if not isinstance(role, str) or not re.fullmatch(r'[a-z][a-z0-9_]{0,47}', role):
            raise ValueError('Observation role must be a short structural identifier')
        proof, value = item['proof'], item['value']
        if not isinstance(proof, dict):
            raise ValueError('Observation proof must be an independent byte proof')
        if proof.get('kind') in ('structured_field', 'typed_field'):
            if self.structured_reader is None:
                raise ValueError('Structured observations need trusted complete-document field replay')
            expected = proof.get('type')
            if expected == 'string':
                if not isinstance(value, str) or not value or len(value.encode('utf-8')) > 4096:
                    raise ValueError('Text observations need bounded nonempty native text')
            elif expected in ('text_integer', 'integer'):
                if type(value) is not int or not 0 <= value <= 2**64 - 1:
                    raise ValueError('Structured integer observations need an exact uint64 value')
            else:
                raise ValueError('Structured observations need type="string", type="text_integer" or type="integer"')
            replayed = self.structured_reader(data, deepcopy(proof))
            if not isinstance(replayed, dict) or replayed.get('type') != expected or 'value' not in replayed:
                raise ValueError('Structured observation reader returned an inconsistent typed field')
            actual = replayed['value']
        elif type(proof.get('offset')) is not int or proof['offset'] < 0:
            raise ValueError('Observation proof needs a native nonnegative offset')
        elif 'encoding' in proof:
            if set(proof) - {'offset', 'length', 'encoding', 'view'}:
                raise ValueError('Unknown text-proof field')
            if not isinstance(value, str) or not value or len(value.encode('utf-8')) > 4096:
                raise ValueError('Text observations need bounded nonempty native text')
            if type(proof.get('length')) is not int or proof['length'] <= 0:
                raise ValueError('Text observation needs a nonempty native span')
            actual = self.replay_span(data, deepcopy(proof))
        else:
            if set(proof) - {'offset', 'width', 'endian', 'view'}:
                raise ValueError('Unknown scalar-proof field')
            if type(value) is not int:
                raise ValueError('Scalar observations need an exact native integer')
            if type(proof.get('width', 4)) is not int or proof.get('width', 4) not in (1, 2, 3, 4, 8):
                raise ValueError('Scalar observation has an unsupported integer width')
            actual = self._scalar(data, deepcopy(proof))
        if type(actual) is not type(value) or actual != value:
            raise ValueError('Observation disagrees with independently replayed bytes')
        return dict(role=role, proof=deepcopy(proof), value=value)

    def _context_plan(self, plan, primary):
        if not isinstance(plan, dict) or set(plan) != {'missing_fact', 'targets'}:
            raise ValueError('needs_context requires one missing_fact and exact file targets')
        # One explicit field keeps the request focused. Its semantics remain an
        # auditable model claim; this module does not judge engine explanations.
        missing = _text(plan['missing_fact'], 'missing_fact', 400, single_line=True)
        targets = plan['targets']
        if not isinstance(targets, list) or not 1 <= len(targets) <= MAX_PLAN_TARGETS:
            raise ValueError('context_plan requires a bounded set of real target files')
        checked, seen = [], set()
        for target in targets:
            if not isinstance(target, dict) or set(target) != {'root', 'path'}:
                raise ValueError('Context targets need an exact named root and file path')
            if target['root'] not in ('game', 'save', 'work', 'skill'):
                raise ValueError('Unknown context target root')
            stamp = self._context_dependency(target['root'], target['path'], force_hash=True)
            if stamp['path'] in seen:
                raise ValueError('Context targets must not duplicate the same file')
            seen.add(stamp['path'])
            checked.append(dict(root=target['root'], path=target['path'], input=stamp))
        if primary['path'] not in seen:
            raise ValueError('Context targets must include the current save being explained')
        return dict(missing_fact=missing, targets=checked,
                    recheck_before=['script_batch', 'finish'])

    def record(self, arguments):
        """Record grounded native observations and their current byte identity.

        This receipt permits workflow progression only. It never populates
        validated_saves or substitutes for the final current-node verifier.
        """
        allowed = {'path', 'classification', 'observations', 'explanation', 'context_plan'}
        if not isinstance(arguments, dict) or set(arguments) - allowed:
            raise ValueError('Unknown save-preflight argument')
        try:
            encoded = json.dumps(arguments, ensure_ascii=False).encode('utf-8')
        except (TypeError, ValueError) as error:
            raise ValueError('Preflight arguments must be ordinary JSON data') from error
        if len(encoded) > MAX_RECORD_BYTES:
            raise ValueError('Preflight record exceeds its byte limit')
        classification = arguments.get('classification')
        if classification not in ('progress_candidate', 'needs_context'):
            raise ValueError('Unsupported save-preflight classification')
        explanation = _text(arguments.get('explanation'), 'explanation', 1200)
        relative = _relative(arguments.get('path'))
        data, stamp = self._input('save', relative)
        observations = arguments.get('observations')
        if not isinstance(observations, list) or not 1 <= len(observations) <= MAX_OBSERVATIONS:
            raise ValueError('Preflight needs bounded independently verified observations')
        verified = [self._observation(data, item) for item in observations]
        if classification == 'progress_candidate':
            if all(item['role'] == 'container_header' for item in verified):
                raise ValueError('A container header alone does not establish a progress candidate')
            if 'context_plan' in arguments:
                raise ValueError('A progress candidate must not retain a missing-context plan')
            plan = None
        else:
            plan = self._context_plan(arguments.get('context_plan'), stamp)
        receipt = dict(version=1, path=relative, input=stamp,
                       classification=classification, observations=verified,
                       explanation=explanation, context_plan=plan,
                       verified_current_node=False)
        if stamp['path'] not in self.records and len(self.records) >= MAX_RECORDS:
            raise ValueError('Too many retained save-preflight records')
        # Commit only after every observation and dependency has been checked.
        self.records[stamp['path']] = receipt
        return deepcopy(receipt)

    def _current(self, relative):
        _, stamp = self._input('save', relative)
        record = self.records.get(stamp['path'])
        if not record:
            return None, dict(path=relative, reason='not_inspected')
        old = record['input']
        if (old['path'], old['size'], old['sha256']) != (stamp['path'], stamp['size'], stamp['sha256']):
            return None, dict(path=relative, reason='save_bytes_changed')
        plan = record['context_plan']
        if plan:
            for target in plan['targets']:
                current = self._context_dependency(target['root'], target['path'])
                previous = target['input']
                if (previous['path'], previous['size'], previous['sha256']) != (
                        current['path'], current['size'], current['sha256']):
                    return None, dict(path=relative, reason='context_target_changed',
                                      target=dict(root=target['root'], path=target['path']))
        return record, None

    def check_intent(self, intent):
        """Gate work by semantics rather than recognizing any engine or extension."""
        if intent not in GUARDED_INTENTS:
            return dict(allowed=True, intent=intent)
        required = self._required()
        if not required:
            raise PreflightRequired('save_targets_unknown',
                                    'Discover the current required save before script work.', [])
        pending = []
        for relative in required:
            try:
                record, stale = self._current(relative)
            except (OSError, ValueError):
                record, stale = None, dict(path=relative, reason='save_or_dependency_missing')
            if stale:
                pending.append(stale)
            elif intent in RECHECK_INTENTS and record['classification'] == 'needs_context':
                pending.append(dict(path=relative, reason='context_recheck_required',
                                    missing_fact=record['context_plan']['missing_fact']))
        if pending:
            raise PreflightRequired('save_preflight_required',
                                    'Recheck the current save before advancing script preparation.', pending)
        return dict(allowed=True, intent=intent, checked_targets=len(required),
                    verified_current_node=False)

    def check(self, tool, arguments, *, intent_for_tool):
        """The host classifies tool intent; the draft never changes live dispatch."""
        return self.check_intent(intent_for_tool(tool, arguments))

    def status(self):
        """Supply facts for host scheduling, including early save-read exemptions.

        Resource exploration policy belongs to the caller. It should keep
        targeted save inspection available while these records are pending or
        need context, even if its ordinary resource exploration gate is closed.
        """
        required = self._required()
        pending, records = [], []
        for relative in required:
            try:
                record, stale = self._current(relative)
            except (OSError, ValueError):
                record, stale = None, dict(path=relative, reason='save_or_dependency_missing')
            if stale:
                pending.append(stale)
            else:
                records.append(dict(path=relative, classification=record['classification'],
                                    input=deepcopy(record['input']),
                                    context_plan=deepcopy(record['context_plan'])))
        needs_context = [record['path'] for record in records
                         if record['classification'] == 'needs_context']
        return dict(version=1, required=list(required), targets_unknown=not required,
                    pending=pending, records=records, needs_context=needs_context,
                    ready_for_script_sample=bool(required) and not pending,
                    ready_for_script_batch=bool(required) and not pending and not needs_context,
                    verified_current_node=False)

    def serialise(self):
        """Persist data only; restored receipts are independently rechecked."""
        return dict(version=1, records=deepcopy(list(self.records.values())))

    @staticmethod
    def _same_input(first, second):
        return (isinstance(first, dict) and isinstance(second, dict)
                and all(first.get(key) == second.get(key) for key in ('path', 'sha256', 'size')))

    def restore(self, state):
        """Absent/old records never satisfy the gate; stale records are dropped.

        A replayed record must still cite the same input and dependency hashes.
        The module reruns every original observation, including byte transforms.
        """
        self.records.clear()
        self.dependency_cache.clear()
        if (not isinstance(state, dict) or state.get('version') != 1
                or not isinstance(state.get('records'), list)
                or len(state['records']) > MAX_RECORDS):
            return dict(restored=0, discarded=0, compatible=False)
        restored = discarded = 0
        for saved in state['records']:
            previous_records = dict(self.records)
            try:
                if not isinstance(saved, dict) or saved.get('version') != 1:
                    raise ValueError('Unsupported saved preflight receipt')
                if saved.get('verified_current_node') is not False:
                    raise ValueError('Preflight cannot restore a node-verification claim')
                arguments = {key: deepcopy(saved[key])
                             for key in ('path', 'classification', 'observations', 'explanation')}
                old_plan = saved.get('context_plan')
                if old_plan is not None:
                    arguments['context_plan'] = dict(missing_fact=old_plan['missing_fact'],
                        targets=[{key: target[key] for key in ('root', 'path')}
                                 for target in old_plan['targets']])
                replayed = self.record(arguments)
                if not self._same_input(saved.get('input'), replayed['input']):
                    raise ValueError('Saved preflight input changed')
                if old_plan:
                    new_targets = replayed['context_plan']['targets']
                    if (len(old_plan['targets']) != len(new_targets)
                            or any(not self._same_input(old['input'], new['input'])
                                   for old, new in zip(old_plan['targets'], new_targets))):
                        raise ValueError('Saved context dependency changed')
                restored += 1
            except (OSError, ValueError, TypeError, KeyError):
                self.records = previous_records
                discarded += 1
        return dict(restored=restored, discarded=discarded, compatible=True)
