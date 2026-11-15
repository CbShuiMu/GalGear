"""复用 Skill 图像转换器和本机 OCR；保留帧来源，绝不把转换序号冒充引擎 ID。"""
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import time


def normalized_text(text):
    # Windows OCR 在日中字符之间插入分词空格；原始行和词框仍完整保留供复核。
    return re.sub(r'(?<=[\u3000-\u9fff\uff00-\uffef]) +(?=[\u3000-\u9fff\uff00-\uffef])', '', text).strip()


def native_frame_ids(data):
    """HG-3 的转换器来源指向帧内容；将偏移映射回原生 ID，不从输出序号猜测。"""
    if len(data)<20 or data[:4]!=b'HG-3': return {}
    result={}; offset=struct.unpack_from('<I',data,4)[0]
    if offset<12: return {}
    while offset+8<=len(data):
        following,frame_id=struct.unpack_from('<II',data,offset)
        result[offset+8]=frame_id
        if following==0: return result
        if following<8 or offset+following+8>len(data): return {}
        offset+=following
    return {}


def prepare_route_images(agent,graph):
    """把草稿已引用且本轮已提取的唯一图像补为证据；不扫描无关美术或猜版本。"""
    text=json.dumps(graph,ensure_ascii=False).casefold()
    candidates={}
    extensions={'.png','.jpg','.jpeg','.bmp','.webp','.hg2','.hg3','.g00','.eri','.tlg','.pimg'}
    for event in getattr(agent,'observations',{}).values():
        if event.get('tool')!='run_format_program' or event.get('error'):continue
        result=event.get('result',{})
        if not result.get('manifest') or not result.get('output'):continue
        manifest=json.loads(agent.host.path('work',result['manifest']).read_text('utf-8'))
        for member in manifest.get('members',[]):
            name=Path(member['path'])
            if name.suffix.casefold() not in extensions:continue
            if not re.search(r'(?<![\w.-])'+re.escape(name.stem.casefold())+r'(?![\w-])',text):continue
            path=agent.host.path('work',str(Path(result['output'])/name))
            candidates.setdefault(name.name.casefold(),{})[str(path)]=member['sha256']
    existing=set()
    for receipt in getattr(agent,'image_receipts',set()):
        record=json.loads(agent.host.path('work',receipt).read_text('utf-8'))
        if record.get('source'):existing.add(str(Path(record['source']).resolve()))
    attempted=getattr(agent,'route_image_attempts',set())
    agent.route_image_attempts=attempted
    evidence=[]
    for versions in candidates.values():
        if len(versions)!=1:continue  # 基础包/补丁同名图不擅自选择。
        path,digest=next(iter(versions.items()))
        if path in existing or (path,digest) in attempted:continue
        attempted.add((path,digest))
        relative=str(Path(path).relative_to(agent.work))
        try:
            if hashlib.sha256(Path(path).read_bytes()).hexdigest()!=digest:
                raise ValueError('Extracted image changed since its receipt')
            result=read_images(agent,dict(root='work',path=relative))
            evidence.append(dict(path=relative,**result))
        except (ValueError,OSError,subprocess.SubprocessError) as error:
            evidence.append(dict(path=relative,error=str(error)[:1500]))
    return evidence


