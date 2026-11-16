"""先验证少量样本，再冻结可复用的批量参数；模型不能绕过试提取直接启动批量任务。"""
import fnmatch
import json
from pathlib import Path


def audio_probe(path):
    with path.open('rb') as stream:
        header = stream.read(64)
    if len(header) < 16:
        raise ValueError('Audio sample is truncated')
    if header.startswith(b'OggS'):
        return 'ogg'
    if header.startswith(b'RIFF') and header[8:12] == b'WAVE':
        return 'wav'
    if header.startswith(b'fLaC'):
        return 'flac'
    if header.startswith(b'ID3') or header[0] == 0xff and header[1] & 0xe0 == 0xe0:
        return 'mp3/aac'
    if header[4:8] == b'ftyp':
        return 'mp4-audio'
    if header.startswith(b'MThd'):
        return 'midi'
    raise ValueError('Audio sample has no recognized playable header; decode it with the installed format tools before exporting')


def listing(agent, args):
    path = agent.files_api['source'](agent, 'game', args['path'])
    mode, pattern = args.get('mode'), args.get('pattern')
    if mode not in ('archive', 'files') or not isinstance(pattern, str) or not pattern or len(pattern)>512:
        raise ValueError('Provide mode=archive/files and a bounded glob selecting the requested resource class')
    if mode == 'archive':
        if not path.is_file():
            raise ValueError('Select an archive file')
        agent.stamp(path)
        result = agent.host._dispatch('run_skill', dict(command='l', archive=args['path']))
        if result.get('error') or result.get('exit_code', 0):
            raise ValueError(result.get('error') or 'Archive listing failed')
        inventory = json.loads(agent.host.path('work', result['member_inventory']).read_text('utf-8'))
        names = inventory['members']
        members = [name for name in names if fnmatch.fnmatchcase(name.replace('\\','/'), pattern)]
        if len(set(members)) != len(members):
            raise ValueError('Duplicate native names need an indexed extraction backend; this recipe cannot safely batch them')
    else:
        directory, paths = agent.files_api['select'](agent, dict(root='game',path=args['path'],pattern=pattern))
        members = [p.relative_to(directory).as_posix() for p in paths]
        if path.is_file():
            raise ValueError('files recipes need a directory and a glob')
    if not members:
        raise ValueError('No resources match this extraction recipe')
    if len(members)>200000:
        raise ValueError('Selection exceeds the batch inventory limit')
    return members


def extracted_sample(agent, archive, name):
    literal = agent.api['ARCHIVE_TOOLS']['literal_glob_path'](name)
    result = agent.host._dispatch('run_skill', dict(command='x', archive=archive, filter=literal, filter_syntax='glob'))
    if result.get('error') or result.get('exit_code',0) or not result.get('sample_output'):
        raise ValueError(result.get('error') or 'Sample archive extraction failed')
    # 每个请求只选择一个原生名称；控制脚本扩展在媒体任务中保持关闭。
    if result.get('member_count') != 1:
        raise ValueError('Sample extraction did not select exactly one resource')
    return agent.host.path('work', result['sample_output'])


def test_extraction(agent, args):
    agent.progress('sampling')
    members = listing(agent,args)
    selected = list(dict.fromkeys([members[0], members[len(members)//2], members[-1]]))
    samples = []
    for name in selected:
        path = extracted_sample(agent,args['path'],name) if args['mode']=='archive' else agent.host.path('game',str(Path(args['path'])/name))
        root = 'work' if path.is_relative_to(agent.work) else 'game'
        relative = path.relative_to(agent.host.roots[root]).as_posix()
        if agent.kind == 'cg':
            before = len(agent.files)
            result = agent.files_api['convert_images'](agent, dict(root=root,path=relative,mode='files'))
            pngs = [entry for entry in agent.files[before:] if entry['path'].endswith('.png') and entry.get('source',{}).get('kind')=='png']
            if result['failed'] or not pngs:
                raise ValueError('CG sample did not decode successfully; resolve its format before batching')
            validation = dict(format='png',frames=len(pngs))
        else:
            validation = dict(format=audio_probe(path),validation='container header; original encoded audio retained')
            agent.files_api['export'](agent,dict(root=root,path=relative))
        samples.append(dict(name=name,sha256=agent.files_api['digest'](path),**validation))
    key = 'sample-'+str(len(agent.tested)+1)
    recipe = dict(mode=args['mode'],path=args['path'],pattern=args['pattern'],members=members,samples=samples)
    agent.tested[key] = recipe
    agent.emit(key+'.json',recipe)
    return dict(test=key,selected=len(members),tested=len(samples),samples=samples,
        next_action='Test other distinct resource formats/archives as needed, then start_batch with these successful test IDs. Do not manually extract the rest; the standalone batch script will continue without AI.')


def start_batch(agent, args):
    tests = args.get('tests')
    if not isinstance(tests,list) or not tests or len(tests)>128 or len(set(tests))!=len(tests) or any(test not in agent.tested for test in tests):
        raise ValueError('Select successful test_extraction IDs; untested batch recipes are forbidden')
    scope, warnings = args.get('scope'), args.get('warnings')
    if not isinstance(scope,str) or not scope.strip() or not isinstance(warnings,list) or len(warnings)>64 or any(not isinstance(w,str) or len(w)>2000 for w in warnings):
        raise ValueError('Describe the actual scope and explicit unsupported resources')
    agent.check_inputs()
    plan = dict(schema=1,kind=agent.kind,scope=scope[:8000],warnings=warnings,recipes=[agent.tested[test] for test in tests],
        input_stamps=agent.input_stamps,documents=agent.host.documents)
    agent.emit('media-plan.json',plan)
    agent.done = dict(schema=1,kind=agent.kind,batch_plan='media-plan.json',tested_samples=sum(len(recipe['samples']) for recipe in plan['recipes']))
    return agent.done