def read_images(agent, args):
    root = args.get('root')
    if root not in ('game', 'work'):
        raise ValueError('UI images must come from game/work, not save/skill files')
    path = agent.host.path(root, args['path'])
    if not path.is_file() or not 0 < path.stat().st_size <= 32 * 1024 * 1024:
        raise ValueError('Select one nonempty native image container <=32 MiB; extract only referenced members first')
    agent.stamp(path)
    raw=path.read_bytes()
    digest = hashlib.sha256(raw).hexdigest()
    cache = getattr(agent, 'image_cache', {})
    if digest not in cache:
        executable = agent.host.roots['skill'] / 'tools/GARbro-cli/GalImageExport.exe'
        if not executable.is_file():
            raise ValueError('Installed Skill has no GalImageExport.exe; follow its image extraction guide')
        directory = agent.work / ('ui-images-' + digest[:20] + '-' + str(len(agent.observations)+1))
        job = directory.with_suffix('.job.json')
        agent.api['atomic_json'](job, dict(mode='decode', source=str(path.parent), files=[str(path)], output=str(directory), seconds=35))
        completed = subprocess.run([str(executable), str(job)], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            timeout=min(45, max(1, agent.deadline-time.monotonic())), creationflags=0x08000000 if os.name == 'nt' else 0)
        manifest = directory / 'manifest.jsonl'
        if completed.returncode or not manifest.is_file():
            raise ValueError('Skill image conversion failed: ' + completed.stderr.decode('utf-8','replace')[-1500:])
        entries = [json.loads(line) for line in manifest.read_text('utf-8-sig').splitlines() if line.strip()]
        errors = [e for e in entries if e.get('kind','').endswith(('_error','_deferred'))]
        frames = []
        native_ids=native_frame_ids(raw)
        for entry in entries:
            if entry.get('kind') != 'png': continue
            png = Path(entry['path']).resolve()
            if not png.is_relative_to(directory.resolve()) or not png.is_file() or png.is_symlink():
                raise ValueError('Converter output escaped its dedicated directory')
            if hashlib.sha256(png.read_bytes()).hexdigest() != entry['sha256']:
                raise ValueError('Converter PNG hash mismatch')
            frames.append(dict(ordinal=len(frames), path=str(png.relative_to(agent.work)), sha256=entry['sha256'],
                width=entry['width'], height=entry['height'], origin=entry.get('origin',{})))
            native_id=native_ids.get(entry.get('origin',{}).get('offset'))
            if native_id is not None: frames[-1]['native_frame_id']=native_id
        if not frames: raise ValueError('No decoded frames: ' + json.dumps(errors,ensure_ascii=False)[:1200])
        cache[digest] = dict(source=str(path), source_sha256=digest, frames=frames, errors=errors)
        agent.image_cache = cache
    record = json.loads(json.dumps(cache[digest]))
    selected = args.get('frames')
    if selected is None:
        selected = [f['ordinal'] for f in record['frames'] if f['width']*f['height'] <= 400000][:32]
    if not isinstance(selected,list) or len(selected)>32 or any(type(i) is not int or i<0 or i>=len(record['frames']) for i in selected):
        raise ValueError('frames selects up to 32 converter ordinals from this catalog, NOT native frame IDs')
    language = args.get('language','ja')
    if not re.fullmatch(r'[a-zA-Z0-9-]{2,32}',language): raise ValueError('Invalid OCR language tag')
    if selected:
        ps = Path(os.environ.get('SystemRoot',r'C:\Windows')) / 'System32/WindowsPowerShell/v1.0/powershell.exe'
        script = Path(__file__).with_name('image-ocr.ps1')
        payload = dict(language=language,images=[dict(id=i,path=str(agent.work/record['frames'][i]['path'])) for i in selected])
        completed = subprocess.run([str(ps),'-NoProfile','-NonInteractive','-File',str(script)], input=json.dumps(payload).encode(),
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=min(45,max(1,agent.deadline-time.monotonic())),
            creationflags=0x08000000 if os.name=='nt' else 0)
        if completed.returncode: raise ValueError('Local OCR failed: '+completed.stderr.decode('utf-8','replace')[-1800:])
        result = json.loads(completed.stdout.decode('utf-8-sig'))
        for image in result['images']:
            record['frames'][image['id']].update(ocr=image, text=normalized_text(image['text']), language=language)
    # 一次路线提交可能补读多个图像，不能只按观察编号命名而覆盖上一张图。
    # 后续指定帧读取还需保留此前的 OCR，避免已经绑定的选项证据消失。
    cache[digest]=record
    output = 'ui-text-' + str(len(agent.observations)+1) + '-' + digest[:12] + '.json'
    agent.emit(output, record)
    if not hasattr(agent,'image_receipts'): agent.image_receipts=set()
    agent.image_receipts.add(output)
    agent.check_inputs()
    catalog=[{k:f[k] for k in ('ordinal','native_frame_id','width','height','origin','text') if k in f} for f in record['frames']]
    return dict(output=output, source_sha256=digest, frames=catalog, errors=record['errors'],
        limitation='OCR candidate, not runtime proof. ordinal/member suffix is converter order, NOT native frame ID. Join origin.offset to the native header and UI event. Empty text may be an icon; never invent option wording.')


def tool_schemas(api):
    return [api['tool']('read_ui_images', 'Decode one referenced UI image using the installed Skill converter, then local Windows OCR. First extract ONLY members referenced by UI scripts using run_format_program. Returns all frame origins/ordinals and OCR for up to 32 small frames by default. Reuse with frames=[converter ordinals] for selected others, or frames=[] for a catalog without OCR. native_frame_id is the actual engine ID when present; ordinal is converter order. Outputs registered source evidence for graph characters/text_basis.',
        dict(root=dict(type='string',enum=['game','work']),path=dict(type='string'), language=dict(type='string'), frames=dict(type='array',items=dict(type='integer'),maxItems=32)), ['root','path'])]
